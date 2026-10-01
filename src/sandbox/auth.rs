//! How an agent inside a sandbox is signed in, with the sign-in it already has on this machine,
//! each agent the way its own documentation says a headless one should be:
//!
//! - **Files** a CLI keeps its login in (`~/.codex/auth.json`) are kept in step between this
//!   machine and the sandbox's home, never mounted: a CLI replaces such a file by renaming a new
//!   one over it, which a single-file mount refuses (measured 2026-10-01). Before a run the newer
//!   side wins; after it, what the agent refreshed inside goes back, unless this machine changed
//!   the file meanwhile, in which case this machine's copy is kept.
//! - **Variables** (`CLAUDE_CODE_OAUTH_TOKEN`, API keys, a local model server's URL) come from
//!   Orochi's own store (`orochi sandbox secret`, `orochi sandbox login`), else the agent's
//!   configured `env`, else this machine's environment. They reach the agent through a `0600`
//!   file in the sandbox's memory, never through a command line `ps` can read.
//!
//! What cannot be carried — a login that lives only in this machine's keychain — is named by
//! `orochi sandbox auth` with the documented alternative, rather than failing at the first run.
use super::{HOME, Incus, Placement};
use crate::config::{AgentConfig, McpConfig, SandboxConfig};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// What an agent needs inside, by what it is.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Recipe {
    /// Paths under the home directory, the same on both sides.
    pub files: Vec<String>,
    /// Variables it reads its credentials or endpoint from.
    pub env: Vec<String>,
    /// What to do when nothing is provided, in the agent's own documented terms.
    pub hint: Option<&'static str>,
}

fn which(agent: &AgentConfig) -> String {
    let (command, _) = crate::discovery::inside(agent);
    format!("{} {}", agent.id, command).to_ascii_lowercase()
}

pub fn recipe(agent: &AgentConfig) -> Recipe {
    let name = which(agent);
    let mut recipe = if name.contains("codex") {
        // https://learn.chatgpt.com/docs/auth: auth.json may be copied to another machine.
        Recipe {
            files: vec![".codex/auth.json".into()],
            env: vec!["OPENAI_API_KEY".into(), "OPENAI_BASE_URL".into()],
            hint: Some("sign in on this machine (`codex login`); its auth.json is shared"),
        }
    } else if name.contains("claude") {
        // https://code.claude.com/docs/en/iam: a login here lives in the macOS keychain;
        // `claude setup-token` makes the long-lived token a headless session uses.
        Recipe {
            files: vec![],
            env: vec![
                "CLAUDE_CODE_OAUTH_TOKEN".into(),
                "ANTHROPIC_API_KEY".into(),
                "ANTHROPIC_AUTH_TOKEN".into(),
                "ANTHROPIC_BASE_URL".into(),
            ],
            hint: Some(
                "`orochi sandbox login claude` runs `claude setup-token` and keeps the token",
            ),
        }
    } else if name.contains("agy") || name.contains("antigravity") {
        // https://antigravity.google/docs/cli/install/: a Google sign-in is kept only in the OS
        // keyring; `modelProvider: "gemini"` with GEMINI_API_KEY needs no sign-in.
        Recipe {
            files: vec![
                ".gemini/antigravity-cli/settings.json".into(),
                ".config/gcloud/application_default_credentials.json".into(),
            ],
            env: vec!["GEMINI_API_KEY".into(), "GOOGLE_GEMINI_BASE_URL".into()],
            hint: Some(
                "a Google sign-in stays in this machine's keychain; set `modelProvider: \"gemini\"` in ~/.gemini/antigravity-cli/settings.json and `orochi sandbox secret set GEMINI_API_KEY`",
            ),
        }
    } else if name.contains("gemini") {
        // https://geminicli.com/docs/get-started/authentication: headless uses cached
        // credentials, ~/.gemini/.env, or GEMINI_API_KEY / GOOGLE_* variables.
        Recipe {
            files: vec![
                ".gemini/.env".into(),
                ".gemini/oauth_creds.json".into(),
                ".gemini/google_accounts.json".into(),
                ".config/gcloud/application_default_credentials.json".into(),
            ],
            env: vec![
                "GEMINI_API_KEY".into(),
                "GOOGLE_API_KEY".into(),
                "GOOGLE_CLOUD_PROJECT".into(),
                "GOOGLE_CLOUD_LOCATION".into(),
                "GOOGLE_GENAI_USE_VERTEXAI".into(),
            ],
            hint: Some(
                "sign in on this machine (`gemini`), or `orochi sandbox secret set GEMINI_API_KEY`",
            ),
        }
    } else {
        Recipe::default()
    };
    for file in &agent.sandbox_files {
        if !recipe.files.contains(file) {
            recipe.files.push(file.clone());
        }
    }
    for variable in &agent.sandbox_env {
        if !recipe.env.contains(variable) {
            recipe.env.push(variable.clone());
        }
    }
    recipe
}

/// A home-relative path a recipe may name: plain components, nothing that climbs out.
pub fn file_ok(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && p.is_relative()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// A variable name a recipe may name.
pub fn variable_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

// --- Secrets ---------------------------------------------------------------------------------

/// Orochi's own store of values for agents in sandboxes: the keychain where MCP tokens are
/// kept, under names of their own; their names, never their values, are listed in a file.
pub struct Secrets {
    vault: crate::mcp::oauth::Vault,
    index: PathBuf,
}
const PREFIX: &str = "sandbox-secret:";
impl Secrets {
    pub fn open(data: &Path, mcp: &McpConfig) -> Self {
        Self {
            vault: crate::mcp::oauth::Vault::new(data, mcp),
            index: data.join("sandbox/secrets.json"),
        }
    }
    pub fn names(&self) -> Vec<String> {
        std::fs::read(&self.index)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn write_names(&self, names: &[String]) -> Result<()> {
        std::fs::create_dir_all(self.index.parent().expect("parent"))?;
        std::fs::write(&self.index, serde_json::to_vec_pretty(names)?)?;
        Ok(())
    }
    pub fn get(&self, name: &str) -> Option<String> {
        self.vault
            .get(&format!("{PREFIX}{name}"))
            .map(|c| c.access_token)
    }
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        ensure!(variable_ok(name), "{name} is not a variable name");
        ensure!(!value.is_empty(), "an empty value is not a secret");
        self.vault.put(
            &format!("{PREFIX}{name}"),
            &crate::mcp::oauth::Credentials {
                url: String::new(),
                access_token: value.to_owned(),
                refresh_token: None,
                expires_at: None,
                client_id: String::new(),
                client_secret: None,
                token_endpoint: String::new(),
                resource: String::new(),
            },
        )?;
        let mut names = self.names();
        if !names.iter().any(|n| n == name) {
            names.push(name.to_owned());
            names.sort();
            self.write_names(&names)?;
        }
        Ok(())
    }
    pub fn forget(&self, name: &str) -> Result<bool> {
        let removed = self.vault.forget(&format!("{PREFIX}{name}"))?;
        let mut names = self.names();
        names.retain(|n| n != name);
        self.write_names(&names)?;
        Ok(removed)
    }
}

/// Where a variable's value comes from, for `orochi sandbox auth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Secret,
    Config,
    Environment,
}

/// The variables an agent gets inside, with where each came from.
pub fn variables(
    agent: &AgentConfig,
    recipe: &Recipe,
    secrets: &Secrets,
) -> BTreeMap<String, (String, Source)> {
    let mut out = BTreeMap::new();
    // Whatever the agent is configured with goes in as it would here, but no longer on a
    // command line: it may hold a key.
    for (key, value) in &agent.env {
        if !super::LOCAL_ONLY.contains(&key.as_str()) {
            out.insert(key.clone(), (value.clone(), Source::Config));
        }
    }
    for name in &recipe.env {
        if out.contains_key(name) {
            continue;
        }
        if let Some(value) = secrets.get(name) {
            out.insert(name.clone(), (value, Source::Secret));
        } else if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            out.insert(name.clone(), (value, Source::Environment));
        }
    }
    out
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The file in the sandbox's memory an agent's variables are read from: per agent, so one
/// agent's key is not another's.
pub fn env_file(agent: &AgentConfig) -> String {
    format!("/dev/shm/orochi-env-{}", agent.id)
}

// --- Keeping files in step -------------------------------------------------------------------

/// What both sides held when they were last the same, per file, so a change can be told apart
/// from a copy.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Bases(BTreeMap<String, String>);

fn bases_path(data: &Path, sandbox: &str) -> PathBuf {
    data.join("sandbox/auth").join(format!("{sandbox}.json"))
}
fn load_bases(data: &Path, sandbox: &str) -> Bases {
    std::fs::read(bases_path(data, sandbox))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn save_bases(data: &Path, sandbox: &str, bases: &Bases) -> Result<()> {
    let path = bases_path(data, sandbox);
    std::fs::create_dir_all(path.parent().expect("parent"))?;
    std::fs::write(path, serde_json::to_vec_pretty(bases)?)?;
    Ok(())
}
/// Forgets what a deleted sandbox's files last held.
pub fn forget(data: &Path, sandbox: &str) {
    let _ = std::fs::remove_file(bases_path(data, sandbox));
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

/// Which way one file goes, from what each side holds now and what both held last.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Nothing,
    In,
    Out,
}
pub fn step(here: Option<&str>, inside: Option<&str>, base: Option<&str>) -> Step {
    match (here, inside) {
        (None, None) => Step::Nothing,
        (Some(_), None) => Step::In,
        (None, Some(i)) if Some(i) != base => Step::Out,
        (None, Some(_)) => Step::Nothing,
        (Some(h), Some(i)) if h == i => Step::Nothing,
        // Only the inside moved since they last matched: the agent refreshed it.
        (Some(h), Some(_)) if Some(h) == base => Step::Out,
        // This machine moved, or both did: this machine's copy is the one kept.
        (Some(_), Some(_)) => Step::In,
    }
}

const READER: &str = r#"
import base64, json, os, sys
out = {}
for path in json.load(sys.stdin):
    try:
        with open(os.path.expanduser("~/" + path), "rb") as f:
            out[path] = base64.b64encode(f.read()).decode()
    except OSError:
        pass
print(json.dumps(out))
"#;

const WRITER: &str = r#"
import base64, json, os, sys
request = json.load(sys.stdin)
os.umask(0o077)
for path, data in request.get("files", {}).items():
    target = os.path.expanduser("~/" + path)
    os.makedirs(os.path.dirname(target), exist_ok=True)
    temp = target + ".orochi-tmp"
    with open(temp, "wb") as f:
        f.write(base64.b64decode(data))
    os.replace(temp, target)
env = request.get("env")
if env is not None:
    path = sys.argv[1]
    temp = path + ".tmp"
    with open(temp, "w") as f:
        f.write(env)
    os.replace(temp, path)
"#;

fn exec_python(
    config: &SandboxConfig,
    placement: &Placement,
    script: &str,
    input: &[u8],
    args: &[&str],
) -> Result<String> {
    let incus = Incus::new(config);
    let uid = placement.uid.to_string();
    let gid = placement.gid.to_string();
    let home = format!("HOME={HOME}");
    let instance = incus.instance(&placement.name);
    let mut argv = vec![
        "exec", &instance, "-T", "--user", &uid, "--group", &gid, "--env", &home, "--", "python3",
        "-c", script,
    ];
    argv.extend(args);
    incus.run_with(&argv, Some(input))
}

/// Brings the agent's credential files and variables into the sandbox before it starts.
pub fn prepare(
    config: &SandboxConfig,
    mcp: &McpConfig,
    data: &Path,
    placement: &Placement,
    agent: &AgentConfig,
) -> Result<()> {
    let recipe = recipe(agent);
    let secrets = Secrets::open(data, mcp);
    let variables = variables(agent, &recipe, &secrets);
    let mut env = String::new();
    for (key, (value, _)) in &variables {
        env.push_str(&format!("export {key}={}\n", shell_quote(value)));
    }
    let files = sync(config, data, placement, &recipe.files, true)?;
    let request = serde_json::json!({ "files": files, "env": env });
    exec_python(
        config,
        placement,
        WRITER,
        &serde_json::to_vec(&request)?,
        &[&env_file(agent)],
    )?;
    Ok(())
}

/// After a run: what the agent refreshed inside goes back to this machine.
pub fn settle(
    config: &SandboxConfig,
    data: &Path,
    placement: &Placement,
    agent: &AgentConfig,
) -> Result<()> {
    let files = sync(config, data, placement, &recipe(agent).files, false)?;
    if !files.is_empty() {
        let request = serde_json::json!({ "files": files });
        exec_python(
            config,
            placement,
            WRITER,
            &serde_json::to_vec(&request)?,
            &[],
        )?;
    }
    Ok(())
}

/// Compares both sides of each file, writes what goes out here, and returns what goes in
/// (base64, for the writer). `starting` is a run about to begin, where this machine's copy
/// going in is expected; afterwards only the agent's refresh going out is.
fn sync(
    config: &SandboxConfig,
    data: &Path,
    placement: &Placement,
    files: &[String],
    starting: bool,
) -> Result<BTreeMap<String, String>> {
    use base64_engine as b64;
    let mut into = BTreeMap::new();
    if files.is_empty() {
        return Ok(into);
    }
    let home = home()?;
    let inside: BTreeMap<String, String> = serde_json::from_str(
        exec_python(config, placement, READER, &serde_json::to_vec(files)?, &[])?.trim(),
    )
    .context("unreadable answer from the sandbox")?;
    let mut bases = load_bases(data, &placement.name);
    for path in files {
        let local = home.join(path);
        let here = std::fs::read(&local).ok();
        let there = inside.get(path).and_then(|s| b64::decode(s));
        let (h, t) = (here.as_deref().map(digest), there.as_deref().map(digest));
        let base = bases.0.get(path).cloned();
        // What both sides hold once this is done; unchanged where nothing was made equal.
        let agreed = match step(h.as_deref(), t.as_deref(), base.as_deref()) {
            Step::Nothing if h == t => h.clone(),
            Step::Nothing => base.clone(),
            Step::In if starting || base.is_some() => {
                let bytes = here.expect("present when it goes in");
                into.insert(path.clone(), b64::encode(&bytes));
                h.clone()
            }
            Step::In => base.clone(),
            Step::Out => {
                let bytes = there.expect("present when it goes out");
                write_here(&local, &bytes)?;
                t.clone()
            }
        };
        match agreed {
            Some(d) => {
                bases.0.insert(path.clone(), d);
            }
            None => {
                bases.0.remove(path);
            }
        }
    }
    save_bases(data, &placement.name, &bases)?;
    Ok(into)
}

/// Replaces a file here the way the CLIs do, keeping it private to the user.
fn write_here(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("a file has a directory")?;
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    std::io::Write::write_all(&mut temp, bytes)?;
    temp.persist(path)?;
    Ok(())
}

/// Base64 without another dependency: the standard alphabet, padded.
mod base64_engine {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * i)) as usize & 63] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }
    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        let mut buffer = 0u32;
        let mut bits = 0;
        for c in text.bytes() {
            if c == b'=' {
                break;
            }
            let v = ALPHABET.iter().position(|&a| a == c)? as u32;
            buffer = (buffer << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buffer >> bits) as u8);
                buffer &= (1 << bits) - 1;
            }
        }
        Some(out)
    }
}
pub use base64_engine::{decode as base64_decode, encode as base64_encode};

// --- What `orochi sandbox auth` shows ---------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub agent: String,
    /// Credential files present on this machine, which go in.
    pub files: Vec<String>,
    /// Variables provided, by name, with where each value comes from. Never the value.
    pub variables: Vec<(String, Source)>,
    /// Nothing at all is provided: the agent has to be signed in inside, or will fail.
    pub missing: bool,
    pub hint: Option<&'static str>,
}

pub fn report(agent: &AgentConfig, data: &Path, mcp: &McpConfig) -> Result<Report> {
    let recipe = recipe(agent);
    let home = home()?;
    let files: Vec<String> = recipe
        .files
        .iter()
        .filter(|f| home.join(f).is_file())
        .cloned()
        .collect();
    let secrets = Secrets::open(data, mcp);
    let variables: Vec<(String, Source)> = variables(agent, &recipe, &secrets)
        .into_iter()
        .filter(|(k, _)| recipe.env.contains(k))
        .map(|(k, (_, s))| (k, s))
        .collect();
    let missing = (!recipe.files.is_empty() || !recipe.env.is_empty())
        && files.is_empty()
        && variables.is_empty();
    Ok(Report {
        agent: agent.id.clone(),
        files,
        variables,
        missing,
        hint: recipe.hint,
    })
}

/// `orochi sandbox login <agent>`: the agent's own documented way to make a credential a
/// headless session can use, run here.
pub fn login(agent: &AgentConfig, data: &Path, mcp: &McpConfig) -> Result<String> {
    let name = which(agent);
    if name.contains("claude") {
        let claude = crate::discovery::locate("claude").context(
            "`claude` is not installed here; install Claude Code, or `orochi sandbox secret set ANTHROPIC_API_KEY`",
        )?;
        let status = std::process::Command::new(claude)
            .arg("setup-token")
            .status()?;
        ensure!(status.success(), "`claude setup-token` did not finish");
        let token = read_hidden("Paste the token it printed (not shown): ")?;
        ensure!(!token.is_empty(), "no token was pasted");
        Secrets::open(data, mcp).set("CLAUDE_CODE_OAUTH_TOKEN", &token)?;
        return Ok(
            "Claude Code in sandboxes now signs in with this token (valid for a year).".into(),
        );
    }
    let recipe = recipe(agent);
    match recipe.hint {
        Some(hint) => Ok(format!("{}: {hint}.", agent.id)),
        None => bail!(
            "{} has no known sign-in to carry; list its files in `sandbox_files` or its variables in `sandbox_env`",
            agent.id
        ),
    }
}

/// A line read without echo, for a token pasted into a terminal.
pub fn read_hidden(prompt: &str) -> Result<String> {
    use std::io::{BufRead, IsTerminal, Write};
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let stdin = std::io::stdin();
    #[cfg(unix)]
    let restore = if stdin.is_terminal() {
        use std::os::fd::AsRawFd;
        let fd = stdin.as_raw_fd();
        let mut term: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut term) } == 0 {
            let saved = term;
            term.c_lflag &= !libc::ECHO;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &term) };
            Some((fd, saved))
        } else {
            None
        }
    } else {
        None
    };
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    #[cfg(unix)]
    if let Some((fd, saved)) = restore {
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
        eprintln!();
    }
    read?;
    Ok(line.trim().to_owned())
}
