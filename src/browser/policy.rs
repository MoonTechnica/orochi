//! Where an agent may take the browser (`docs/conversation-tabs-design.md` §3.5.6).
//!
//! `deny` wins over `allow`; a pattern is a host or `*.host` for its subdomains (the apex is
//! its own entry, as VS Code's lists have it); only `http` and `https` are ever navigated to,
//! and `about:blank` is the empty page. Anything else — `file:`, `chrome:`, `javascript:`,
//! `data:` — is refused whoever asks, because it is how a page would read the disk or run as
//! something it is not.
use crate::config::BrowserConfig;
use anyhow::{Result, ensure};
use reqwest::Url;

#[derive(Debug, Clone)]
pub struct Policy {
    allow: Vec<String>,
    deny: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Scheme(String),
    Denied(String),
    NotAllowed(String),
    Unreadable,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Scheme(s) => write!(f, "{s}: addresses are not opened by the browser tools"),
            Self::Denied(h) => write!(f, "{h} is in browser.deny"),
            Self::NotAllowed(h) => write!(
                f,
                "{h} is not in browser.allow; the person can add it to the configuration"
            ),
            Self::Unreadable => write!(f, "that is not an address the browser can open"),
        }
    }
}

fn matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(rest) => host.len() > rest.len() + 1 && host.ends_with(&format!(".{rest}")),
        None => pattern == host,
    }
}

impl Policy {
    pub fn new(config: &BrowserConfig) -> Self {
        let lower = |list: &[String]| list.iter().map(|p| p.trim().to_ascii_lowercase()).collect();
        Self {
            allow: lower(&config.allow),
            deny: lower(&config.deny),
        }
    }

    /// `Ok` when the address may be opened or stayed on.
    pub fn check(&self, address: &str) -> Result<(), Refusal> {
        if address == "about:blank" {
            return Ok(());
        }
        let url = Url::parse(address).map_err(|_| Refusal::Unreadable)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Refusal::Scheme(url.scheme().to_owned()));
        }
        let host = url
            .host_str()
            .ok_or(Refusal::Unreadable)?
            .to_ascii_lowercase();
        if self.deny.iter().any(|p| matches(p, &host)) {
            return Err(Refusal::Denied(host));
        }
        if self.allow.iter().any(|p| matches(p, &host)) {
            Ok(())
        } else {
            Err(Refusal::NotAllowed(host))
        }
    }
}

/// `Config::validate`: a pattern is a plain host name, `*.` and a name, or a bracketed IPv6.
pub fn check(config: &BrowserConfig) -> Result<()> {
    for (list, name) in [(&config.allow, "allow"), (&config.deny, "deny")] {
        for pattern in list {
            let body = pattern.strip_prefix("*.").unwrap_or(pattern);
            ensure!(
                !body.is_empty()
                    && body.len() <= 253
                    && body
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-._[]:".contains(c)),
                "browser.{name}: {pattern:?} is not a host name or a *.host pattern"
            );
        }
    }
    if let Some(chrome) = &config.chrome {
        ensure!(
            chrome.is_absolute(),
            "browser.chrome must be an absolute path"
        );
    }
    ensure!(
        config.idle_secs >= 30,
        "browser.idle_secs must be at least 30"
    );
    Ok(())
}
