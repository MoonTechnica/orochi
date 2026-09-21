//! Signing in to a remote MCP server. Orochi is the OAuth client — it holds the token and
//! hands it to whichever agent runs the work — so the whole flow is driven here against a
//! fixture authorization server, with no account and no network.
use orochi::{
    config::{McpServerConfig, McpTransport},
    mcp::oauth,
};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, Command, Stdio},
};

/// The fixture MCP server, which is its own authorization server, killed on drop.
struct Server {
    child: Child,
    url: String,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Server {
    fn start(no_register: bool) -> Self {
        Self::with(if no_register {
            &[("MOCK_OAUTH_NO_REGISTER", "1")]
        } else {
            &[]
        })
    }
    /// The fixture with switches from `tests/fixtures/mock_oauth.py`'s docstring.
    fn with(switches: &[(&str, &str)]) -> Self {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_oauth.py");
        let mut command = Command::new("python3");
        command
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .envs(switches.iter().copied());
        let mut child = command.spawn().unwrap();
        let mut url = String::new();
        BufReader::new(child.stdout.as_mut().unwrap())
            .read_line(&mut url)
            .unwrap();
        Server {
            child,
            url: url.trim().to_owned(),
        }
    }
    fn server(&self) -> McpServerConfig {
        McpServerConfig {
            name: "docs".into(),
            transport: McpTransport::Http,
            url: format!("{}/mcp", self.url),
            ..Default::default()
        }
    }
}

/// Stands in for the browser: follows the authorization URL, which redirects to the loopback
/// address Orochi is listening on.
fn browser(url: &str) {
    let url = url.to_owned();
    tokio::spawn(async move {
        let _ = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap()
            .get(url)
            .send()
            .await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_in_once_hands_every_agent_the_authorized_server() {
    let fixture = Server::start(false);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let server = fixture.server();

    assert_eq!(
        oauth::state(&server, &vault).await,
        oauth::State::NeedsSignIn,
        "the server refuses Orochi until it signs in"
    );

    let pending = oauth::begin(&server, &vault).await.unwrap();
    assert!(
        pending.url().contains("code_challenge_method=S256")
            && pending.url().contains("resource=")
            && pending.url().contains("127.0.0.1"),
        "PKCE, the resource it is for, and a loopback redirect: {}",
        pending.url()
    );
    browser(pending.url());
    pending.finish(&vault).await.unwrap();

    // The token is Orochi's, kept beside memory rather than in telemetry, and readable only by
    // the user.
    let file = data.path().join("mcp/credentials.json");
    let held = std::fs::read_to_string(&file).unwrap();
    assert!(held.contains("access_token"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
    }

    // What a session is given carries it, so the agent — which is what connects — is authorized.
    let mut servers = vec![server.clone()];
    let notes = oauth::authorize(&mut servers, &vault).await;
    assert!(notes.is_empty(), "{notes:?}");
    let header = servers[0].headers["Authorization"].clone();
    assert!(header.starts_with("Bearer token-"), "{header}");
    assert_eq!(
        oauth::state(&server, &vault).await,
        oauth::State::SignedIn {
            expires_in: Some(3600)
        }
    );

    // An expired token is renewed before the run rather than at the agent's 401.
    let mut expired = vault.get("docs").unwrap();
    expired.expires_at = Some(orochi::types::now() - 10);
    vault.put("docs", &expired).unwrap();
    let mut servers = vec![server.clone()];
    assert!(oauth::authorize(&mut servers, &vault).await.is_empty());
    let renewed = servers[0].headers["Authorization"].clone();
    assert_ne!(renewed, header, "a new token, not the expired one");
    assert_eq!(
        vault.get("docs").unwrap().access_token,
        renewed.trim_start_matches("Bearer "),
        "and it is kept, so the next run does not renew again"
    );

    assert!(vault.forget("docs").unwrap());
    let mut servers = vec![server.clone()];
    assert!(oauth::authorize(&mut servers, &vault).await.is_empty());
    assert!(
        !servers[0].headers.contains_key("Authorization"),
        "forgotten means forgotten"
    );
}

/// The code must come back with the state this sign-in generated, or it belongs to someone
/// else's sign-in and is worth nothing here.
#[tokio::test(flavor = "multi_thread")]
async fn a_callback_that_does_not_carry_this_sign_ins_state_is_refused() {
    let fixture = Server::start(false);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let pending = oauth::begin(&fixture.server(), &vault).await.unwrap();
    let redirect = reqwest::Url::parse(pending.url())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .map(|(_, v)| v.into_owned())
        .unwrap();
    browser(&format!("{redirect}?code=stolen&state=someone-elses"));
    let error = pending.finish(&vault).await.unwrap_err();
    assert!(format!("{error:#}").contains("state"), "{error:#}");
    assert!(vault.get("docs").is_none());
}

/// A server with no way to register a client cannot be signed in to, and says what to do
/// instead rather than failing at the agent later.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_registers_no_client_says_to_configure_the_token_by_hand() {
    let fixture = Server::start(true);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let error = match oauth::begin(&fixture.server(), &vault).await {
        Ok(_) => panic!("there is no client to register"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("headers"), "{error:#}");
}

/// A credential the user put in the configuration themselves is theirs to manage.
#[tokio::test(flavor = "multi_thread")]
async fn a_configured_authorization_header_is_never_replaced_by_a_stored_token() {
    let fixture = Server::start(false);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let mut server = fixture.server();
    let pending = oauth::begin(&server, &vault).await.unwrap();
    browser(pending.url());
    pending.finish(&vault).await.unwrap();

    server
        .headers
        .insert("Authorization".into(), "Bearer mine".into());
    let mut servers = vec![server.clone()];
    oauth::authorize(&mut servers, &vault).await;
    assert_eq!(servers[0].headers["Authorization"], "Bearer mine");
    assert_eq!(
        oauth::state(&server, &vault).await,
        oauth::State::Configured
    );
}

/// What Claude Code and Codex both call `--no-browser`: over SSH the browser is on another
/// machine, so nothing reaches the loopback socket and the address it ended up at comes back
/// by hand. The same sign-in accepts either, and checks the same `state` either way.
#[tokio::test(flavor = "multi_thread")]
async fn a_sign_in_can_be_finished_by_pasting_the_address_the_browser_ended_up_at() {
    let fixture = Server::start(false);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let server = fixture.server();
    let pending = oauth::begin(&server, &vault).await.unwrap();

    // The browser ran somewhere else: ask the authorization server for the redirect without
    // following it, as a browser on another machine would show the user its address bar.
    let redirected = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(pending.url())
        .send()
        .await
        .unwrap();
    let landed = redirected.headers()["location"]
        .to_str()
        .unwrap()
        .to_owned();

    assert!(
        pending
            .pasted("https://example.com/callback?code=x&state=someone-elses")
            .is_err(),
        "a pasted address is held to the same state as the socket"
    );
    let code = pending.pasted(&landed).unwrap();
    pending.redeem(code, &vault).await.unwrap();

    let mut servers = vec![server.clone()];
    oauth::authorize(&mut servers, &vault).await;
    assert!(servers[0].headers["Authorization"].starts_with("Bearer token-"));
}

/// What Codex calls `--oauth-client-id`: a server that registers no client on the spot can
/// still be signed in to with the client this machine was given by hand.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_id_given_by_hand_signs_in_where_nothing_registers_one() {
    let fixture = Server::start(true);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let mut server = fixture.server();
    server.oauth_client_id = "given-by-hand".into();
    let pending = oauth::begin(&server, &vault).await.unwrap();
    assert!(
        pending.url().contains("client_id=given-by-hand"),
        "{}",
        pending.url()
    );
    browser(pending.url());
    pending.finish(&vault).await.unwrap();
    assert_eq!(vault.get("docs").unwrap().client_id, "given-by-hand");
}

/// The MCP specification (2025-11-25) has a client try an issuer's metadata in a fixed order —
/// for an issuer with a path, RFC 8414 insertion, OpenID Connect insertion, then OpenID Connect
/// appending. A server whose metadata is only in the last place is still found.
#[tokio::test(flavor = "multi_thread")]
async fn an_issuer_under_a_path_is_found_where_openid_connect_appends_to_it() {
    let fixture = Server::with(&[("MOCK_OAUTH_ISSUER_PATH", "/tenant")]);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let pending = oauth::begin(&fixture.server(), &vault).await.unwrap();
    browser(pending.url());
    pending.finish(&vault).await.unwrap();
    assert!(vault.get("docs").is_some());
}

/// Without `code_challenge_methods_supported` the server does not do PKCE, and the
/// specification has the client refuse rather than send a code anyone who sees it could redeem.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_does_not_offer_pkce_is_refused_before_anything_is_registered() {
    let fixture = Server::with(&[("MOCK_OAUTH_NO_PKCE", "1")]);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let error = match oauth::begin(&fixture.server(), &vault).await {
        Ok(_) => panic!("a server without PKCE was signed in to"),
        Err(error) => error,
    };
    assert!(format!("{error:#}").contains("PKCE"), "{error:#}");
}

/// The scope a `401` asks for is authoritative for the request it refused; the resource's own
/// `scopes_supported` is only what to ask for without one.
#[tokio::test(flavor = "multi_thread")]
async fn the_scope_the_challenge_asks_for_outranks_what_the_resource_lists() {
    let fixture = Server::with(&[("MOCK_OAUTH_SCOPE", "files:read files:write")]);
    let data = tempfile::tempdir().unwrap();
    let vault = oauth::Vault::file(data.path());
    let pending = oauth::begin(&fixture.server(), &vault).await.unwrap();
    let scope = reqwest::Url::parse(pending.url())
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "scope")
        .map(|(_, v)| v.into_owned());
    assert_eq!(scope.as_deref(), Some("files:read files:write"));
}
