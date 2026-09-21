//! OAuth for a remote MCP server, run by Orochi rather than by the agent it routes to. Orochi
//! holds the token and hands every session the authorized server, so signing in once covers
//! whichever CLI runs the work — and covers the CLIs that cannot sign in at all.
//!
//! The flow is the one the MCP specification asks for: the endpoint's `401` names its protected
//! resource metadata (RFC 9728), that names the authorization server (RFC 8414), a public client
//! is registered on the spot where the server allows it (RFC 7591), and the code is exchanged
//! with PKCE and an exact loopback redirect, naming the resource it is for (RFC 8707).
use crate::config::{McpServerConfig, McpTransport};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the browser has. Long enough for a login with a second factor, not indefinite.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// Refreshed this long before it expires, so a run does not start with a token about to die.
const REFRESH_MARGIN: i64 = 120;

/// What Orochi holds for one server. It is the user's credential, so it lives beside memory in
/// its own `0600` file and never in telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    /// The URL these were issued for; a server pointed somewhere else signs in again.
    pub url: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds. `None` is a token the server did not time-limit.
    #[serde(default)]
    pub expires_at: Option<i64>,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    pub token_endpoint: String,
    pub resource: String,
}
impl Credentials {
    fn expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|at| at - REFRESH_MARGIN <= now)
    }
}

/// Where tokens are kept: the macOS keychain, as Claude Code keeps its own, or a `0600` file
/// beside Orochi's data — elsewhere, when `mcp.keychain` is off, and in tests, which must never
/// touch the keychain of the machine they run on.
pub struct Vault {
    data: PathBuf,
    keychain: bool,
}

const SERVICE: &str = "orochi-mcp";
/// `security -i` reads a line into a 4 KiB buffer and runs whatever is past it as another
/// command (measured on 2026-09-22: 4089 bytes went through, 4099 were cut). The secret goes on
/// stdin, where `ps` cannot see it, whenever the line fits.
const STDIN_LINE: usize = 4000;

impl Vault {
    pub fn new(data: &Path, config: &crate::config::McpConfig) -> Self {
        Self {
            data: data.to_path_buf(),
            keychain: config.keychain && cfg!(target_os = "macos"),
        }
    }
    pub fn file(data: &Path) -> Self {
        Self {
            data: data.to_path_buf(),
            keychain: false,
        }
    }
    pub fn in_keychain(&self) -> bool {
        self.keychain
    }
    fn path(&self) -> PathBuf {
        self.data.join("mcp").join("credentials.json")
    }
    /// One keychain item per server, so an item stays small enough to go through stdin; the
    /// data directory is part of the account, so two data directories keep separate tokens as
    /// their files would.
    fn account(&self, name: &str) -> String {
        let data = self
            .data
            .canonicalize()
            .unwrap_or_else(|_| self.data.clone());
        let digest = format!("{:x}", Sha256::digest(data.as_os_str().as_encoded_bytes()));
        format!("{}/{name}", &digest[..16])
    }
    fn file_entries(&self) -> BTreeMap<String, Credentials> {
        std::fs::read(self.path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    fn write_file(&self, all: &BTreeMap<String, Credentials>) -> Result<()> {
        let path = self.path();
        if all.is_empty() {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        std::fs::create_dir_all(path.parent().expect("credentials directory"))?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        serde_json::to_writer_pretty(file, all)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // An existing file keeps its own mode, so it is set either way.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Credentials> {
        if !self.keychain {
            return self.file_entries().remove(name);
        }
        let found = std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SERVICE, "-a"])
            .arg(self.account(name))
            .arg("-w")
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| serde_json::from_slice(output.stdout.trim_ascii()).ok());
        if found.is_some() {
            return found;
        }
        // A token signed in before the keychain was used moves there the first time it is read.
        let mut file = self.file_entries();
        let moved = file.remove(name)?;
        if self.put(name, &moved).is_ok() {
            let _ = self.write_file(&file);
        }
        Some(moved)
    }

    pub fn put(&self, name: &str, credentials: &Credentials) -> Result<()> {
        if !self.keychain {
            let mut all = self.file_entries();
            all.insert(name.to_owned(), credentials.clone());
            return self.write_file(&all);
        }
        let account = self.account(name);
        let secret = hex(&serde_json::to_vec(credentials)?);
        let line =
            format!("add-generic-password -U -a \"{account}\" -s \"{SERVICE}\" -X \"{secret}\"\n");
        let output = if line.len() <= STDIN_LINE {
            use std::io::Write;
            let mut child = std::process::Command::new("/usr/bin/security")
                .arg("-i")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .expect("security stdin")
                .write_all(line.as_bytes())?;
            child.wait_with_output()?
        } else {
            // Past the line `security -i` can read, the one way left is argv, as Claude Code
            // falls back too: visible to `ps` for as long as the command runs.
            tracing::debug!("MCP token too large for security -i; writing it through argv");
            std::process::Command::new("/usr/bin/security")
                .args([
                    "add-generic-password",
                    "-U",
                    "-a",
                    &account,
                    "-s",
                    SERVICE,
                    "-X",
                ])
                .arg(&secret)
                .stdout(std::process::Stdio::null())
                .output()?
        };
        // `security -i` exits 0 even when the command it read failed, so what it wrote is read
        // back rather than trusted.
        ensure!(
            output.status.success() && self.keychain_has(&account, credentials),
            "the keychain did not take the token: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(())
    }
    fn keychain_has(&self, account: &str, credentials: &Credentials) -> bool {
        std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", SERVICE, "-a", account, "-w"])
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .and_then(|output| {
                serde_json::from_slice::<Credentials>(output.stdout.trim_ascii()).ok()
            })
            .is_some_and(|stored| stored.access_token == credentials.access_token)
    }

    /// Forgets one server's token. `false` means there was nothing to forget.
    pub fn forget(&self, name: &str) -> Result<bool> {
        let mut file = self.file_entries();
        let mut removed = file.remove(name).is_some();
        if removed {
            self.write_file(&file)?;
        }
        if self.keychain {
            removed |= std::process::Command::new("/usr/bin/security")
                .args(["delete-generic-password", "-s", SERVICE, "-a"])
                .arg(self.account(name))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
        }
        Ok(removed)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        // Said, rather than left for a CDN to guess at: Notion, Sentry and Atlassian answer an
        // anonymous `Python-urllib` with something other than their 401 (2026-09-22).
        .user_agent(concat!("orochi/", env!("CARGO_PKG_VERSION")))
        .build()?)
}
async fn json(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body = response.text().await?;
    ensure!(
        status.is_success(),
        "{status}: {}",
        body.chars().take(200).collect::<String>()
    );
    Ok(serde_json::from_str(&body)?)
}

/// Where the server said to send the user, and what to call the token endpoint with.
#[derive(Debug, Clone)]
struct Endpoints {
    authorization: String,
    token: String,
    registration: Option<String>,
    /// The canonical resource identifier the token is for (RFC 8707), from the server's own
    /// metadata where it gives one, so the token is bound to this MCP server and no other.
    resource: String,
    scopes: Vec<String>,
}

/// What a `401` said: where its protected resource metadata is and which scopes this request
/// needs, either of which it may leave out.
#[derive(Debug, Default)]
struct Challenge {
    metadata: Option<String>,
    scope: Option<String>,
}

/// One quoted or bare parameter of a `WWW-Authenticate: Bearer …` header (RFC 6750 §3).
fn parameter(header: &str, name: &str) -> Option<String> {
    let mut rest = header;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().last();
        let after = &rest[at + name.len()..];
        rest = after;
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let Some(value) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        return Some(match value.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next().unwrap_or_default().to_owned(),
            None => value
                .split([',', ' '])
                .next()
                .unwrap_or_default()
                .to_owned(),
        })
        .filter(|v| !v.is_empty());
    }
    None
}

/// `Some` when the endpoint refused for want of a token, `None` when it answered without one.
async fn challenge(url: &str, token: Option<&str>) -> Result<Option<Challenge>> {
    let mut request = client()?
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"orochi","version":"probe"}}}"#,
        );
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = request.send().await?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(None);
    }
    let header = response
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    Ok(Some(Challenge {
        metadata: parameter(header, "resource_metadata"),
        scope: parameter(header, "scope"),
    }))
}

/// `(origin, path)` of a URL, the path without its trailing slash.
fn split(url: &reqwest::Url) -> (String, String) {
    let origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default())
        + &url.port().map(|p| format!(":{p}")).unwrap_or_default();
    (origin, url.path().trim_end_matches('/').to_owned())
}

/// Where RFC 9728 puts a resource's metadata: under the endpoint's own path, then at the root —
/// the order the MCP specification (2025-11-25) says to try when the `401` names none.
fn resource_metadata(url: &reqwest::Url) -> Vec<String> {
    let (origin, path) = split(url);
    let mut candidates = vec![format!(
        "{origin}/.well-known/oauth-protected-resource{path}"
    )];
    if !path.is_empty() {
        candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));
    }
    candidates
}

/// Where an authorization server's metadata may be, in the order the MCP specification
/// (2025-11-25) says a client MUST try them: for an issuer with a path, RFC 8414 path insertion,
/// then OpenID Connect path insertion, then OpenID Connect path appending; for one without, the
/// two at the root.
fn server_metadata(issuer: &reqwest::Url) -> Vec<String> {
    let (origin, path) = split(issuer);
    if path.is_empty() {
        return vec![
            format!("{origin}/.well-known/oauth-authorization-server"),
            format!("{origin}/.well-known/openid-configuration"),
        ];
    }
    vec![
        format!("{origin}/.well-known/oauth-authorization-server{path}"),
        format!("{origin}/.well-known/openid-configuration{path}"),
        format!("{origin}{path}/.well-known/openid-configuration"),
    ]
}

async fn metadata(candidates: Vec<String>) -> Option<Value> {
    let client = client().ok()?;
    for candidate in candidates {
        if let Ok(response) = client.get(&candidate).send().await
            && let Ok(value) = json(response).await
        {
            return Some(value);
        }
    }
    None
}

async fn discover(url: &str) -> Result<Endpoints> {
    let parsed = reqwest::Url::parse(url).context("invalid MCP server url")?;
    let challenged = challenge(url, None)
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    // The metadata the 401 named comes first; the well-known places are where to look without.
    let mut candidates: Vec<String> = challenged.metadata.into_iter().collect();
    candidates.extend(resource_metadata(&parsed));
    let protected = metadata(candidates).await;
    let resource = protected
        .as_ref()
        .and_then(|v| v["resource"].as_str())
        .unwrap_or(url)
        .to_owned();
    let issuer = match protected
        .as_ref()
        .and_then(|v| v["authorization_servers"][0].as_str())
    {
        Some(issuer) => issuer.to_owned(),
        // A server that predates RFC 9728 is its own authorization server, if it is one at all.
        None => split(&parsed).0,
    };
    let issuer_url = reqwest::Url::parse(&issuer).context("invalid authorization server")?;
    let server = metadata(server_metadata(&issuer_url))
        .await
        .with_context(|| {
            format!("{issuer} publishes no authorization server metadata; put a token in `headers`")
        })?;
    // Without this field the server does not do PKCE, and the specification has the client
    // refuse rather than send a code anyone who sees it could redeem.
    ensure!(
        server["code_challenge_methods_supported"]
            .as_array()
            .is_some_and(|methods| methods.iter().any(|m| m == "S256")),
        "{issuer} does not offer PKCE (S256), so Orochi will not sign in to it"
    );
    let endpoint = |name: &str| -> Result<String> {
        Ok(server[name]
            .as_str()
            .with_context(|| format!("{issuer} names no {name}"))?
            .to_owned())
    };
    // The scope the 401 asked for is authoritative for this request; the resource's own list
    // is what to ask for without one.
    let scopes = match challenged.scope {
        Some(scope) => scope.split_whitespace().map(str::to_owned).collect(),
        None => protected
            .as_ref()
            .and_then(|v| v["scopes_supported"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|s| s.as_str().map(str::to_owned))
            .collect(),
    };
    Ok(Endpoints {
        authorization: endpoint("authorization_endpoint")?,
        token: endpoint("token_endpoint")?,
        registration: server["registration_endpoint"].as_str().map(str::to_owned),
        resource,
        scopes,
    })
}

/// A public client registered on the spot, which is how an MCP client with no prior
/// relationship to the server gets an ID at all (RFC 7591).
async fn register(endpoint: &str, redirect: &str) -> Result<(String, Option<String>)> {
    let body = serde_json::json!({
        "client_name": "Orochi",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
    });
    let response = client()?
        .post(endpoint)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await?;
    let registered = json(response).await.context("client registration failed")?;
    let id = registered["client_id"]
        .as_str()
        .context("registration returned no client_id")?
        .to_owned();
    Ok((id, registered["client_secret"].as_str().map(str::to_owned)))
}

fn base64url(bytes: &[u8]) -> String {
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut block = [0u8; 3];
        block[..chunk.len()].copy_from_slice(chunk);
        let packed = u32::from_be_bytes([0, block[0], block[1], block[2]]);
        for i in 0..chunk.len() + 1 {
            out.push(SET[(packed >> (18 - 6 * i) & 0x3f) as usize] as char);
        }
    }
    out
}
fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// A sign-in waiting for the browser. It is two steps so the caller can show the URL — and the
/// console can keep drawing — before anything blocks.
pub struct Pending {
    name: String,
    url: String,
    server_url: String,
    redirect: String,
    verifier: String,
    state: String,
    client_id: String,
    client_secret: Option<String>,
    endpoints: Endpoints,
    listener: tokio::net::TcpListener,
}

pub async fn begin(server: &McpServerConfig, vault: &Vault) -> Result<Pending> {
    ensure!(
        matches!(server.transport, McpTransport::Http | McpTransport::Sse),
        "{} is a stdio server; it starts a command and signs in however that command does",
        server.name
    );
    // Bound before the authorization URL is built: the redirect must name the port that is
    // already listening, and the server will compare it exactly.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!(
        "http://127.0.0.1:{}/callback",
        listener.local_addr()?.port()
    );
    let endpoints = discover(&server.url).await?;
    let reuse = vault
        .get(&server.name)
        .filter(|c| c.url == server.url && c.token_endpoint == endpoints.token);
    let (client_id, client_secret) = match (reuse.as_ref(), &endpoints.registration) {
        // A client the user was given by hand comes first: a server that hands out client IDs
        // out of band usually registers none on the spot.
        _ if !server.oauth_client_id.is_empty() => (server.oauth_client_id.clone(), None),
        (Some(credentials), _) => (
            credentials.client_id.clone(),
            credentials.client_secret.clone(),
        ),
        (None, Some(endpoint)) => register(endpoint, &redirect).await?,
        (None, None) => bail!(
            "{} offers no dynamic client registration; set `oauth_client_id` for it, or put a \
             token in its `headers`",
            server.name
        ),
    };
    let verifier = secret();
    let state = secret();
    let mut url = reqwest::Url::parse(&endpoints.authorization)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("state", &state)
            .append_pair("code_challenge", &base64url(&Sha256::digest(&verifier)))
            .append_pair("code_challenge_method", "S256")
            .append_pair("resource", &endpoints.resource);
        if !endpoints.scopes.is_empty() {
            query.append_pair("scope", &endpoints.scopes.join(" "));
        }
    }
    Ok(Pending {
        name: server.name.clone(),
        url: url.to_string(),
        server_url: server.url.clone(),
        redirect,
        verifier,
        state,
        client_id,
        client_secret,
        endpoints,
        listener,
    })
}

impl Pending {
    pub fn url(&self) -> &str {
        &self.url
    }
    /// Asks the desktop to open it. Failing is fine: the URL was printed either way.
    pub fn open_browser(&self) {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        if let Some(command) = crate::discovery::locate(opener) {
            let _ = std::process::Command::new(command)
                .arg(&self.url)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
    }
    /// Waits for the browser to come back. Kept apart from redeeming the code so the caller can
    /// offer the other way in at the same time: over SSH the browser is on another machine and
    /// nothing ever reaches this loopback socket.
    pub async fn wait(&self) -> Result<String> {
        let since = crate::interrupt::mark();
        tokio::select! {
            result = tokio::time::timeout(CALLBACK_TIMEOUT, self.callback()) => {
                result.context("the browser did not come back in time")?
            }
            _ = since.wait() => bail!("sign-in cancelled"),
        }
    }
    /// The code out of the address the browser ended up at, pasted back by hand. It is the same
    /// check the loopback socket makes: the state has to be this sign-in's.
    pub fn pasted(&self, redirect: &str) -> Result<String> {
        let url = reqwest::Url::parse(redirect.trim())
            .context("that is not the address the browser ended up at")?;
        let pairs: BTreeMap<_, _> = url.query_pairs().collect();
        if let Some(error) = pairs.get("error") {
            bail!("{error}");
        }
        ensure!(
            pairs.get("state").map(|s| s.as_ref()) == Some(self.state.as_str()),
            "state did not match; sign-in refused"
        );
        Ok(pairs
            .get("code")
            .context("no code in that address")?
            .to_string())
    }
    /// Waits for the browser, then exchanges and stores. The two steps apart are `wait` and
    /// `redeem`.
    pub async fn finish(self, vault: &Vault) -> Result<()> {
        let code = self.wait().await?;
        self.redeem(code, vault).await
    }
    /// Exchanges the code for a token and stores it.
    pub async fn redeem(self, code: String, vault: &Vault) -> Result<()> {
        let mut form = vec![
            ("grant_type", "authorization_code".to_owned()),
            ("code", code),
            ("redirect_uri", self.redirect.clone()),
            ("client_id", self.client_id.clone()),
            ("code_verifier", self.verifier.clone()),
            ("resource", self.endpoints.resource.clone()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let response = client()?
            .post(&self.endpoints.token)
            .form(&form)
            .send()
            .await?;
        let token = json(response).await.context("token exchange failed")?;
        let credentials = Credentials {
            url: self.server_url,
            access_token: token["access_token"]
                .as_str()
                .context("no access_token in the reply")?
                .to_owned(),
            refresh_token: token["refresh_token"].as_str().map(str::to_owned),
            expires_at: token["expires_in"]
                .as_i64()
                .map(|secs| crate::types::now() + secs),
            client_id: self.client_id,
            client_secret: self.client_secret,
            token_endpoint: self.endpoints.token,
            resource: self.endpoints.resource,
        };
        vault.put(&self.name, &credentials)
    }

    /// One request on the loopback socket: the redirect the authorization server sent the
    /// browser to. `state` must be the one this sign-in generated, or the code belongs to
    /// someone else's sign-in.
    async fn callback(&self) -> Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        loop {
            let (mut socket, _) = self.listener.accept().await?;
            let mut request = vec![0u8; 8192];
            let read = socket.read(&mut request).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&request[..read]);
            let target = head
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or_default();
            let url = reqwest::Url::parse("http://127.0.0.1")
                .expect("loopback base")
                .join(target)
                .ok();
            let pairs: BTreeMap<String, String> = url
                .as_ref()
                .map(|url| {
                    url.query_pairs()
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect()
                })
                .unwrap_or_default();
            let answer = |text: &str| {
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\nconnection: \
                     close\r\ncontent-length: {}\r\n\r\n{text}",
                    text.len()
                )
            };
            let outcome = match (pairs.get("code"), pairs.get("error")) {
                (Some(code), _) if pairs.get("state") == Some(&self.state) => Ok(code.clone()),
                (Some(_), _) => Err("state did not match; sign-in refused".to_owned()),
                (None, Some(error)) => Err(format!(
                    "{error}{}",
                    pairs
                        .get("error_description")
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                )),
                // A browser asking for something else on this port (a favicon, say) is not the
                // redirect; keep waiting for the one that is.
                (None, None) => {
                    let _ = socket
                        .write_all(answer("<p>Waiting for the sign-in.</p>").as_bytes())
                        .await;
                    continue;
                }
            };
            let page = match &outcome {
                Ok(_) => "<p>Signed in. You can close this tab and go back to Orochi.</p>",
                Err(_) => "<p>Sign-in failed. Go back to Orochi for the reason.</p>",
            };
            let _ = socket.write_all(answer(page).as_bytes()).await;
            let _ = socket.flush().await;
            return outcome.map_err(|error| anyhow::anyhow!(error));
        }
    }
}

async fn refresh(credentials: &Credentials) -> Result<Credentials> {
    let token = credentials
        .refresh_token
        .as_ref()
        .context("no refresh token")?;
    let mut form = vec![
        ("grant_type", "refresh_token".to_owned()),
        ("refresh_token", token.clone()),
        ("client_id", credentials.client_id.clone()),
        ("resource", credentials.resource.clone()),
    ];
    if let Some(secret) = &credentials.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    let response = client()?
        .post(&credentials.token_endpoint)
        .form(&form)
        .send()
        .await?;
    let reply = json(response).await?;
    Ok(Credentials {
        access_token: reply["access_token"]
            .as_str()
            .context("no access_token in the reply")?
            .to_owned(),
        // A server that rotates refresh tokens returns the next one; one that does not keeps
        // the one we have.
        refresh_token: reply["refresh_token"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| credentials.refresh_token.clone()),
        expires_at: reply["expires_in"]
            .as_i64()
            .map(|secs| crate::types::now() + secs),
        ..credentials.clone()
    })
}

/// Hands each remote server the token Orochi holds for it, refreshing one that is about to
/// expire. The token travels to the agent in the server's headers, which is the point: the
/// agent is what connects. Anything worth saying comes back as a line.
pub async fn authorize(servers: &mut [McpServerConfig], vault: &Vault) -> Vec<String> {
    let mut notes = Vec::new();
    for server in servers.iter_mut() {
        if !matches!(server.transport, McpTransport::Http | McpTransport::Sse) {
            continue;
        }
        // A token the user put in the configuration themselves is theirs to manage.
        if server
            .headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"))
        {
            continue;
        }
        let Some(credentials) = vault.get(&server.name) else {
            continue;
        };
        if credentials.url != server.url {
            notes.push(format!(
                "MCP server {} now points somewhere else; sign in again with /mcp login {}",
                server.name, server.name
            ));
            continue;
        }
        let credentials = if credentials.expired(crate::types::now()) {
            match refresh(&credentials).await {
                Ok(refreshed) => {
                    if let Err(error) = vault.put(&server.name, &refreshed) {
                        tracing::debug!(%error, "refreshed MCP token not stored");
                    }
                    refreshed
                }
                Err(error) => {
                    notes.push(format!(
                        "MCP server {} could not be refreshed ({error:#}); sign in again with \
                         /mcp login {}",
                        server.name, server.name
                    ));
                    continue;
                }
            }
        } else {
            credentials
        };
        server.headers.insert(
            "Authorization".into(),
            format!("Bearer {}", credentials.access_token),
        );
    }
    notes
}

/// Where one server stands, for `/mcp` and `orochi mcp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// A command Orochi starts through the agent; it signs in however that command does.
    Stdio,
    /// The user put their own credential in the server's headers.
    Configured,
    SignedIn {
        expires_in: Option<i64>,
    },
    /// Signed in once, and the token could not be renewed.
    Stale,
    NeedsSignIn,
    /// It answered without a token.
    Open,
    Unreachable(String),
}

/// Asks the server itself, because a stored token that the server no longer honors looks
/// exactly like a good one from here.
pub async fn state(server: &McpServerConfig, vault: &Vault) -> State {
    if !matches!(server.transport, McpTransport::Http | McpTransport::Sse) {
        return State::Stdio;
    }
    if server
        .headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("authorization"))
    {
        return State::Configured;
    }
    let credentials = vault.get(&server.name).filter(|c| c.url == server.url);
    let now = crate::types::now();
    let token = match credentials {
        Some(credentials) if credentials.expired(now) => match refresh(&credentials).await {
            Ok(refreshed) => {
                let _ = vault.put(&server.name, &refreshed);
                Some(refreshed)
            }
            Err(_) => return State::Stale,
        },
        other => other,
    };
    match challenge(&server.url, token.as_ref().map(|c| c.access_token.as_str())).await {
        Ok(None) if token.is_some() => State::SignedIn {
            expires_in: token.and_then(|c| c.expires_at).map(|at| at - now),
        },
        Ok(None) => State::Open,
        Ok(Some(_)) if token.is_some() => State::Stale,
        Ok(Some(_)) => State::NeedsSignIn,
        Err(error) => State::Unreachable(format!("{error:#}")),
    }
}
