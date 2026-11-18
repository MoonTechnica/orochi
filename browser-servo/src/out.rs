//! The one writer of the protocol. Servo and its dependencies may print to fd 1, so the
//! protocol runs on a private duplicate and fd 1 is pointed at stderr.
use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::sync::{Mutex, OnceLock};

static OUT: OnceLock<Mutex<File>> = OnceLock::new();

pub fn init() {
    // SAFETY: plain fd duplication at startup, before any other thread exists.
    let file = unsafe {
        let fd = libc::dup(1);
        assert!(fd >= 0, "cannot duplicate stdout");
        libc::dup2(2, 1);
        File::from_raw_fd(fd)
    };
    let _ = OUT.set(Mutex::new(file));
}

fn write(parts: &[&[u8]]) -> bool {
    let Some(out) = OUT.get() else { return false };
    let mut out = out.lock().unwrap_or_else(|e| e.into_inner());
    for part in parts {
        if out.write_all(part).is_err() {
            return false;
        }
    }
    out.flush().is_ok()
}

pub fn event(value: serde_json::Value) {
    let mut line = value.to_string().into_bytes();
    line.push(b'\n');
    if !write(&[&line]) {
        // The broker is gone.
        std::process::exit(0);
    }
}

pub fn frame(seq: u64, width: u32, height: u32, jpeg: &[u8]) {
    let mut line = serde_json::json!({"event": "frame", "seq": seq, "width": width,
                                      "height": height, "len": jpeg.len()})
    .to_string()
    .into_bytes();
    line.push(b'\n');
    if !write(&[&line, jpeg]) {
        std::process::exit(0);
    }
}

/// A binary answer to a command: a line naming it, then `bytes`.
pub fn blob(kind: &str, id: u64, bytes: &[u8]) {
    let mut line = serde_json::json!({"event": kind, "id": id, "len": bytes.len()})
        .to_string()
        .into_bytes();
    line.push(b'\n');
    if !write(&[&line, bytes]) {
        std::process::exit(0);
    }
}
