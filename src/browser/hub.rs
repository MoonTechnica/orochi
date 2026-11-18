//! Which engine answers a call: Servo where it is configured and present, for the person's tab
//! and the agents' sessions alike, Chrome for whatever only it does (`docs/servo-browser-design.md` §3.4).
//!
//! Everything that reaches the broker — the unix socket the window and the agents' MCP relays
//! use, and the phone listener — goes through [`Hub::call`], so a rule here (an agent cannot
//! touch the person's tabs; a tab stays on the engine it started on) holds for all of them.
use super::{
    broker::{Browser, Request, failure, text},
    engine::Engines,
    remote::{Dispatch, Remote, Reply},
};
use crate::config::{BrowserConfig, BrowserEngine};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub struct Hub {
    pub browser: Option<Arc<Browser>>,
    pub engines: Engines,
    config: BrowserConfig,
    data: PathBuf,
    /// Called on every request, so a phone using the browser keeps the broker from idling out.
    activity: Arc<dyn Fn() + Send + Sync>,
    remote: Mutex<Option<Arc<Remote>>>,
}

/// The phone listener's way back into the hub.
struct Back(Arc<Hub>);
impl Dispatch for Back {
    fn call(&self, request: Request) -> Reply {
        let hub = self.0.clone();
        Box::pin(async move { hub.call(request).await })
    }
}

/// What a `view_*` call needs from the engine that holds the tab. Previewing a folder is
/// Chrome's: it confines requests with Chrome's own interception. (Watching an agent's page is
/// answered by whichever engine holds that agent's page.)
fn chrome_only(request: &Request) -> bool {
    request.tool == "view_preview"
}

impl Hub {
    pub fn new(
        browser: Option<Arc<Browser>>,
        engines: Engines,
        config: &BrowserConfig,
        data: &Path,
        activity: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            browser,
            engines,
            config: config.clone(),
            data: data.to_path_buf(),
            activity,
            remote: Mutex::new(None),
        }
    }

    /// Starts the phone listener now, whatever `browser.remote.enabled` says: the window's
    /// switch saves that setting and calls this, and the broker's own copy of it is older.
    pub async fn start_remote(self: &Arc<Self>) -> anyhow::Result<Arc<Remote>> {
        if let Some(remote) = self.remote.lock().unwrap().clone() {
            return Ok(remote);
        }
        // The settings the window just saved, not the ones this broker started with.
        let remote_config = super::server::load(&self.data)
            .map_or_else(|_| self.config.remote.clone(), |c| c.remote);
        crate::browser::policy::remote_bind(&remote_config)?;
        let remote = Remote::start(
            &self.data,
            &remote_config,
            Arc::new(Back(self.clone())),
            self.activity.clone(),
        )
        .await?;
        *self.remote.lock().unwrap() = Some(remote.clone());
        Ok(remote)
    }

    /// The broker is ending: the phone's sessions close and every engine is asked to exit, so
    /// what a page stored reaches the profile.
    pub async fn shutdown(&self) {
        self.stop_remote();
        self.engines.shutdown_all().await;
    }

    pub fn stop_remote(&self) {
        if let Some(remote) = self.remote.lock().unwrap().take() {
            remote.stop();
        }
    }

    /// `remote_*`: the window's switch, pairing and device list. They arrive on the broker's
    /// unix socket, which only this user can open, and are not among the tools the phone may
    /// call (`remote.rs` names the ones it may).
    async fn admin(self: &Arc<Self>, request: &Request) -> Value {
        let result: anyhow::Result<Value> = async {
            match request.tool.as_str() {
                "remote_start" => {
                    let remote = self.start_remote().await?;
                    Ok(json!({"running": true, "address": remote.address().to_string()}))
                }
                "remote_stop" => {
                    self.stop_remote();
                    Ok(json!({"running": false}))
                }
                "remote_status" => {
                    let remote = self.remote.lock().unwrap().clone();
                    Ok(match remote {
                        None => json!({"running": false, "devices": []}),
                        Some(remote) => {
                            let connected = remote.connected();
                            let devices: Vec<Value> = remote
                                .devices()
                                .iter()
                                .map(|d| {
                                    json!({"id": d.id, "name": d.name, "created": d.created,
                                           "last_seen": d.last_seen,
                                           "connected": connected.contains(&d.id)})
                                })
                                .collect();
                            json!({"running": true, "address": remote.address().to_string(),
                                   "devices": devices})
                        }
                    })
                }
                "remote_pair" => {
                    let remote = self.start_remote().await?;
                    let pairing = remote.pair()?;
                    Ok(json!({"url": pairing.url, "code": pairing.code,
                              "expires_secs": pairing.expires_secs}))
                }
                "remote_revoke" => {
                    let id = request.args["id"].as_str().unwrap_or("");
                    let remote = self.remote.lock().unwrap().clone();
                    Ok(json!({"revoked": remote.is_some_and(|r| r.revoke(id))}))
                }
                other => anyhow::bail!("{other} is not something the browser does"),
            }
        }
        .await;
        match result {
            Ok(value) => text(value.to_string()),
            Err(error) => failure(format!("{error:#}")),
        }
    }

    pub async fn forget(&self, scope: &str) {
        self.engines.forget(scope).await;
        if let Some(browser) = &self.browser {
            browser.forget(scope).await;
        }
    }

    pub async fn call(self: &Arc<Self>, request: Request) -> Value {
        (self.activity)();
        if request.tool.starts_with("remote_") {
            return self.admin(&request).await;
        }
        if request.tool == "view_list" {
            return self.list(&request);
        }
        if !request.tool.starts_with("view_") {
            // An agent's session sticks to the engine it started on, like the person's tab.
            if self.wants_servo(&request.scope) {
                match self.engines.warm(&request.scope).await {
                    Ok(()) => return self.engines.agent(&request).await,
                    Err(error)
                        if self.browser.is_some()
                            && !self.engines.owns(&request.scope)
                            && self.config.engine == BrowserEngine::Auto =>
                    {
                        tracing::debug!(%error, "servo could not start a session; using Chrome");
                    }
                    Err(error) => return failure(format!("{error:#}")),
                }
            }
            return match &self.browser {
                Some(browser) => browser.call(request).await,
                None => {
                    failure("no Chrome, Chromium or orochi-servo was found for the agents' browser")
                }
            };
        }
        // Same wall as the Chrome path has: a person's tab is never an agent's.
        if !request.scope.starts_with("user:") {
            return failure("that belongs to the person's browser tab");
        }
        if request.tool == "view_agents" {
            return self.agents(request).await;
        }
        if let Some(agent) = request.args["watch"].as_str()
            && self.engines.owns(agent)
        {
            return match self
                .engines
                .watch(agent, &request.tool, &request.args)
                .await
            {
                Ok(value) => text(value.to_string()),
                Err(error) => failure(format!("{error:#}")),
            };
        }
        let servo = !chrome_only(&request)
            && request.args["watch"].is_null()
            && self.wants_servo(&request.scope);
        if servo {
            match self
                .engines
                .view(&request.scope, &request.tool, &request.args)
                .await
            {
                Ok(value) => return text(value.to_string()),
                // Servo was an option, not a requirement, and it could not start this tab.
                Err(error)
                    if self.browser.is_some()
                        && !self.engines.owns(&request.scope)
                        && self.config.engine == BrowserEngine::Auto =>
                {
                    tracing::debug!(%error, "servo could not start a tab; using Chrome");
                }
                Err(error) => return failure(format!("{error:#}")),
            }
        }
        match &self.browser {
            Some(browser) => browser.call(request).await,
            None => failure(
                if chrome_only(&request) || !request.args["watch"].is_null() {
                    "a file is not previewed without Chrome, which Orochi does not run: serve the \
                 folder with a web server (for example `python3 -m http.server` in it) and \
                 open its address in a browser tab"
                        .to_owned()
                } else if self.engines.required() {
                    self.engines.why_missing()
                } else {
                    "no Chrome, Chromium or orochi-servo was found for the browser tab".to_owned()
                },
            ),
        }
    }

    fn wants_servo(&self, scope: &str) -> bool {
        if self.engines.owns(scope) {
            return true;
        }
        if self.browser.as_ref().is_some_and(|b| b.has_scope(scope)) {
            return false;
        }
        match self.config.engine {
            BrowserEngine::Chromium => false,
            // Required: a missing binary is reported, not worked around.
            BrowserEngine::Servo => true,
            BrowserEngine::Auto => self.engines.available(),
        }
    }

    /// The agents that have a page open, on either engine.
    async fn agents(&self, request: Request) -> Value {
        let mut agents = self.engines.agents().await;
        if let Some(browser) = &self.browser {
            let reply = browser.call(request).await;
            if let Some(chrome) = reply["content"][0]["text"]
                .as_str()
                .and_then(|t| serde_json::from_str::<Value>(t).ok())
                .and_then(|v| v["agents"].as_array().cloned())
            {
                agents.extend(chrome);
            }
        }
        agents.sort_by(|a, b| a["scope"].as_str().cmp(&b["scope"].as_str()));
        text(json!({"agents": agents}).to_string())
    }

    /// The person's tabs on either engine. Not a thing an agent's scope may ask for.
    fn list(&self, request: &Request) -> Value {
        if !request.scope.starts_with("user:") {
            return failure("that belongs to the person's browser tab");
        }
        let mut tabs = self.engines.list();
        if let Some(browser) = &self.browser {
            tabs.extend(browser.user_tabs());
        }
        tabs.sort_by(|a, b| a["scope"].as_str().cmp(&b["scope"].as_str()));
        text(json!({"tabs": tabs}).to_string())
    }
}
