//! Provider-only observations live here; transport and session logic live in `acp`.
//! Nonstandard quota support is opt-in under `_meta["orochi.dev/quota"]`.
use crate::types::{Provider, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    RateLimit,
    Authentication,
    Unavailable,
    Configuration,
    Timeout,
    Cancelled,
    Other,
}
impl ErrorKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::Authentication => "authentication",
            Self::Unavailable => "unavailable",
            Self::Configuration => "configuration",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Other => "other",
        }
    }
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct AgentError {
    pub kind: ErrorKind,
    pub message: String,
    pub reset_at: Option<i64>,
    pub model_scoped: bool,
}
impl AgentError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            reset_at: None,
            model_scoped: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaObservation {
    pub remaining: f64,
    pub reset_at: Option<i64>,
    pub model: Option<String>,
}

pub trait AgentAdapter {
    fn classify_error(&self, error: &Value, fallback: &str, time: i64) -> AgentError;
    fn get_usage(&self, payload: &Value) -> Option<Usage>;
    fn get_quota(&self, payload: &Value) -> Option<QuotaObservation>;
}
pub struct ProviderAdapter(pub Provider);

/// Next local occurrence of "try again at 9:11 PM", as printed by Codex usage limits.
fn retry_clock(text: &str, time: i64) -> Option<i64> {
    let rest = &text[text.find("try again at ")? + "try again at ".len()..];
    let (clock, suffix) = rest.split_once(' ').unwrap_or((rest, ""));
    let (hour, minute) = clock.split_once(':')?;
    let mut hour: i32 = hour.parse().ok()?;
    let minute: i32 = minute
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()?;
    match suffix.get(..2) {
        Some("pm") if (1..12).contains(&hour) => hour += 12,
        Some("am") if hour == 12 => hour = 0,
        _ => {}
    }
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    local_clock(time, hour, minute)
}

#[cfg(unix)]
fn local_clock(time: i64, hour: i32, minute: i32) -> Option<i64> {
    let now = time as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return None;
    }
    tm.tm_hour = hour;
    tm.tm_min = minute;
    tm.tm_sec = 0;
    tm.tm_isdst = -1;
    let at = unsafe { libc::mktime(&mut tm) } as i64;
    (at > 0).then(|| if at <= time { at + 86_400 } else { at })
}
#[cfg(not(unix))]
fn local_clock(_: i64, _: i32, _: i32) -> Option<i64> {
    None
}

fn number(v: &Value, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|k| v[*k].as_u64())
}
impl AgentAdapter for ProviderAdapter {
    fn classify_error(&self, error: &Value, fallback: &str, time: i64) -> AgentError {
        let message = error["message"].as_str().unwrap_or(fallback);
        // Bridges often wrap provider failures in a generic -32603 whose data carries the cause.
        let text = format!(
            "{} {} {} {} {} {}",
            message,
            error["data"]["type"],
            error["data"]["code"],
            error["data"]["codexErrorInfo"],
            crate::context::bounded(error["data"]["message"].as_str().unwrap_or(""), 4000),
            crate::context::bounded(error["data"]["details"].as_str().unwrap_or(""), 4000)
        )
        .to_lowercase();
        let code = error["code"].as_i64();
        let provider_limit = match self.0 {
            Provider::Openai => text.contains("usage limit") || text.contains("insufficient_quota"),
            Provider::Anthropic => {
                text.contains("overloaded_error") || text.contains("usage limit")
            }
            Provider::Google => {
                text.contains("resource_exhausted") || text.contains("quota exceeded")
            }
        };
        let kind = if matches!(code, Some(402 | 429))
            || text.contains("rate_limit")
            || text.contains("rate limit")
            || text.contains("insufficient_quota")
            || text.contains("insufficient credits")
            || text.contains("credit balance is too low")
            || text.contains("quota exhausted")
            || text.contains("usagelimitexceeded")
            || provider_limit
        {
            ErrorKind::RateLimit
        } else if matches!(code, Some(401 | 403 | -32000))
            && (code != Some(-32000) || text.contains("auth"))
            || text.contains("authentication")
            || text.contains("not logged in")
            || text.contains("login required")
        {
            ErrorKind::Authentication
        } else if matches!(code, Some(-32601 | -32602)) {
            ErrorKind::Configuration
        } else if text.contains("connection closed")
            || text.contains("broken pipe")
            || text.contains("failed to spawn")
        {
            ErrorKind::Unavailable
        } else {
            ErrorKind::Other
        };
        let data = &error["data"];
        // A classified failure says what to do about it, and the nested detail it was read
        // from can carry provider request IDs, so it stays hidden. An unclassified one says
        // nothing at all — "Internal error" is not something a user can act on — so there the
        // cause is worth showing, on one line, shortened, and stripped of control characters.
        // Only `kind` is ever persisted; this reaches stderr and the screen and nothing else.
        let detail = (kind == ErrorKind::Other)
            .then(|| {
                ["details", "message", "type"]
                    .iter()
                    .find_map(|key| data[*key].as_str())
            })
            .flatten()
            .map(|detail| {
                // Shown the way mailbox bodies are: an escape is made visible and inert
                // rather than dropped, which would leave its parameters as stray text.
                detail
                    .replace('\u{1b}', "\\x1b")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect::<String>()
            })
            .filter(|detail| !detail.is_empty() && !message.contains(detail.as_str()));
        let message = match &detail {
            Some(detail) => &format!("{message}: {detail}"),
            None => message,
        };
        let reset_at = number(data, &["reset_at", "resetAt", "resetsAt"])
            .and_then(|n| i64::try_from(n).ok())
            .or_else(|| {
                number(data, &["retry_after", "retryAfter"])
                    .and_then(|n| i64::try_from(n).ok())
                    .map(|n| time.saturating_add(n))
            })
            .or_else(|| retry_clock(&text, time))
            .filter(|t| *t > time);
        AgentError {
            kind,
            message: crate::context::bounded(message, 1000),
            reset_at,
            model_scoped: data["scope"].as_str() == Some("model"),
        }
    }
    fn get_usage(&self, payload: &Value) -> Option<Usage> {
        let v = payload
            .get("usage")
            .or_else(|| payload.pointer("/_meta/orochi.dev~1usage"))?;
        let mut input = number(v, &["inputTokens", "input_tokens", "prompt_tokens"]);
        let output = number(v, &["outputTokens", "output_tokens", "completion_tokens"]);
        let cached = number(
            v,
            &[
                "cachedReadTokens",
                "cached_tokens",
                "cache_read_input_tokens",
            ],
        )
        .or_else(|| {
            v.pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
        })
        .or_else(|| {
            v.pointer("/prompt_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
        });
        // Anthropic native input_tokens excludes cache creation/read input.
        if self.0 == Provider::Anthropic
            && (v.get("cache_read_input_tokens").is_some()
                || v.get("cache_creation_input_tokens").is_some())
        {
            input = input.map(|n| {
                n.saturating_add(cached.unwrap_or(0))
                    .saturating_add(number(v, &["cache_creation_input_tokens"]).unwrap_or(0))
            });
        }
        Some(Usage {
            total_tokens: number(v, &["totalTokens", "total_tokens"]),
            input_tokens: input,
            output_tokens: output,
            reasoning_tokens: number(v, &["thoughtTokens", "reasoningTokens", "reasoning_tokens"])
                .or_else(|| {
                    v.pointer("/output_tokens_details/reasoning_tokens")
                        .and_then(Value::as_u64)
                })
                .or_else(|| {
                    v.pointer("/completion_tokens_details/reasoning_tokens")
                        .and_then(Value::as_u64)
                }),
            cached_tokens: cached,
        })
    }
    fn get_quota(&self, payload: &Value) -> Option<QuotaObservation> {
        let quota: QuotaObservation =
            serde_json::from_value(payload.pointer("/_meta/orochi.dev~1quota")?.clone()).ok()?;
        (quota.remaining.is_finite() && (0.0..=1.0).contains(&quota.remaining)).then_some(quota)
    }
}
