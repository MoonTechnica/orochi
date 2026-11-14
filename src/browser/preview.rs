//! A folder's files served on loopback so an HTML page opens with its CSS, scripts and images
//! beside it (`docs/conversation-tabs-design.md` §4.7.2). The page runs in the person's browser
//! tab, whose every request outside this server's origin is refused (`broker::confined`), so a
//! page an agent wrote cannot reach anything but the folder it came from.
//!
//! Only `GET` and `HEAD`, only below the folder given, never a `.git` directory, never through a
//! symlink that leaves it; the address carries a random token, since another process on this
//! machine can open any loopback port.
use anyhow::Result;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const FILE_CAP: u64 = 50 * 1024 * 1024;

pub struct Preview {
    port: u16,
    roots: Arc<Mutex<HashMap<String, PathBuf>>>,
}

impl Preview {
    pub async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let roots: Arc<Mutex<HashMap<String, PathBuf>>> = Arc::default();
        let served = roots.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let roots = served.clone();
                tokio::spawn(async move {
                    let _ = answer(socket, roots).await;
                });
            }
        });
        Ok(Self { port, roots })
    }

    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The token a folder is served under; the same folder keeps its token.
    pub fn mount(&self, root: &Path) -> Result<String> {
        let root = std::fs::canonicalize(root)?;
        let mut roots = self.roots.lock().unwrap();
        if let Some((token, _)) = roots.iter().find(|(_, r)| **r == root) {
            return Ok(token.clone());
        }
        let token = uuid::Uuid::new_v4().simple().to_string();
        roots.insert(token.clone(), root);
        Ok(token)
    }
}

fn decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The file a request path names, or nothing: inside its folder, not under `.git`.
pub fn resolve(roots: &HashMap<String, PathBuf>, target: &str) -> Option<PathBuf> {
    let path = target.split(['?', '#']).next()?;
    let rest = path.strip_prefix('/')?;
    let (token, rel) = rest.split_once('/').unwrap_or((rest, ""));
    let root = roots.get(token)?;
    let rel = decode(rel)?;
    if rel.contains('\0') || rel.split('/').any(|p| p == ".." || p == ".git") {
        return None;
    }
    let mut full = root.join(rel);
    if full.is_dir() {
        full = full.join("index.html");
    }
    let full = std::fs::canonicalize(full).ok()?;
    (full.starts_with(root) && full.is_file()).then_some(full)
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

async fn answer(
    mut socket: tokio::net::TcpStream,
    roots: Arc<Mutex<HashMap<String, PathBuf>>>,
) -> Result<()> {
    let mut head = vec![0u8; 8192];
    let mut filled = 0;
    loop {
        let n = socket.read(&mut head[filled..]).await?;
        if n == 0 {
            return Ok(());
        }
        filled += n;
        if head[..filled].windows(4).any(|w| w == b"\r\n\r\n") || filled == head.len() {
            break;
        }
    }
    let request = String::from_utf8_lossy(&head[..filled]).into_owned();
    let line = request.lines().next().unwrap_or("");
    let mut parts = line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let found = if matches!(method, "GET" | "HEAD") {
        let found = resolve(&roots.lock().unwrap(), target);
        found.filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= FILE_CAP))
    } else {
        None
    };
    let Some(path) = found else {
        socket
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await?;
        return Ok(());
    };
    let body = tokio::fs::read(&path).await?;
    let header = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {}\r\ncontent-length: {}\r\nx-content-type-options: nosniff\r\ncache-control: no-store\r\nconnection: close\r\n\r\n",
        content_type(&path),
        body.len()
    );
    socket.write_all(header.as_bytes()).await?;
    if method == "GET" {
        socket.write_all(&body).await?;
    }
    socket.shutdown().await.ok();
    Ok(())
}
