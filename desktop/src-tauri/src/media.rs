//! What the `orochi-media://` scheme answers (`docs/conversation-tabs-design.md` §4.4).
//!
//! A picture or a recording is a file beside the store, and the window asks for it by thread
//! and hash. The answer is built here, away from the webview, so it is tested without one:
//! both parts of the path are checked as plain ids, nothing else is ever opened, the type comes
//! from the file's own bytes, and a `Range` is honoured because WKWebView will not play a
//! `<video>` without partial answers.
use std::{io::Read, io::Seek, path::Path};

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

fn plain(status: u16, text: &str) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type", "text/plain; charset=utf-8".into())],
        body: text.as_bytes().to_vec(),
    }
}

/// `request` is the URL's path, `/<thread>/<sha256>`; `range` is the request's `Range` header.
pub fn serve(data: &Path, request: &str, range: Option<&str>) -> Reply {
    let mut parts = request.trim_start_matches('/').split('/');
    let (Some(thread), Some(sha), None) = (parts.next(), parts.next(), parts.next()) else {
        return plain(404, "not found");
    };
    let Some(path) = orochi::media::file(data, thread, sha) else {
        return plain(404, "not found");
    };
    let Ok(mut file) = std::fs::File::open(&path) else {
        return plain(404, "not found");
    };
    let Ok(length) = file.metadata().map(|m| m.len()) else {
        return plain(404, "not found");
    };
    let mut head = [0u8; 2048];
    let read = file.read(&mut head).unwrap_or(0);
    let Some(kind) = orochi::media::sniff(&head[..read]) else {
        return plain(404, "not found");
    };
    let mut headers = vec![
        ("Content-Type", kind.mime.to_string()),
        ("Accept-Ranges", "bytes".into()),
        ("X-Content-Type-Options", "nosniff".into()),
        // An SVG opened on its own may not run script; it is only ever shown through <img>.
        (
            "Content-Security-Policy",
            "default-src 'none'; sandbox".into(),
        ),
        // The name is the hash of the content, so the answer never changes.
        (
            "Cache-Control",
            "private, max-age=31536000, immutable".into(),
        ),
    ];
    let (start, end, status) = match range.and_then(|r| byte_range(r, length)) {
        Some((start, end)) => (start, end, 206),
        None if range.is_some() => {
            headers.push(("Content-Range", format!("bytes */{length}")));
            return Reply {
                status: 416,
                headers,
                body: Vec::new(),
            };
        }
        None => (0, length.saturating_sub(1), 200),
    };
    if status == 206 {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{length}")));
    }
    let mut body = vec![
        0u8;
        if length == 0 {
            0
        } else {
            (end - start + 1) as usize
        }
    ];
    if file.seek(std::io::SeekFrom::Start(start)).is_err() || file.read_exact(&mut body).is_err() {
        return plain(404, "not found");
    }
    headers.push(("Content-Length", body.len().to_string()));
    Reply {
        status,
        headers,
        body,
    }
}

/// `bytes=START-END`, `bytes=START-` or `bytes=-SUFFIX` for a file of `length`, as inclusive
/// offsets. Several ranges are not served; the client then gets the whole file's answer or a
/// 416, which every player copes with.
fn byte_range(header: &str, length: u64) -> Option<(u64, u64)> {
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || length == 0 {
        return None;
    }
    let (from, to) = spec.split_once('-')?;
    let (start, end) = match (from.trim(), to.trim()) {
        ("", suffix) => {
            let suffix: u64 = suffix.parse().ok()?;
            (length.saturating_sub(suffix.min(length)), length - 1)
        }
        (from, "") => (from.parse().ok()?, length - 1),
        (from, to) => (from.parse().ok()?, to.parse::<u64>().ok()?.min(length - 1)),
    };
    (start <= end && start < length).then_some((start, end))
}
