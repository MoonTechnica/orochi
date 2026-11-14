//! Images, audio and video a conversation produced (`docs/conversation-tabs-design.md` §4).
//!
//! Bytes live in files, never in a row: `<data>/activity-media/<thread>/<sha256>.<ext>`,
//! `0600`, content-addressed so the same screenshot taken twice is one file. The activity
//! store keeps the index (`media`, `v_media`) and the timeline item that says where it came
//! from. Nothing here reaches telemetry, and the type of a file is read from its bytes, never
//! from what an agent said it was.
use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// One file past this is not kept (its item says so).
pub const FILE_CAP: u64 = 25 * 1024 * 1024;
/// One thread's media past this drops the oldest first.
pub const THREAD_CAP: u64 = 500 * 1024 * 1024;
/// How many paths one piece of text may name before the rest are ignored.
const PATHS_PER_TEXT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sniffed {
    pub mime: &'static str,
    pub ext: &'static str,
    pub kind: Kind,
}

/// Where a piece of media came from: bytes an agent sent, or a file it wrote or named.
#[derive(Debug, Clone)]
pub enum Source {
    Inline(Vec<u8>),
    Path(PathBuf),
}

#[derive(Debug, Clone)]
pub struct MediaRef {
    pub name: String,
    pub source: Source,
}

/// What was kept, as the index and the timeline item record it.
#[derive(Debug, Clone, Serialize)]
pub struct Kept {
    pub sha: String,
    pub mime: String,
    pub ext: String,
    pub kind: Kind,
    pub bytes: u64,
    pub name: String,
    /// `inline` for bytes an agent sent, `path` for a file it wrote or named.
    pub source: &'static str,
    pub from: Option<String>,
    /// False when the file was too large to keep; the item still says what it was.
    pub kept: bool,
}

/// What the bytes are, or `None` for anything that is not media. HTML, PDF and archives are
/// deliberately not media: they have viewers of their own or none.
pub fn sniff(bytes: &[u8]) -> Option<Sniffed> {
    let is = |mime, ext, kind| Some(Sniffed { mime, ext, kind });
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return is("image/png", "png", Kind::Image);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return is("image/jpeg", "jpg", Kind::Image);
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return is("image/gif", "gif", Kind::Image);
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" {
        match &bytes[8..12] {
            b"WEBP" => return is("image/webp", "webp", Kind::Image),
            b"WAVE" => return is("audio/wav", "wav", Kind::Audio),
            _ => {}
        }
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return match &bytes[8..12] {
            b"avif" | b"avis" => is("image/avif", "avif", Kind::Image),
            b"qt  " => is("video/quicktime", "mov", Kind::Video),
            b"M4A " | b"M4B " => is("audio/mp4", "m4a", Kind::Audio),
            _ => is("video/mp4", "mp4", Kind::Video),
        };
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return is("video/webm", "webm", Kind::Video);
    }
    if bytes.starts_with(b"OggS") {
        return is("audio/ogg", "ogg", Kind::Audio);
    }
    if bytes.starts_with(b"ID3") || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        return is("audio/mpeg", "mp3", Kind::Audio);
    }
    // SVG is text: only an image, and only when the document element is `svg`.
    let head = &bytes[..bytes.len().min(2048)];
    if let Ok(text) = std::str::from_utf8(head) {
        let text = text.trim_start_matches('\u{feff}').trim_start();
        if (text.starts_with("<svg") || text.starts_with("<?xml")) && text.contains("<svg") {
            return is("image/svg+xml", "svg", Kind::Image);
        }
    }
    None
}

const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "avif", "svg", "mp4", "mov", "m4v", "webm", "mp3", "wav",
    "ogg", "m4a",
];

fn media_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Files a piece of text names that are worth keeping: an absolute path or one relative to
/// `cwd`, that exists, is a regular file with a media extension, and — after symlinks are
/// resolved — lies under `cwd` or one of `roots` (the temporary directory, Orochi's data). A
/// path anywhere else stays a plain word: an agent's reply must not make Orochi read `/etc`.
pub fn find_paths(text: &str, cwd: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect();
    let mut found: Vec<PathBuf> = Vec::new();
    let delimiters = |c: char| c.is_whitespace() || "\"'`()[]<>{}|,;=".contains(c);
    for token in text.split(delimiters) {
        let token = token.trim_end_matches(['.', ':', '!', '?']);
        let token = token.strip_prefix("file://").unwrap_or(token);
        if token.is_empty() || token.len() > 1024 {
            continue;
        }
        let path = Path::new(token);
        if !media_extension(path) {
            continue;
        }
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        let Ok(real) = std::fs::canonicalize(&candidate) else {
            continue;
        };
        if !real.is_file() || !(real.starts_with(&cwd) || roots.iter().any(|r| real.starts_with(r)))
        {
            continue;
        }
        if !found.contains(&real) {
            found.push(real);
        }
        if found.len() >= PATHS_PER_TEXT {
            break;
        }
    }
    found
}

pub fn store_root(data: &Path) -> PathBuf {
    data.join("activity-media")
}

pub fn thread_dir(data: &Path, thread: &str) -> PathBuf {
    store_root(data).join(thread)
}

fn valid_id(id: &str, hex: bool) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_hexdigit() || (!hex && (c == '-' || c.is_ascii_alphanumeric())))
}

/// The stored file for a thread's media, by hash. Both parts are validated as plain ids, so
/// there is no path to walk out of the store with.
pub fn file(data: &Path, thread: &str, sha: &str) -> Option<PathBuf> {
    if !valid_id(thread, false) || sha.len() != 64 || !valid_id(sha, true) {
        return None;
    }
    std::fs::read_dir(thread_dir(data, thread))
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|stem| stem == sha)
        })
}

/// Reads what a reference holds, bounded: a path larger than the cap is not read at all.
fn read(source: &Source) -> Result<(Vec<u8>, Option<String>)> {
    match source {
        Source::Inline(bytes) => Ok((bytes.clone(), None)),
        Source::Path(path) => {
            let length = std::fs::metadata(path)?.len();
            if length > FILE_CAP {
                bail!("larger than the {FILE_CAP}-byte cap");
            }
            Ok((std::fs::read(path)?, Some(path.display().to_string())))
        }
    }
}

/// Keeps one reference: sniffs it, and writes it under its hash unless it is already there.
/// `None` means it is not media. A file over the cap comes back with `kept = false`, hashed
/// when it is inline and described only by its name when it is a path too large to read.
pub fn keep(data: &Path, thread: &str, reference: &MediaRef) -> Result<Option<Kept>> {
    if !valid_id(thread, false) {
        bail!("not a thread id");
    }
    if let Source::Inline(bytes) = &reference.source
        && bytes.len() as u64 > FILE_CAP
    {
        return Ok(None);
    }
    let (bytes, from) = match read(&reference.source) {
        Ok(read) => read,
        Err(_) => return Ok(None),
    };
    let Some(sniffed) = sniff(&bytes) else {
        return Ok(None);
    };
    let sha = format!("{:x}", Sha256::digest(&bytes));
    let dir = thread_dir(data, thread);
    let path = dir.join(format!("{sha}.{}", sniffed.ext));
    if !path.exists() {
        std::fs::create_dir_all(&dir).context("media directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for directory in [&store_root(data), &dir] {
                let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
            }
        }
        write_private(&path, &bytes)?;
    }
    Ok(Some(Kept {
        sha,
        mime: sniffed.mime.into(),
        ext: sniffed.ext.into(),
        kind: sniffed.kind,
        bytes: bytes.len() as u64,
        name: reference.name.clone(),
        source: if from.is_some() { "path" } else { "inline" },
        from,
        kept: true,
    }))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    // Written beside and renamed, so a reader never sees half a file.
    let partial = path.with_extension("partial");
    let mut file = options.open(&partial).or_else(|_| {
        let _ = std::fs::remove_file(&partial);
        options.open(&partial)
    })?;
    file.write_all(bytes)?;
    file.sync_all().ok();
    std::fs::rename(&partial, path)?;
    Ok(())
}

pub fn remove_file(data: &Path, thread: &str, sha: &str) {
    if let Some(path) = file(data, thread, sha) {
        let _ = std::fs::remove_file(path);
    }
}

/// Removes every media directory whose thread is gone: what a deleted thread, a retention
/// sweep or an interrupted run left behind.
pub fn sweep(data: &Path, live: &[String]) {
    let Ok(entries) = std::fs::read_dir(store_root(data)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !live.iter().any(|id| *id == name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `bytes` of base64 text, decoded; whitespace is ignored and padding is optional.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A content block of an ACP update (`image` or `audio`) as a reference, when it carries
/// bytes. A `uri` alone is not fetched: the agent's word is not a reason to read an address.
pub fn from_block(block: &serde_json::Value, fallback: &str) -> Option<MediaRef> {
    if !matches!(block["type"].as_str(), Some("image" | "audio")) {
        return None;
    }
    let bytes = decode_base64(block["data"].as_str()?)?;
    let name = block["uri"]
        .as_str()
        .and_then(|uri| uri.rsplit('/').next())
        .filter(|name| !name.is_empty() && name.len() <= 120 && !name.contains(['\\', ':']))
        .unwrap_or(fallback);
    Some(MediaRef {
        name: name.to_owned(),
        source: Source::Inline(bytes),
    })
}

/// The address a `resource_link` block names, which is only a name until the recorder has
/// checked where it points.
pub fn link_of(block: &serde_json::Value) -> Option<&str> {
    if block["type"].as_str() != Some("resource_link") {
        return None;
    }
    block["uri"].as_str().filter(|uri| uri.len() <= 1024)
}

/// Whether `path` is a file this module would keep as media, by its name alone.
pub fn is_media_name(path: &Path) -> bool {
    media_extension(path)
}

/// One muted line for a terminal, which does not draw pictures: `[image: shot.png · 184 KiB]`.
pub fn describe(reference: &MediaRef) -> String {
    let (bytes, what) = match &reference.source {
        Source::Inline(bytes) => (bytes.len() as u64, sniff(bytes)),
        Source::Path(path) => (
            std::fs::metadata(path).map_or(0, |m| m.len()),
            std::fs::read(path).ok().and_then(|b| sniff(&b)),
        ),
    };
    let kind = what.map_or("media", |s| match s.kind {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Audio => "audio",
    });
    let mime = what.map_or(String::new(), |s| format!(" · {}", s.mime));
    format!(
        "[{kind}: {}{mime} · {} KiB]",
        reference.name,
        bytes.div_ceil(1024)
    )
}
