//! The MCP servers an agent session is given: the ones configured here, the ones the target
//! repository names in its own file, and the ones an ACP client asked Orochi to pass on.
//! Orochi's own coordination server lives in `mailbox.rs` and is not one of them; it is
//! attached first and can never be replaced. A remote server's sign-in is in `oauth.rs`.
pub mod oauth;

use crate::config::{McpConfig, McpServerConfig, McpTransport};
use agent_client_protocol::schema::v1::{
    EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerSse, McpServerStdio,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

/// What an agent said it takes beyond stdio, which every ACP agent must support.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Support {
    pub http: bool,
    pub sse: bool,
}

/// What a server has to say for itself before Orochi will hand it to an agent. The same rule
/// reads the user's configuration and a repository's own file, so a name that would shadow the
/// mailbox or a command Orochi would not run is refused in both.
pub fn check(server: &McpServerConfig) -> Result<()> {
    ensure!(
        !server.name.is_empty()
            && server
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "invalid MCP server name: {}",
        server.name
    );
    ensure!(
        server.name != crate::mailbox::SERVER_NAME,
        "{} is Orochi's own MCP server",
        crate::mailbox::SERVER_NAME
    );
    match server.transport {
        McpTransport::Stdio => {
            ensure!(
                !server.command.trim().is_empty(),
                "MCP server {} has no command",
                server.name
            );
            ensure!(
                Path::new(&server.command).is_absolute()
                    || Path::new(&server.command).components().count() == 1,
                "use an absolute path for MCP executables outside PATH: {}",
                server.name
            );
            ensure!(
                server.url.is_empty()
                    && server.headers.is_empty()
                    && server.oauth_client_id.is_empty(),
                "MCP server {} is stdio: it takes a command, not a url to sign in to",
                server.name
            );
        }
        McpTransport::Http | McpTransport::Sse => {
            ensure!(
                endpoint(&server.url),
                "MCP server {} needs an https:// url (http:// only for localhost)",
                server.name
            );
            ensure!(
                server.command.is_empty() && server.args.is_empty() && server.env.is_empty(),
                "MCP server {} is remote: it takes a url, not a command",
                server.name
            );
        }
    }
    Ok(())
}

/// Plaintext leaves the machine only when the user points it at their own.
fn endpoint(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', ':']).next().unwrap_or_default();
        return matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    }
    url.strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
}

/// Where a server came from, and — for one the repository's own file names — what the user
/// decided about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Configured,
    Repository { approval: Approval, digest: String },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Approval {
    Approved,
    Pending,
    Rejected,
}

/// What the user decided about one repository's own servers: what Claude Code keeps in the
/// project's `settings.local.json` as `enabledMcpjsonServers`, `disabledMcpjsonServers` and
/// `enableAllProjectMcpServers`. Orochi never writes into the repository, so it keeps them in its
/// own data, keyed by the repository.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Choices {
    /// "Use this and all future MCP servers in this project".
    pub all: bool,
    /// Each approved server with the definition that was approved. A definition changed
    /// afterwards is asked about again: rewriting an approved entry is exactly how a name the
    /// user trusted would be made to start something else.
    pub enabled: BTreeMap<String, String>,
    pub disabled: std::collections::BTreeSet<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Use,
    UseAll,
    Refuse,
}

fn choices_file(data: &Path) -> std::path::PathBuf {
    data.join("mcp").join("choices.json")
}
fn all_choices(data: &Path) -> BTreeMap<String, Choices> {
    std::fs::read(choices_file(data))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
fn write_choices(data: &Path, all: &BTreeMap<String, Choices>) -> Result<()> {
    let path = choices_file(data);
    std::fs::create_dir_all(path.parent().expect("choices directory"))?;
    std::fs::write(path, serde_json::to_vec_pretty(all)?)?;
    Ok(())
}
/// What was decided about `repository`'s own servers (its salted id, as telemetry names it).
pub fn choices(data: &Path, repository: &str) -> Choices {
    all_choices(data).remove(repository).unwrap_or_default()
}
pub fn decide(
    data: &Path,
    repository: &str,
    name: &str,
    digest: &str,
    decision: Decision,
) -> Result<()> {
    let mut all = all_choices(data);
    let choices = all.entry(repository.to_owned()).or_default();
    match decision {
        Decision::Use | Decision::UseAll => {
            choices.disabled.remove(name);
            choices.enabled.insert(name.to_owned(), digest.to_owned());
            choices.all |= decision == Decision::UseAll;
        }
        Decision::Refuse => {
            choices.enabled.remove(name);
            choices.disabled.insert(name.to_owned());
        }
    }
    write_choices(data, &all)
}
/// `reset-project-choices`: the next console session in this repository asks again.
pub fn reset(data: &Path, repository: &str) -> Result<bool> {
    let mut all = all_choices(data);
    let removed = all.remove(repository).is_some();
    if removed {
        write_choices(data, &all)?;
    }
    Ok(removed)
}

fn approval(config: &McpConfig, choices: &Choices, name: &str, digest: &str) -> Approval {
    if choices.disabled.contains(name) {
        Approval::Rejected
    } else if config.enable_all_repository_servers
        || choices.all
        || choices.enabled.get(name).is_some_and(|d| d == digest)
    {
        Approval::Approved
    } else {
        Approval::Pending
    }
}

/// Every server named for `root` with where it came from — configuration first, then what the
/// repository's own file names that the configuration does not — and what is worth saying about
/// that file: which of its servers Orochi would not run, and why.
pub fn inventory(
    config: &McpConfig,
    root: &Path,
    choices: &Choices,
) -> (Vec<(McpServerConfig, Source)>, Vec<String>) {
    let mut servers = Vec::new();
    let mut notes = Vec::new();
    // A token belongs in the environment rather than in a configuration file, so `${VAR}` is
    // expanded in what carries one. What the server *is* — its command, its url — is validated
    // as written, so it is taken as written.
    for server in &config.servers {
        let expanded = pairs(&server.env, false).and_then(|env| {
            Ok(McpServerConfig {
                env,
                headers: pairs(&server.headers, false)?,
                ..server.clone()
            })
        });
        match expanded {
            Ok(server) => servers.push((server, Source::Configured)),
            Err(reason) => notes.push(format!("MCP server {} left out: {reason}", server.name)),
        }
    }
    if config.trust_repository {
        let (found, refused) = repository(root);
        for (server, digest) in found {
            if !servers.iter().any(|(s, _)| s.name == server.name) {
                let approval = approval(config, choices, &server.name, &digest);
                servers.push((server, Source::Repository { approval, digest }));
            }
        }
        notes.extend(refused.into_iter().map(|(name, reason)| {
            format!("MCP server {name} in this repository's .mcp.json left out: {reason}")
        }));
    }
    (servers, notes)
}

/// What a session is given: every server but the repository's rejected ones, and what is worth
/// saying about the repository's file. A pending server is given too — a run nobody can be asked
/// in loads it, as Claude Code does in `-p` and SDK sessions — and the console asks about it
/// before any turn it runs (`pending`), so none reaches a console turn undecided.
pub fn servers(
    config: &McpConfig,
    root: &Path,
    choices: &Choices,
) -> (Vec<McpServerConfig>, Vec<String>) {
    let (inventory, mut notes) = inventory(config, root, choices);
    let mut taken = Vec::new();
    let mut servers = Vec::new();
    for (server, source) in inventory {
        match source {
            Source::Repository {
                approval: Approval::Rejected,
                ..
            } => continue,
            Source::Repository {
                approval: Approval::Pending,
                ..
            } => taken.push(server.name.clone()),
            Source::Repository { .. } | Source::Configured => {}
        }
        servers.push(server);
    }
    // Only what nobody approved is worth saying: an approved server was the user's decision.
    if !taken.is_empty() {
        notes.insert(
            0,
            format!(
                "using {} from this repository's .mcp.json without asking; the console asks \
                 before it uses them",
                taken.join(", ")
            ),
        );
    }
    (servers, notes)
}

/// The repository's servers nobody has decided about yet, with the definition to approve, for
/// the console to ask about.
pub fn pending(
    config: &McpConfig,
    root: &Path,
    choices: &Choices,
) -> Vec<(McpServerConfig, String)> {
    inventory(config, root, choices)
        .0
        .into_iter()
        .filter_map(|(server, source)| match source {
            Source::Repository {
                approval: Approval::Pending,
                digest,
            } => Some((server, digest)),
            _ => None,
        })
        .collect()
}

/// What a repository's own file has to say is said the first time a run in it is given its
/// servers: the repository gets to start a command here, so it is said — and said once, not
/// on every turn of a conversation.
pub fn once(root: &Path, notes: Vec<String>) -> Vec<String> {
    static ANNOUNCED: std::sync::Mutex<Option<std::collections::BTreeSet<std::path::PathBuf>>> =
        std::sync::Mutex::new(None);
    if notes.is_empty() {
        return notes;
    }
    let mut announced = ANNOUNCED.lock().expect("announced repositories");
    match announced
        .get_or_insert_with(Default::default)
        .insert(root.to_path_buf())
    {
        true => notes,
        false => vec![],
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryFile {
    #[serde(default)]
    mcp_servers: BTreeMap<String, RepositoryServer>,
}
#[derive(Deserialize, Serialize)]
struct RepositoryServer {
    #[serde(default, rename = "type")]
    transport: Option<String>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

/// A repository's servers, each with a digest of its definition as written, and the ones refused
/// with the reason.
type Found = (Vec<(McpServerConfig, String)>, Vec<(String, String)>);

/// Read as empty in a repository server's remote `url` and `headers`, so a project's file cannot
/// have an agent send its own credentials to a server the file names. The first group is Claude
/// Code's own list; the rest are what the other agents Orochi drives sign in with, and proxy
/// settings, which carry credentials in their URL.
const WITHHELD: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "HTTPS_PROXY",
    "NPM_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "OPENAI_API_KEY",
    "CODEX_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "HTTP_PROXY",
    "ALL_PROXY",
    "https_proxy",
    "http_proxy",
    "all_proxy",
];

/// The target repository's `.mcp.json`, in the shape the CLIs already write: the servers it
/// names, each with a digest of its definition as written, and the ones Orochi would not run,
/// with the reason. A file that cannot be read at all is nothing — it is the repository's text,
/// not the user's, and no run fails over it.
fn repository(root: &Path) -> Found {
    let Ok(text) = std::fs::read_to_string(root.join(".mcp.json")) else {
        return (vec![], vec![]);
    };
    let file = match serde_json::from_str::<RepositoryFile>(&text) {
        Ok(file) => file,
        Err(error) => {
            tracing::debug!(%error, "ignoring unreadable .mcp.json");
            return (vec![], vec![]);
        }
    };
    let mut found = Vec::new();
    let mut refused = Vec::new();
    for (name, server) in file.mcp_servers {
        let transport = match server.transport.as_deref() {
            Some("http") => McpTransport::Http,
            Some("sse") => McpTransport::Sse,
            Some("stdio") | None => McpTransport::Stdio,
            Some(other) => {
                refused.push((name, format!("unknown transport {other}")));
                continue;
            }
        };
        // Digested as written, before expansion, so rotating a token in the environment is not
        // a new definition and changing what the file says is.
        let digest = format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(serde_json::to_vec(&server).unwrap_or_default())
        );
        let remote = transport != McpTransport::Stdio;
        let expanded = (|| {
            Ok::<_, String>(McpServerConfig {
                name: name.clone(),
                transport,
                command: expand(&server.command, false)?,
                args: server
                    .args
                    .iter()
                    .map(|a| expand(a, false))
                    .collect::<Result<_, _>>()?,
                env: pairs(&server.env, false)?,
                url: expand(&server.url, remote)?,
                headers: pairs(&server.headers, remote)?,
                // A repository names a server, never the client this machine signs in as.
                oauth_client_id: String::new(),
            })
        })();
        match expanded.and_then(|server| match check(&server) {
            Ok(()) => Ok(server),
            Err(error) => Err(format!("{error}")),
        }) {
            Ok(server) => found.push((server, digest)),
            Err(reason) => refused.push((name, reason)),
        }
    }
    (found, refused)
}

fn pairs(
    values: &BTreeMap<String, String>,
    withhold: bool,
) -> Result<BTreeMap<String, String>, String> {
    values
        .iter()
        .map(|(k, v)| expand(v, withhold).map(|v| (k.clone(), v)))
        .collect()
}

/// `${VAR}` and `${VAR:-default}`, which is how the CLIs' own `.mcp.json` keeps a token out of
/// a committed file. A variable that is set nowhere and defaults to nothing is not expanded to
/// nothing: the server is left out and the reason says which variable, because a token-shaped
/// hole is a failure to read afterwards rather than one to read here. `withhold` reads the
/// `WITHHELD` credentials as empty, as Claude Code does for what a project file sends away.
fn expand(value: &str, withhold: bool) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}').map(|i| start + i) else {
            break;
        };
        let (name, fallback) = match rest[start + 2..end].split_once(":-") {
            Some((name, fallback)) => (name, Some(fallback)),
            None => (&rest[start + 2..end], None),
        };
        if withhold && WITHHELD.contains(&name) {
            rest = &rest[end + 1..];
            continue;
        }
        match (std::env::var(name), fallback) {
            (Ok(value), _) => out.push_str(&value),
            (Err(_), Some(fallback)) => out.push_str(fallback),
            (Err(_), None) => return Err(format!("${name} is not set")),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The ACP form of what this agent can take. A stdio command is resolved the way agent
/// executables are, because ACP asks for an absolute path; anything left out comes back as
/// (name, reason), for whoever is reporting to name the agent that could not take it.
pub fn attach(
    servers: &[McpServerConfig],
    support: Support,
) -> (Vec<McpServer>, Vec<(String, String)>) {
    let mut attached = Vec::new();
    let mut skipped = Vec::new();
    for server in servers {
        let name = server.name.clone();
        match server.transport {
            McpTransport::Stdio => match crate::discovery::locate(&server.command) {
                Some(command) => attached.push(McpServer::Stdio(
                    McpServerStdio::new(name, command)
                        .args(server.args.clone())
                        .env(
                            server
                                .env
                                .iter()
                                .map(|(k, v)| EnvVariable::new(k, v))
                                .collect(),
                        ),
                )),
                None => skipped.push((name, format!("executable not found ({})", server.command))),
            },
            McpTransport::Http if support.http => attached.push(McpServer::Http(
                McpServerHttp::new(name, server.url.clone()).headers(headers(server)),
            )),
            McpTransport::Sse if support.sse => attached.push(McpServer::Sse(
                McpServerSse::new(name, server.url.clone()).headers(headers(server)),
            )),
            McpTransport::Http | McpTransport::Sse => {
                skipped.push((name, "it takes no remote MCP server".into()))
            }
        }
    }
    (attached, skipped)
}

fn headers(server: &McpServerConfig) -> Vec<HttpHeader> {
    server
        .headers
        .iter()
        .map(|(k, v)| HttpHeader::new(k, v))
        .collect()
}

/// What an ACP client asked Orochi to pass on to whichever agent runs its prompt. Orochi
/// advertises no MCP transport beyond stdio, so anything else is a request it never invited.
pub fn forwarded(servers: &[McpServer]) -> Result<Vec<McpServerConfig>> {
    let mut forwarded: Vec<McpServerConfig> = Vec::new();
    for server in servers {
        let McpServer::Stdio(stdio) = server else {
            anyhow::bail!("only stdio MCP servers are accepted");
        };
        let server = McpServerConfig {
            name: stdio.name.clone(),
            transport: McpTransport::Stdio,
            command: stdio.command.to_string_lossy().into_owned(),
            args: stdio.args.clone(),
            env: stdio
                .env
                .iter()
                .map(|e| (e.name.clone(), e.value.clone()))
                .collect(),
            url: String::new(),
            headers: BTreeMap::new(),
            oauth_client_id: String::new(),
        };
        check(&server)?;
        ensure!(
            !forwarded.iter().any(|s| s.name == server.name),
            "duplicate MCP server: {}",
            server.name
        );
        forwarded.push(server);
    }
    Ok(forwarded)
}
