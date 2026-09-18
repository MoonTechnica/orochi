//! Read-only CLI quota probes. Credentials stay with the installed CLI.
use crate::{
    config::{Config, QuotaProbe, QuotaProbeKind},
    storage::Store,
    types::*,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub bucket: String,
    pub remaining: f64,
    pub reset_at: Option<i64>,
    pub model: Option<String>,
    /// Unknown Codex bucket-to-model mappings are displayed but never guessed.
    pub affects_routing: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub agent: String,
    pub source: String,
    pub observed_at: i64,
    pub valid_until: i64,
    pub windows: Vec<Window>,
}
#[derive(Debug, Serialize)]
pub struct ProbeResult {
    pub agent: String,
    pub status: &'static str,
    pub detail: Option<String>,
    pub snapshot: Option<Snapshot>,
}

pub fn parse_codex(value: &Value, time: i64) -> Result<Vec<Window>> {
    let mut windows = vec![];
    let buckets: BTreeMap<String, Value> = if let Some(map) = value["rateLimitsByLimitId"]
        .as_object()
        .filter(|m| !m.is_empty())
    {
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    } else if value["rateLimits"].is_object() {
        let bucket = value["rateLimits"]["limitId"].as_str().unwrap_or("codex");
        BTreeMap::from([(bucket.into(), value["rateLimits"].clone())])
    } else {
        BTreeMap::new()
    };
    for (bucket, limits) in buckets {
        for period in ["primary", "secondary"] {
            let v = &limits[period];
            let Some(used) = v["usedPercent"].as_f64() else {
                continue;
            };
            ensure!(
                used.is_finite() && (0.0..=100.0).contains(&used),
                "invalid quota percentage"
            );
            let reset_at = v["resetsAt"].as_i64();
            if reset_at.is_some_and(|t| t <= time) {
                continue;
            }
            windows.push(Window {
                bucket: format!("{bucket}/{period}"),
                remaining: 1.0 - used / 100.0,
                reset_at,
                model: None,
                affects_routing: bucket == "codex",
            });
        }
    }
    Ok(windows)
}

/// Official Claude Code statusline payload; context-window percentages are unrelated to quota.
pub fn parse_claude_statusline(value: &Value, time: i64) -> Result<Vec<Window>> {
    let mut windows = vec![];
    for bucket in ["five_hour", "seven_day", "spend_limit"] {
        let v = &value["rate_limits"][bucket];
        let Some(used) = v["used_percentage"].as_f64() else {
            continue;
        };
        ensure!(
            used.is_finite() && used >= 0.0 && (bucket == "spend_limit" || used <= 100.0),
            "invalid Claude quota percentage"
        );
        let reset_at = v["resets_at"].as_i64();
        if reset_at.is_some_and(|t| t <= time) {
            continue;
        }
        windows.push(Window {
            bucket: bucket.into(),
            remaining: (1.0 - used / 100.0).clamp(0.0, 1.0),
            reset_at,
            model: None,
            affects_routing: true,
        });
    }
    Ok(windows)
}

pub fn apply(map: &mut RuntimeMap, snapshot: &Snapshot, time: i64) {
    if snapshot.valid_until <= time || snapshot.observed_at > time {
        return;
    }
    let mut groups: BTreeMap<String, Vec<&Window>> = BTreeMap::new();
    for window in snapshot
        .windows
        .iter()
        .filter(|w| w.affects_routing && w.reset_at.is_none_or(|t| t > time))
    {
        groups
            .entry(window.model.clone().unwrap_or("*".into()))
            .or_default()
            .push(window);
    }
    for (model, windows) in groups {
        let limiting = windows
            .iter()
            .min_by(|a, b| a.remaining.total_cmp(&b.remaining))
            .unwrap();
        let state = map
            .entry((snapshot.agent.clone(), model.clone()))
            .or_insert_with(|| RuntimeState::new(&snapshot.agent, &model));
        let exhausted: Vec<_> = windows.iter().filter(|w| w.remaining <= 0.0).collect();
        let reset_at = if exhausted.is_empty() {
            limiting.reset_at
        } else {
            Some(
                exhausted
                    .iter()
                    .map(|w| w.reset_at.unwrap_or(snapshot.valid_until))
                    .max()
                    .unwrap()
                    .min(snapshot.valid_until),
            )
        };
        // Preserve live error cooldowns. Source expiry never clears a separately observed rate limit.
        let error_cooldown = state.cooldown_until.filter(|t| *t > time);
        crate::scheduler::quota::observe(
            state,
            &crate::agents::QuotaObservation {
                remaining: limiting.remaining,
                reset_at,
                model: Some(model),
            },
            time,
        );
        if let Some(until) = error_cooldown {
            state.cooldown_until = Some(state.cooldown_until.unwrap_or(until).max(until));
            state.status = RuntimeStatus::Cooldown;
        }
    }
}

struct ProcessGuard(Option<u32>);
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0 {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
}

async fn probe(probe: &QuotaProbe, config: &Config, time: i64) -> Result<Snapshot> {
    let agent = config
        .agents
        .iter()
        .find(|a| a.id == probe.agent)
        .context("unknown quota agent")?;
    let directory = tempfile::tempdir()?;
    let terminal = matches!(
        probe.kind,
        QuotaProbeKind::ClaudeUsage | QuotaProbeKind::AntigravityUsage
    );
    let mut command = if terminal {
        let mut command = tokio::process::Command::new("python3");
        command.args([
            "-c",
            include_str!("quota_terminal.py"),
            if matches!(probe.kind, QuotaProbeKind::ClaudeUsage) {
                "claude_usage"
            } else {
                "antigravity_usage"
            },
            &config
                .quota
                .timeout_secs
                .saturating_sub(1)
                .max(1)
                .to_string(),
            &probe.command,
        ]);
        command
    } else {
        tokio::process::Command::new(&probe.command)
    };
    command
        .args(&probe.args)
        .envs(&agent.env)
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if matches!(probe.kind, QuotaProbeKind::CodexAppServer) {
        command.arg("app-server");
    }
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().context("quota CLI could not be started")?;
    let _guard = ProcessGuard(child.id());
    let output = child.stdout.take().context("quota stdout unavailable")?;
    let work = async {
        let value: Value = match probe.kind {
            QuotaProbeKind::CodexAppServer => {
                let mut input = child.stdin.take().context("quota stdin unavailable")?;
                input.write_all(format!("{}\n", json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"orochi-quota","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}})).as_bytes()).await?;
                let mut lines = BufReader::new(output.take(1_048_577)).lines();
                let mut received = 0;
                let mut initialized = false;
                loop {
                    let line = lines
                        .next_line()
                        .await?
                        .context("quota CLI closed the connection")?;
                    received += line.len();
                    ensure!(received <= 1_048_576, "quota output too large");
                    let message: Value =
                        serde_json::from_str(&line).context("invalid quota RPC response")?;
                    if message["id"] == 1 && !initialized {
                        ensure!(
                            message.get("error").is_none(),
                            "quota CLI initialization failed"
                        );
                        initialized = true;
                        input.write_all(b"{\"method\":\"initialized\",\"params\":{}}\n{\"id\":2,\"method\":\"account/rateLimits/read\"}\n").await?;
                    } else if message["id"] == 2 && initialized {
                        ensure!(
                            message.get("error").is_none(),
                            "quota request failed; check CLI login and account support"
                        );
                        break message["result"].clone();
                    }
                }
            }
            QuotaProbeKind::Command
            | QuotaProbeKind::ClaudeUsage
            | QuotaProbeKind::AntigravityUsage => {
                drop(child.stdin.take());
                let mut bytes = Vec::new();
                output.take(1_048_577).read_to_end(&mut bytes).await?;
                ensure!(bytes.len() <= 1_048_576, "quota output too large");
                ensure!(child.wait().await?.success(), "quota command failed");
                serde_json::from_slice(&bytes).context("invalid quota command JSON")?
            }
        };
        let windows = match probe.kind {
            QuotaProbeKind::CodexAppServer => parse_codex(&value, time)?,
            QuotaProbeKind::Command
            | QuotaProbeKind::ClaudeUsage
            | QuotaProbeKind::AntigravityUsage => {
                ensure!(
                    value["schema_version"] == 1,
                    "quota command requires schema_version=1"
                );
                serde_json::from_value::<Vec<Window>>(value["windows"].clone())?
            }
        };
        ensure!(windows.len() <= 128, "too many quota windows");
        let mut ids = std::collections::BTreeSet::new();
        for w in &windows {
            ensure!(
                w.remaining.is_finite()
                    && (0.0..=1.0).contains(&w.remaining)
                    && !w.bucket.is_empty()
                    && ids.insert((&w.bucket, &w.model))
                    && w.reset_at.is_none_or(|t| t > time)
                    && w.model.as_ref().is_none_or(|s| !s.is_empty() && s != "*"),
                "invalid quota window"
            );
        }
        Ok::<_, anyhow::Error>(Snapshot {
            agent: probe.agent.clone(),
            source: match probe.kind {
                QuotaProbeKind::CodexAppServer => "codex_app_server",
                QuotaProbeKind::Command => "command",
                QuotaProbeKind::ClaudeUsage => "claude_usage",
                QuotaProbeKind::AntigravityUsage => "antigravity_usage",
            }
            .into(),
            observed_at: time,
            valid_until: time.saturating_add(config.quota.max_age_secs),
            windows,
        })
    };
    let result = tokio::time::timeout(Duration::from_secs(config.quota.timeout_secs), work).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result.context("quota probe timed out")?
}

pub async fn refresh(config: &Config, store: &Store) -> Result<Vec<ProbeResult>> {
    let mut results = vec![];
    for agent in config.agents.iter().filter(|a| a.enabled) {
        let auto = if agent.id == "codex" && agent.provider == Provider::Openai {
            crate::discovery::inspect(agent, &config.discovery, store.data_dir())
                .native_cli
                .map(|path| QuotaProbe {
                    agent: agent.id.clone(),
                    kind: QuotaProbeKind::CodexAppServer,
                    command: path.to_string_lossy().into_owned(),
                    args: vec![],
                })
        } else if agent.id == "claude" && agent.provider == Provider::Anthropic {
            crate::discovery::inspect(agent, &config.discovery, store.data_dir())
                .native_cli
                .map(|path| QuotaProbe {
                    agent: agent.id.clone(),
                    kind: QuotaProbeKind::ClaudeUsage,
                    command: path.to_string_lossy().into_owned(),
                    args: vec![
                        "--tools".into(),
                        "".into(),
                        "--strict-mcp-config".into(),
                        "--mcp-config".into(),
                        "{\"mcpServers\":{}}".into(),
                    ],
                })
        } else if agent.id == "antigravity" && agent.provider == Provider::Google {
            let native = crate::config::AgentConfig {
                command: "agy".into(),
                args: vec![],
                ..agent.clone()
            };
            crate::discovery::inspect(&native, &config.discovery, store.data_dir())
                .executable
                .map(|path| QuotaProbe {
                    agent: agent.id.clone(),
                    kind: QuotaProbeKind::AntigravityUsage,
                    command: path.to_string_lossy().into_owned(),
                    args: vec![],
                })
        } else {
            None
        };
        let Some(p) = config
            .quota
            .probes
            .iter()
            .find(|p| p.agent == agent.id)
            .or(auto.as_ref())
        else {
            results.push(ProbeResult {
                agent: agent.id.clone(),
                status: "unsupported",
                detail: Some(
                    "No configured direct quota source; ACP runtime observations remain available."
                        .into(),
                ),
                snapshot: None,
            });
            continue;
        };
        match probe(p, config, now()).await {
            Ok(snapshot) => {
                if !snapshot.windows.is_empty() {
                    store.save_quota_snapshot(&snapshot)?;
                }
                results.push(ProbeResult {
                    agent: agent.id.clone(),
                    status: if snapshot.windows.is_empty() {
                        "unknown"
                    } else {
                        "fresh"
                    },
                    detail: None,
                    snapshot: Some(snapshot),
                });
            }
            Err(error) => results.push(ProbeResult {
                agent: agent.id.clone(),
                status: "unavailable",
                detail: Some(error.to_string()),
                snapshot: None,
            }),
        }
    }
    Ok(results)
}
