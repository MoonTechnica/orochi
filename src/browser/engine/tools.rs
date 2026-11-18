//! The agents' `browser_*` tools on a Servo tab (`docs/servo-browser-design.md` §3.4).
//!
//! The engine can run a script and take a picture; everything else is built here from those
//! and the input events the person's tab already uses. A ref is an element a snapshot held,
//! kept in the page under a property that is not enumerable (the page could still read it,
//! as it can read anything in its own window). What Chrome gives and this engine does not —
//! file choosers, the network log, several tabs, an awaited promise — answers with that said.
use super::{Engines, Tab};
use crate::browser::broker::{
    NOTICE, Request, SNAPSHOT_CHARS, definitions, failure, save_picture, text,
};
use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::time::Instant;

const SNAPSHOT: &str = include_str!("snapshot.js");
const CLIP: &str = include_str!("clip.js");
const FOCUS: &str = include_str!("focus.js");

fn script_for(template: &str, reference: &str) -> String {
    template.replace(
        "__REF__",
        &serde_json::to_string(reference).expect("a string serializes"),
    )
}

fn parse(value: &Value) -> Result<Value> {
    let text = value.as_str().context("the page answered with no text")?;
    let parsed: Value = serde_json::from_str(text).context("the page answered with nonsense")?;
    if let Some(error) = parsed["error"].as_str() {
        bail!("{error}");
    }
    Ok(parsed)
}

fn modifiers_and_key(spec: &str) -> (i64, String) {
    let mut bits = 0;
    let mut parts: Vec<&str> = spec.split('+').collect();
    let key = parts.pop().unwrap_or("").to_owned();
    for part in parts {
        bits |= match part.to_ascii_lowercase().as_str() {
            "alt" | "option" => 1,
            "control" | "ctrl" => 2,
            "meta" | "command" | "cmd" => 4,
            "shift" => 8,
            _ => 0,
        };
    }
    (bits, key)
}

impl Engines {
    /// One agent tool call on this scope's engine.
    pub async fn agent(&self, request: &Request) -> Value {
        let known = definitions(&self.config, request.read_only);
        let allowed = known
            .as_array()
            .is_some_and(|tools| tools.iter().any(|t| t["name"] == request.tool));
        if !allowed {
            return failure(format!("{} is not available to this session", request.tool));
        }
        match self.run(request).await {
            Ok(value) => value,
            Err(error) => failure(format!("{error:#}")),
        }
    }

    /// The tab has strayed from where the policy lets it be: it goes to a blank page, and the
    /// agent is told.
    async fn guard(&self, tab: &Tab) -> Result<()> {
        let url = tab.state.lock().unwrap().url.clone();
        if url.is_empty() {
            return Ok(());
        }
        if let Err(refusal) = self.policy.check(&url) {
            tab.send(&json!({"cmd": "navigate", "url": "about:blank"}))
                .await?;
            bail!(
                "the page went to {url}, which is not allowed ({refusal}); the tab was sent to \
                 about:blank. Navigate to an allowed address."
            );
        }
        Ok(())
    }

    /// Waits for a load that has begun to end, then looks where the page is.
    async fn settle(&self, tab: &Tab, limit: Duration) -> Result<()> {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let end = Instant::now() + limit;
        while tab.state.lock().unwrap().loading && Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.guard(tab).await
    }

    async fn snapshot(&self, tab: &Tab) -> Result<String> {
        let script = SNAPSHOT.replace("__LIMIT__", &SNAPSHOT_CHARS.to_string());
        let parsed = parse(&tab.eval(&script).await?)?;
        Ok(parsed["text"].as_str().unwrap_or("").to_owned())
    }

    /// Where an element is, scrolled into view: a rectangle in CSS pixels.
    async fn clip(&self, tab: &Tab, reference: &str) -> Result<[f64; 4]> {
        anyhow::ensure!(
            reference.len() <= 12 && reference.starts_with('e'),
            "{reference} is not a ref from a snapshot"
        );
        let parsed = parse(&tab.eval(&script_for(CLIP, reference)).await?)?;
        let number = |key: &str| parsed[key].as_f64().unwrap_or(0.0);
        Ok([number("x"), number("y"), number("width"), number("height")])
    }

    async fn mouse(&self, tab: &Tab, kind: &str, x: f64, y: f64) -> Result<()> {
        tab.send(
            &json!({"cmd": "input", "type": "mouse", "kind": kind, "x": x, "y": y,
                         "button": "left", "clickCount": 1}),
        )
        .await
    }

    async fn press(&self, tab: &Tab, spec: &str) -> Result<()> {
        let (modifiers, key) = modifiers_and_key(spec);
        anyhow::ensure!(!key.is_empty(), "no key was named");
        if key.chars().count() == 1 && modifiers == 0 {
            return tab
                .send(&json!({"cmd": "input", "type": "text", "text": key}))
                .await;
        }
        let key = if key == "Space" { " ".to_owned() } else { key };
        let code = match key.as_str() {
            " " => "Space".to_owned(),
            k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => {
                format!("Key{}", k.to_ascii_uppercase())
            }
            k => k.to_owned(),
        };
        for kind in ["keyDown", "keyUp"] {
            tab.send(
                &json!({"cmd": "input", "type": "key", "kind": kind, "key": key,
                             "code": code, "modifiers": modifiers}),
            )
            .await?;
        }
        Ok(())
    }

    async fn run(&self, request: &Request) -> Result<Value> {
        let args = &request.args;
        let tool = request.tool.as_str();
        let string = |key: &str| args[key].as_str().map(str::to_owned);
        let need = |key: &str| string(key).with_context(|| format!("{key} is needed"));
        let tab: Arc<Tab> = self.ensure(&request.scope).await?;
        if tool != "browser_navigate" && tool != "browser_tabs" {
            self.guard(&tab).await?;
        }
        match tool {
            "browser_tabs" => match args["action"].as_str().unwrap_or("list") {
                "list" => {
                    let state = tab.state.lock().unwrap();
                    Ok(text(format!(
                        "0: {} — {}  (current)",
                        if state.title.is_empty() {
                            "(untitled)"
                        } else {
                            &state.title
                        },
                        if state.url.is_empty() {
                            "about:blank"
                        } else {
                            &state.url
                        }
                    )))
                }
                _ => bail!(
                    "this session's browser has one tab; use browser_navigate (or set \
                     browser.engine to \"chromium\" for several tabs)"
                ),
            },
            "browser_navigate" => {
                let url = need("url")?;
                self.policy
                    .check(&url)
                    .map_err(|r| anyhow::anyhow!("{r}"))?;
                {
                    let mut state = tab.state.lock().unwrap();
                    state.refused = None;
                    state.loading = true;
                }
                tab.send(&json!({"cmd": "navigate", "url": url})).await?;
                self.settle(&tab, Duration::from_secs(15)).await?;
                let state = tab.state.lock().unwrap();
                if state.refused.is_some() {
                    bail!("{url} could not be opened");
                }
                Ok(text(format!("opened {}", state.url)))
            }
            "browser_back" => {
                anyhow::ensure!(tab.state.lock().unwrap().back, "there is no earlier page");
                tab.send(&json!({"cmd": "back"})).await?;
                self.settle(&tab, Duration::from_secs(15)).await?;
                Ok(text(format!("back to {}", tab.state.lock().unwrap().url)))
            }
            "browser_reload" => {
                tab.state.lock().unwrap().loading = true;
                tab.send(&json!({"cmd": "reload"})).await?;
                self.settle(&tab, Duration::from_secs(15)).await?;
                Ok(text("reloaded"))
            }
            "browser_snapshot" => {
                let tree = self.snapshot(&tab).await?;
                let url = tab.state.lock().unwrap().url.clone();
                Ok(text(format!("{NOTICE}\nURL: {url}\n{tree}")))
            }
            "browser_screenshot" => {
                let clip = match string("ref") {
                    Some(reference) => Some(self.clip(&tab, &reference).await?),
                    None => None,
                };
                tokio::time::sleep(Duration::from_millis(100)).await;
                let png = tab.screenshot(clip).await?;
                let data = base64::engine::general_purpose::STANDARD.encode(&png);
                let mut content =
                    vec![json!({"type": "image", "data": data, "mimeType": "image/png"})];
                if let Some(path) = string("path") {
                    if request.read_only {
                        bail!("this session may only look, and saving a picture writes a file");
                    }
                    let saved = save_picture(request.cwd.as_deref(), &path, &data)?;
                    content.push(json!({"type": "text", "text": format!(
                        "saved to {}; hand it to the person with share_file", saved.display())}));
                }
                Ok(json!({"content": content, "isError": false}))
            }
            "browser_click" | "browser_hover" => {
                let reference = need("ref")?;
                let [x, y, width, height] = self.clip(&tab, &reference).await?;
                let (x, y) = (x + width / 2.0, y + height / 2.0);
                self.mouse(&tab, "mouseMoved", x, y).await?;
                if tool == "browser_click" {
                    self.mouse(&tab, "mousePressed", x, y).await?;
                    self.mouse(&tab, "mouseReleased", x, y).await?;
                    self.settle(&tab, Duration::from_secs(10)).await?;
                }
                Ok(text(format!(
                    "{} {reference}",
                    if tool == "browser_click" {
                        "clicked"
                    } else {
                        "hovering"
                    }
                )))
            }
            "browser_type" => {
                let reference = need("ref")?;
                let typed = need("text")?;
                // A click first, as a person would: the keyboard goes where the engine last
                // saw a pointer, and a script's focus() alone left it in the wrong field.
                let [x, y, width, height] = self.clip(&tab, &reference).await?;
                let (x, y) = (x + width / 2.0, y + height / 2.0);
                for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
                    self.mouse(&tab, kind, x, y).await?;
                }
                // The click and the keyboard reach the page by different ways, so text sent at
                // once can arrive before the click has moved the focus (measured: it landed in
                // the field that had it before). Wait until the page says the field has it.
                let end = Instant::now() + Duration::from_secs(2);
                let mut forced = false;
                loop {
                    let script = script_for(FOCUS, &reference)
                        .replace("__FORCE__", if forced { "true" } else { "false" });
                    if parse(&tab.eval(&script).await?)?["ok"].as_bool() == Some(true) {
                        break;
                    }
                    if Instant::now() >= end {
                        if forced {
                            bail!("{reference} cannot take the keyboard; is it a field?");
                        }
                        forced = true;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                tab.send(&json!({"cmd": "input", "type": "text", "text": typed}))
                    .await?;
                if args["submit"].as_bool().unwrap_or(false) {
                    self.press(&tab, "Enter").await?;
                    self.settle(&tab, Duration::from_secs(10)).await?;
                }
                Ok(text(format!("typed into {reference}")))
            }
            "browser_press" => {
                self.press(&tab, &need("key")?).await?;
                self.settle(&tab, Duration::from_secs(10)).await?;
                Ok(text("pressed"))
            }
            "browser_upload" => bail!(
                "this engine has no file chooser; set browser.engine to \"chromium\" for uploads"
            ),
            "browser_network" => bail!(
                "this engine does not record requests; set browser.engine to \"chromium\" for \
                 the network log"
            ),
            "browser_console" => {
                let since = args["since"].as_u64().unwrap_or(0);
                let state = tab.state.lock().unwrap();
                let first = state.console_total - state.console.len() as u64;
                let lines: Vec<String> = state
                    .console
                    .iter()
                    .enumerate()
                    .map(|(i, line)| (first + i as u64 + 1, line))
                    .filter(|(n, _)| *n > since)
                    .map(|(n, line)| format!("[{n}] {line}"))
                    .collect();
                Ok(text(format!(
                    "{NOTICE}\n{}\nnext: {}",
                    if lines.is_empty() {
                        "(nothing new)".to_owned()
                    } else {
                        lines.join("\n")
                    },
                    state.console_total.max(since)
                )))
            }
            "browser_wait" => {
                if let Some(wanted) = string("text") {
                    let probe = format!(
                        "document.body ? document.body.innerText.includes({}) : false",
                        serde_json::to_string(&wanted)?
                    );
                    let end = Instant::now() + Duration::from_secs(10);
                    loop {
                        if tab.eval(&probe).await?.as_bool().unwrap_or(false) {
                            return Ok(text("it is on the page"));
                        }
                        if Instant::now() >= end {
                            bail!("{wanted:?} did not appear in 10 seconds");
                        }
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                }
                tokio::time::sleep(Duration::from_millis(
                    args["ms"].as_u64().unwrap_or(1000).min(10_000),
                ))
                .await;
                Ok(text("waited"))
            }
            "browser_evaluate" => {
                let value = tab.eval(&need("expression")?).await?;
                self.settle(&tab, Duration::from_secs(5)).await?;
                Ok(text(format!("{NOTICE}\n{value}")))
            }
            other => bail!("{other} is not a browser tool"),
        }
    }
}
