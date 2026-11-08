//! Choosing the decision model: on the first interactive run, with `orochi decision setup`,
//! and with `orochi decision use`. A local choice downloads at once, a cloud choice takes
//! the key at once, and either is tried with one question before it is saved, so what is
//! written down is known to work.
use super::{MODELS, Model, key, local, model, recommend};
use crate::config::{Config, DecisionBackend};
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, IsTerminal, Write},
    path::Path,
};

/// Where downloads come from; tests point both at a local server.
#[derive(Debug, Clone)]
pub struct Sources {
    pub llama_releases: String,
    pub gguf: Option<String>,
}
impl Sources {
    pub fn from_env() -> Self {
        Self {
            llama_releases: std::env::var("OROCHI_LLAMA_RELEASES")
                .unwrap_or_else(|_| local::LLAMA_RELEASES.to_owned()),
            gguf: std::env::var("OROCHI_DECISION_GGUF").ok(),
        }
    }
}

/// Puts the choice into effect and saves it: downloads what a local model needs, stores a
/// cloud key, asks one question either way, and only then writes `config` to `path`.
pub async fn adopt(
    config: &mut Config,
    path: &Path,
    data: &Path,
    backend: DecisionBackend,
    name: &str,
    api_key: Option<&str>,
    sources: &Sources,
) -> Result<()> {
    let mut decision = config.decision.clone();
    decision.backend = Some(backend);
    decision.model = name.to_owned();
    match backend {
        DecisionBackend::Off => {}
        DecisionBackend::Local => {
            if decision.endpoint.is_none() {
                let known = model(name)
                    .filter(|m| m.gguf.is_some())
                    .with_context(|| format!("{name} does not run on this machine"))?;
                let home = local::home(data, &decision);
                if !local::installed(&home, known) {
                    download(&home, known, sources).await?;
                }
            }
            eprintln!("  starting {name} and asking it one question…");
            let took = super::check(&decision, data).await?;
            eprintln!("  answered in {:.1} s", took.as_secs_f64());
        }
        DecisionBackend::Cloud => {
            let (provider, _) = super::cloud_of(&decision)?;
            let name_of = provider.map(|p| p.name).unwrap_or("custom");
            if let Some(api_key) = api_key {
                key::put(data, &decision, name_of, api_key)?;
            }
            if key::get(data, &decision, name_of).is_none() {
                bail!("no API key for {name_of}; run `orochi decision key {name_of}`");
            }
            eprintln!("  asking {name} one question…");
            let took = super::check(&decision, data).await?;
            eprintln!("  answered in {:.1} s", took.as_secs_f64());
        }
    }
    config.decision = decision;
    config.save(path)?;
    Ok(())
}

/// `home` is the directory a local model goes in (`local::home`).
pub async fn download(home: &Path, model: &Model, sources: &Sources) -> Result<()> {
    let terminal = std::io::stderr().is_terminal();
    let mut last = String::new();
    let result = local::install(
        home,
        model,
        &sources.llama_releases,
        sources.gguf.as_deref(),
        |d| {
            if d.state != "running" {
                return;
            }
            let line = format!(
                "  downloading {}  {:.2} / {:.2} GB{}",
                d.file,
                d.done as f64 / 1e9,
                d.total as f64 / 1e9,
                (d.done * 100)
                    .checked_div(d.total)
                    .map(|p| format!("  {p}%"))
                    .unwrap_or_default()
            );
            if terminal {
                eprint!("\r{line}\x1b[K");
            } else if line.split("  ").nth(1) != last.split("  ").nth(1) {
                eprintln!("{line}");
            }
            last = line;
        },
    )
    .await;
    if terminal && !last.is_empty() {
        eprintln!();
    }
    result
}

fn line(prompt: &str) -> Result<Option<String>> {
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer)? == 0 {
        return Ok(None);
    }
    Ok(Some(answer.trim().to_owned()))
}

/// Reads a secret from the terminal without echoing it, or a line from a pipe.
pub fn secret(prompt: &str) -> Result<Option<String>> {
    #[cfg(unix)]
    if std::io::stdin().is_terminal() {
        use std::os::fd::AsRawFd;
        let fd = std::io::stdin().as_raw_fd();
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } == 0 {
            let mut quiet = saved;
            quiet.c_lflag &= !libc::ECHO;
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) };
            let answer = line(prompt);
            unsafe { libc::tcsetattr(fd, libc::TCSANOW, &saved) };
            eprintln!();
            return answer;
        }
    }
    line(prompt)
}

fn size(m: &Model) -> String {
    m.gguf
        .map(|g| {
            if g.bytes >= 1_000_000_000 {
                format!("{:.1} GB", g.bytes as f64 / 1e9)
            } else {
                format!("{} MB", g.bytes / 1_000_000)
            }
        })
        .unwrap_or_default()
}

fn measured(m: &Model) -> String {
    match m.measured {
        Some((right, of)) => format!("kind of work right {right}/{of} in Orochi's measurement"),
        None => "not measured here".into(),
    }
}

/// Asks for a number between 1 and `count`; `Ok(None)` at the end of input.
fn pick(count: usize, default: usize) -> Result<Option<usize>> {
    loop {
        let Some(answer) = line(&format!("Choose [{}]: ", default + 1))? else {
            return Ok(None);
        };
        if answer.is_empty() {
            return Ok(Some(default));
        }
        if let Ok(n) = answer.parse::<usize>()
            && (1..=count).contains(&n)
        {
            return Ok(Some(n - 1));
        }
    }
}

/// The first-run question, in two steps: where it runs (this machine, the cloud, or not at
/// all), then which model there, the recommended one first. A cloud model takes its
/// provider's key. `Ok(false)` when it was left unanswered (end of input), so it is asked
/// again next time; choosing none saves `off` and is not asked again.
pub async fn interactive(config: &mut Config, path: &Path, data: &Path) -> Result<bool> {
    let advice = recommend(&local::home(data, &config.decision));
    let sources = Sources::from_env();
    eprintln!(
        "\nOrochi reads each request with a decision model to choose who does it: what kind of work it is and how hard."
    );
    loop {
        let mut places = vec![
            (
                DecisionBackend::Local,
                "on this machine: the model is downloaded now; requests never leave this machine"
                    .to_owned(),
            ),
            (
                DecisionBackend::Cloud,
                "in the cloud: needs the provider's API key; each request's text is sent to it"
                    .to_owned(),
            ),
        ];
        if !advice.local_supported {
            places.remove(0);
        }
        // Only this machine is ever recommended; one that cannot run it starts on the cloud,
        // recommending nothing.
        let default = match advice.backend {
            Some(DecisionBackend::Local) => {
                places[0]
                    .1
                    .push_str(&format!("  (recommended: {})", advice.reason));
                0
            }
            _ => places
                .iter()
                .position(|(b, _)| *b == DecisionBackend::Cloud)
                .unwrap_or(0),
        };
        places.push((DecisionBackend::Off, "none: route by keywords alone".into()));
        eprintln!("Where should it run?");
        for (i, (_, text)) in places.iter().enumerate() {
            eprintln!("  {}) {text}", i + 1);
        }
        let Some(place) = pick(places.len(), default)? else {
            return Ok(false);
        };
        let backend = places[place].0;
        let choice: Option<(&Model, Option<String>)> = match backend {
            DecisionBackend::Off => None,
            DecisionBackend::Local => {
                let models: Vec<&Model> = MODELS.iter().filter(|m| m.gguf.is_some()).collect();
                eprintln!("Which model?");
                for (i, m) in models.iter().enumerate() {
                    eprintln!(
                        "  {}) {:<18} {}, {} · {}{}",
                        i + 1,
                        m.name,
                        size(m),
                        m.license,
                        measured(m),
                        if m.name == advice.model {
                            "  (recommended)"
                        } else {
                            ""
                        }
                    );
                }
                let Some(n) = pick(models.len(), 0)? else {
                    return Ok(false);
                };
                Some((models[n], None))
            }
            DecisionBackend::Cloud => {
                let models: Vec<&Model> = MODELS.iter().filter(|m| m.cloud.is_some()).collect();
                eprintln!("Which model?");
                for (i, m) in models.iter().enumerate() {
                    let via = m.cloud.and_then(|c| super::provider(c.provider));
                    eprintln!(
                        "  {}) {:<18} on {} · {}",
                        i + 1,
                        m.name,
                        via.map(|p| p.label).unwrap_or("?"),
                        measured(m),
                    );
                }
                let Some(n) = pick(models.len(), 0)? else {
                    return Ok(false);
                };
                let m = models[n];
                let via = m
                    .cloud
                    .and_then(|c| super::provider(c.provider))
                    .expect("listed");
                let mut probe = config.decision.clone();
                probe.model = m.name.into();
                let key = if key::get(data, &probe, via.name).is_none() {
                    eprintln!("  A {} key is made at {}", via.label, via.key_url);
                    match secret(&format!("  {} API key: ", via.label))? {
                        Some(k) if !k.is_empty() => Some(k),
                        _ => continue,
                    }
                } else {
                    None
                };
                Some((m, key))
            }
        };
        let (name, api_key) = match choice {
            Some((m, key)) => (m.name.to_owned(), key),
            None => (config.decision.model.clone(), None),
        };
        match adopt(
            config,
            path,
            data,
            backend,
            &name,
            api_key.as_deref(),
            &sources,
        )
        .await
        {
            Ok(()) => {
                eprintln!(
                    "Saved to {}. Change it with `orochi decision setup`.\n",
                    path.display()
                );
                return Ok(true);
            }
            Err(error) => {
                eprintln!("  that did not work: {error:#}");
                eprintln!("  choose again, or none to go without one");
            }
        }
    }
}
