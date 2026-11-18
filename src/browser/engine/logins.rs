//! The sign-ins every Servo engine shares (`docs/servo-browser-design.md` §3.4).
//!
//! An engine writes its cookie jar only when it exits, and two engines on one profile each
//! write the whole of theirs, so the one that ends last erased the other's sign-in
//! (measured). Orochi keeps one file of its own instead: an engine is given what the file
//! holds when it starts, and hands back what it changed when it ends or when the person
//! gives a page back, and the broker, one process, merges. Cookies only: the sites that keep
//! a sign-in in `localStorage` still ask again.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

const KEEP_SECS: u64 = 180 * 24 * 3600;
const MAX_ENTRIES: usize = 5000;

pub struct Logins {
    path: PathBuf,
    lock: Mutex<()>,
}

/// Which cookie an item is: the address it was asked for, its name, domain and path.
pub fn key(item: &Value) -> String {
    let cookie = item["cookie"].as_str().unwrap_or("");
    let name = cookie
        .split(';')
        .next()
        .and_then(|first| first.split('=').next())
        .unwrap_or("")
        .trim();
    let attribute = |wanted: &str| {
        cookie
            .split(';')
            .skip(1)
            .filter_map(|part| part.trim().split_once('='))
            .find(|(k, _)| k.eq_ignore_ascii_case(wanted))
            .map(|(_, v)| v.to_ascii_lowercase())
            .unwrap_or_default()
    };
    format!(
        "{}|{name}|{}|{}",
        item["url"].as_str().unwrap_or(""),
        attribute("domain"),
        attribute("path")
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Logins {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> BTreeMap<String, Value> {
        let items: Vec<Value> = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| v["cookies"].as_array().cloned())
            .unwrap_or_default();
        items.into_iter().map(|item| (key(&item), item)).collect()
    }

    /// What a starting engine is given: every cookie not yet stale.
    pub fn load(&self) -> Vec<Value> {
        let _guard = self.lock.lock().unwrap();
        let oldest = now().saturating_sub(KEEP_SECS);
        self.read()
            .into_values()
            .filter(|item| item["at"].as_u64().unwrap_or(0) >= oldest)
            .map(|item| json!({"url": item["url"], "cookie": item["cookie"]}))
            .collect()
    }

    /// Takes what an engine changed; the newest word on a cookie wins.
    pub fn merge(&self, changed: &[Value]) {
        if changed.is_empty() {
            return;
        }
        let _guard = self.lock.lock().unwrap();
        let mut all = self.read();
        let at = now();
        for item in changed {
            all.insert(
                key(item),
                json!({"url": item["url"], "cookie": item["cookie"], "at": at}),
            );
        }
        let oldest = at.saturating_sub(KEEP_SECS);
        all.retain(|_, item| item["at"].as_u64().unwrap_or(0) >= oldest);
        let mut kept: Vec<Value> = all.into_values().collect();
        kept.sort_by_key(|item| std::cmp::Reverse(item["at"].as_u64().unwrap_or(0)));
        kept.truncate(MAX_ENTRIES);
        let _ = self.write(&json!({"cookies": kept}));
    }

    fn write(&self, value: &Value) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(value)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&temporary, &self.path)
    }
}
