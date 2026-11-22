//! Telling the user (`docs/resident-agent-design.md` §13).
//!
//! A notice is a row before it is anything else, so what failed to arrive can still be read
//! in the window and the app. This decides where each one goes: a notification on this Mac
//! when no window is open to show it, and a push to the paired phones through OneSignal when
//! the user has been away, or when a question is waiting. A third party only ever sees what
//! the user configured it to, and nothing without a key the user gave.
pub mod key;

use crate::{
    activity::{Activity, residents::Notice},
    config::{Config, NotifyConfig, quiet_hours},
    routines::{Zone, now},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A phone this machine may push to: the random id it registered with OneSignal when it
/// paired. It names nobody.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Phone {
    pub id: String,
    pub name: String,
}

pub fn phones_path(data: &Path) -> PathBuf {
    data.join("remote").join("push.json")
}

pub fn phones(data: &Path) -> Vec<Phone> {
    std::fs::read_to_string(phones_path(data))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(data: &Path, phones: &[Phone]) -> Result<()> {
    let path = phones_path(data);
    std::fs::create_dir_all(path.parent().expect("a folder"))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(phones)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// A pairing's push id: 16 to 64 characters of `[A-Za-z0-9_-]`, which is all a random id is.
pub fn valid_push_id(id: &str) -> bool {
    (16..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn add_phone(data: &Path, id: &str, name: &str) -> Result<()> {
    anyhow::ensure!(
        valid_push_id(id),
        "a push id is 16 to 64 letters, digits, - or _"
    );
    let mut all = phones(data);
    all.retain(|p| p.id != id);
    all.push(Phone {
        id: id.to_owned(),
        name: name.chars().take(60).collect(),
    });
    save(data, &all)
}

pub fn remove_phone(data: &Path, id: &str) -> Result<bool> {
    let mut all = phones(data);
    let before = all.len();
    all.retain(|p| p.id != id);
    save(data, &all)?;
    Ok(all.len() != before)
}

/// What each sink is shown of a notice, as `notify.level` allows. The heading never carries
/// anything but the kind of notice.
pub fn shown(level: &str, notice: &Notice) -> (String, String) {
    let heading = match notice.kind.as_str() {
        "ask" => "Approval needed",
        "report" => "Routine finished",
        "mention" => "Mentioned you",
        "trouble" => "Routine problem",
        "scheduled" => "Routine scheduled",
        "budget" => "Routine budget",
        _ => "Orochi",
    };
    let body = notice.body.as_deref().unwrap_or("").trim();
    let text = match level {
        "full" => format!("{}\n{}", notice.title, body),
        "summary" => format!(
            "{}: {}",
            notice.title,
            body.chars().take(120).collect::<String>()
        ),
        _ => notice.title.clone(),
    };
    (heading.to_owned(), text.chars().take(1000).collect())
}

/// Whether `minute` of the day falls inside `from-to`, which may wrap midnight.
pub fn inside(quiet: (u16, u16), minute: u16) -> bool {
    let (from, to) = quiet;
    if from == to {
        false
    } else if from < to {
        (from..to).contains(&minute)
    } else {
        minute >= from || minute < to
    }
}

pub fn in_quiet_hours(config: &NotifyConfig, at: i64, zone: Zone) -> bool {
    let Some(quiet) = quiet_hours(&config.quiet) else {
        return false;
    };
    let p = zone.parts(at);
    inside(quiet, (p.hour * 60 + p.minute) as u16)
}

/// The status each sink ended up with: `sent`, `held`, `off`, `skipped_window`, `coalesced`,
/// `not_registered`, `failed`, `no_key`, `no_phones`.
#[derive(Debug, Clone, Default)]
pub struct Delivery {
    pub mac: Option<String>,
    pub phone: Option<String>,
}

/// Decides and sends. `release` is a notice that was held and whose hold is over.
pub async fn deliver(
    config: &Config,
    data: &Path,
    activity: &Activity,
    notice: &Notice,
    release: bool,
) -> Delivery {
    let notify = &config.notify;
    let at = now();
    let zone = Zone::Local;
    if !notify.kinds.contains(&notice.kind) {
        return Delivery {
            mac: Some("off".into()),
            phone: Some("off".into()),
        };
    }
    let quiet = !release && in_quiet_hours(notify, at, zone);
    let wakes = notice.kind == "ask" && notify.wake_for_approvals;
    let idle_ms = crate::activity::millis() - activity.active_at().unwrap_or(0);
    let window_open = idle_ms < 120_000;
    let (heading, text) = shown(&notify.level, notice);

    // A notice released from quiet hours is sent again only where it was held.
    let kept = |previous: &Option<String>| {
        release
            .then(|| previous.clone())
            .flatten()
            .filter(|p| p != "held")
    };
    let mac = if let Some(done) = kept(&notice.mac) {
        done
    } else if !notify.mac {
        "off".to_owned()
    } else if quiet && !wakes {
        "held".to_owned()
    } else if window_open {
        "skipped_window".to_owned()
    } else {
        match mac(&heading, &text) {
            Ok(()) => "sent".to_owned(),
            Err(_) => "failed".to_owned(),
        }
    };

    let phone = if let Some(done) = kept(&notice.phone) {
        done
    } else if !notify.onesignal.enabled {
        "off".to_owned()
    } else if quiet && !wakes {
        "held".to_owned()
    } else if notice.kind != "ask" && idle_ms < notify.away_secs * 1000 {
        "skipped_window".to_owned()
    } else if coalesced(activity, notice, notify.coalesce_secs) {
        "coalesced".to_owned()
    } else {
        match push(config, data, &heading, &text, notice).await {
            Ok(status) => status,
            Err(_) => "failed".to_owned(),
        }
    };
    Delivery {
        mac: Some(mac),
        phone: Some(phone),
    }
}

/// A push for the same thread went out a moment ago: this one is in the window, not another
/// buzz.
fn coalesced(activity: &Activity, notice: &Notice, secs: i64) -> bool {
    notice.kind != "ask"
        && notice.thread_id.as_deref().is_some_and(|thread| {
            activity
                .notices(false, 20)
                .unwrap_or_default()
                .iter()
                .any(|n| {
                    n.id != notice.id
                        && n.thread_id.as_deref() == Some(thread)
                        && n.phone.as_deref() == Some("sent")
                        && notice.created_at - n.created_at < secs * 1000
                })
        })
}

/// A notification on this Mac. The text is an argument to a fixed script, never part of one.
pub fn mac(title: &str, body: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/osascript")
            .args([
                "-e",
                "on run argv",
                "-e",
                "display notification (item 2 of argv) with title (item 1 of argv)",
                "-e",
                "end run",
            ])
            .arg(title)
            .arg(body)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("could not run osascript")?;
        anyhow::ensure!(status.success(), "the notification was refused");
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (title, body);
        anyhow::bail!("no notification command on this platform")
    }
}

/// One push to every paired phone. OneSignal answers `200` with the aliases it did not know
/// under `errors.invalid_aliases`; a phone that has not opened the app since installing it
/// is one. Returns `sent`, `not_registered` (all unknown) or `no_phones`; anything else is an
/// error whose text is never kept, since a response can carry ids.
pub async fn push(
    config: &Config,
    data: &Path,
    heading: &str,
    text: &str,
    notice: &Notice,
) -> Result<String> {
    let notify = &config.notify;
    let ids: Vec<String> = phones(data).into_iter().map(|p| p.id).collect();
    if ids.is_empty() {
        return Ok("no_phones".into());
    }
    let Some(secret) = key::get(data, notify) else {
        return Ok("no_key".into());
    };
    let body = serde_json::json!({
        "app_id": notify.onesignal.app_id,
        "target_channel": "push",
        "include_aliases": { "external_id": ids },
        "headings": { "en": heading },
        "contents": { "en": text },
        "data": {
            "machine": crate::notify::machine_id(data),
            "thread": notice.thread_id,
            "notice": notice.id,
            "kind": notice.kind,
        },
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let response = client
        .post(&notify.onesignal.url)
        .header("Authorization", format!("Key {secret}"))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .context("the push could not be sent")?;
    anyhow::ensure!(response.status().is_success(), "the push was refused");
    let answer: serde_json::Value =
        serde_json::from_str(&response.text().await.unwrap_or_default()).unwrap_or_default();
    let unknown = answer["errors"]["invalid_aliases"]["external_id"]
        .as_array()
        .map_or(0, Vec::len);
    Ok(if unknown >= ids.len() && unknown > 0 {
        "not_registered".into()
    } else {
        "sent".into()
    })
}

/// The id this machine tells the phone it is, as the phone server's own (`machine.json`).
pub fn machine_id(data: &Path) -> String {
    std::fs::read_to_string(data.join("remote").join("machine.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["id"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// A test notice through every sink that is on, the way `orochi notify test` and the window's
/// *Send test* both do it. A test is meant to be seen, so the window being open does not hold
/// it back.
pub async fn send_test(config: &Config, data: &Path) -> Result<(i64, Delivery)> {
    anyhow::ensure!(config.activity.enabled, "activity.enabled is false");
    let activity = Activity::open(data, config.activity.retention_days)?;
    let id = activity.add_notice(
        "report",
        None,
        None,
        None,
        "Test from Orochi",
        Some("If you can read this, the notice reached you."),
    )?;
    let notice = activity
        .notices(false, 1)?
        .into_iter()
        .next()
        .context("the notice was not written")?;
    let mut shown = config.clone();
    shown.notify.away_secs = 0;
    let sent = deliver(&shown, data, &activity, &notice, true).await;
    activity.notice_delivery(id, sent.mac.as_deref(), sent.phone.as_deref())?;
    Ok((id, sent))
}
