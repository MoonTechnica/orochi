//! The decision model on this machine: llama.cpp's `llama-server` and a GGUF, both put under
//! `decision.models_dir` (`<data>/decision/` unset, `home`) and checked against pinned SHA-256s, and one server kept running for
//! every Orochi process on the machine. A server takes seconds to load its model (measured
//! 2026-11-08: 8.7 s for kev-4b, cold), which no turn should wait for twice, so it outlives
//! the process that started it: `orochi --internal-decision-server` leads its own process
//! group, holds the server, and stops it once nothing has asked for `idle_secs`. Nothing is
//! installed outside the data directory and nothing asks for root.
use super::{Gguf, Model};
use crate::config::DecisionConfig;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

/// llama.cpp's release with `/v1/systemone` for every model in `MODELS` (the API landed with
/// ggml-org/llama.cpp#29818 on 2026-10-02; this build was run here on 2026-11-08).
pub const LLAMA_BUILD: &str = "b12202";
pub const LLAMA_RELEASES: &str = "https://github.com/ggml-org/llama.cpp/releases/download";
/// (os, arch, archive, sha256), from the release's own asset digests.
const LLAMA_ARCHIVES: [(&str, &str, &str, &str); 6] = [
    (
        "macos",
        "aarch64",
        "llama-b12202-bin-macos-arm64.tar.gz",
        "e70cd5918fb1db75652cee66bd6ab9c933cc0c5acf632f15df672c00829ebcaf",
    ),
    (
        "macos",
        "x86_64",
        "llama-b12202-bin-macos-x64.tar.gz",
        "02ce0277e0e393f52debf96668fb61a67e3348b31b20875f2162663afda15f2a",
    ),
    (
        "linux",
        "x86_64",
        "llama-b12202-bin-ubuntu-x64.tar.gz",
        "9076cf9b2ce5b849384711e85b0c37d18c36b1475106c236941de2b6f0899a88",
    ),
    (
        "linux",
        "aarch64",
        "llama-b12202-bin-ubuntu-arm64.tar.gz",
        "6516662c665b06c756013ade93a77f9e7d84e4a3b07c0fde6eab8b37112601f7",
    ),
    (
        "windows",
        "x86_64",
        "llama-b12202-bin-win-cpu-x64.zip",
        "75d108e6f91907638fef0cbdf4cc230e56bd9eac71bbffb209ae8da1f932590e",
    ),
    (
        "windows",
        "aarch64",
        "llama-b12202-bin-win-cpu-arm64.zip",
        "3a9cb4df34bd4dd3b4c6f7fc1d1d802a1f758ca9ff9e5394d06212594fcadb90",
    ),
];

pub const SERVE_FLAG: &str = "--internal-decision-server";

static SUPERVISOR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
/// The `orochi` binary that runs the server's supervisor. The process's own executable unless
/// told otherwise, which a library caller such as a test is, since its own is not `orochi`.
pub fn supervisor(exe: PathBuf) {
    let _ = SUPERVISOR.set(exe);
}

/// The archive llama.cpp publishes for this machine and its checksum.
pub fn platform() -> Option<(&'static str, &'static str)> {
    LLAMA_ARCHIVES
        .iter()
        .find(|(os, arch, ..)| *os == std::env::consts::OS && *arch == std::env::consts::ARCH)
        .map(|(_, _, archive, sum)| (*archive, *sum))
}

/// Where a local decision model lives: `decision.models_dir`, else `<data>/decision/`. Every
/// function here takes this directory, never the data directory.
pub fn home(data: &Path, config: &DecisionConfig) -> PathBuf {
    config
        .models_dir
        .clone()
        .unwrap_or_else(|| data.join("decision"))
}
fn dir(home: &Path) -> PathBuf {
    home.to_path_buf()
}
fn server_binary(data: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    dir(data).join("llama").join(LLAMA_BUILD).join(name)
}
fn gguf_path(data: &Path, gguf: &Gguf) -> PathBuf {
    dir(data).join("models").join(gguf.file)
}

pub fn installed(data: &Path, model: &Model) -> bool {
    model
        .gguf
        .is_some_and(|g| server_binary(data).is_file() && gguf_path(data, &g).is_file())
}

/// How far a download has come, rewritten as it goes for a window to follow
/// (`<data>/decision/download.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Download {
    pub job: Option<String>,
    pub model: String,
    pub file: String,
    pub done: u64,
    pub total: u64,
    pub state: String,
    pub detail: Option<String>,
}
impl Download {
    pub fn read(data: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(dir(data).join("download.json")).ok()?).ok()
    }
    fn write(&self, data: &Path) {
        let path = dir(data).join("download.json");
        let fresh = path.with_extension("json.new");
        if std::fs::write(&fresh, serde_json::to_vec(self).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&fresh, &path);
        }
    }
}

/// llama.cpp and the model's GGUF, whichever is missing. `sources` overrides where they come
/// from (tests serve both locally). Each is downloaded beside its destination as `.part`,
/// hashed as it arrives and renamed only when the hash matches; an interrupted download
/// resumes from what it already has.
pub async fn install(
    data: &Path,
    model: &Model,
    releases: &str,
    gguf_url: Option<&str>,
    mut progress: impl FnMut(&Download),
) -> Result<()> {
    let gguf = model
        .gguf
        .with_context(|| format!("{} runs only in the cloud", model.name))?;
    std::fs::create_dir_all(dir(data))?;
    let mut state = Download {
        job: std::env::var("OROCHI_JOB").ok(),
        model: model.name.into(),
        ..Default::default()
    };
    let result = async {
        if !server_binary(data).is_file() {
            let (archive, sum) =
                platform().context("llama.cpp publishes no build for this machine")?;
            let stage = dir(data).join("llama");
            std::fs::create_dir_all(&stage)?;
            let file = stage.join(archive);
            state.file = archive.into();
            download(
                &format!("{releases}/{LLAMA_BUILD}/{archive}"),
                &file,
                sum,
                &mut |done, total| {
                    state.done = done;
                    state.total = total;
                    state.state = "running".into();
                    state.write(data);
                    progress(&state);
                },
            )
            .await?;
            unpack(&file, &stage).await?;
            let _ = std::fs::remove_file(&file);
        }
        let path = gguf_path(data, &gguf);
        if !path.is_file() {
            std::fs::create_dir_all(path.parent().expect("models directory"))?;
            state.file = gguf.file.into();
            download(
                gguf_url.unwrap_or(gguf.url),
                &path,
                gguf.sha256,
                &mut |done, total| {
                    state.done = done;
                    state.total = total.max(gguf.bytes);
                    state.state = "running".into();
                    state.write(data);
                    progress(&state);
                },
            )
            .await?;
        }
        anyhow::Ok(())
    }
    .await;
    state.state = if result.is_ok() { "done" } else { "failed" }.into();
    state.detail = result.as_ref().err().map(|e| format!("{e:#}"));
    state.write(data);
    progress(&state);
    result
}

async fn download(
    url: &str,
    destination: &Path,
    sha256: &str,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let part = destination.with_extension("part");
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut existing) = std::fs::File::open(&part) {
        have = std::io::copy(&mut existing, &mut HashWriter(&mut hasher))?;
    }
    let client = reqwest::Client::builder()
        .user_agent(concat!("orochi/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()?;
    let mut request = client.get(url);
    if have > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut response = request.send().await?.error_for_status()?;
    let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !resumed && have > 0 {
        hasher = Sha256::new();
        have = 0;
    }
    let total = have + response.content_length().unwrap_or(0);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)?;
    let mut last = Instant::now() - Duration::from_secs(1);
    progress(have, total);
    while let Some(chunk) = response.chunk().await? {
        file.write_all(&chunk)?;
        hasher.update(&chunk);
        have += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(250) {
            progress(have, total);
            last = Instant::now();
        }
    }
    file.sync_all()?;
    progress(have, total.max(have));
    let actual = format!("{:x}", hasher.finalize());
    if actual != sha256 {
        let _ = std::fs::remove_file(&part);
        bail!(
            "{} does not match its pinned checksum; it was removed",
            destination
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        );
    }
    std::fs::rename(&part, destination)?;
    Ok(())
}

struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The archive holds one directory, `llama-<build>/`, with the server and its libraries; it
/// becomes `<data>/decision/llama/<build>/`.
async fn unpack(archive: &Path, stage: &Path) -> Result<()> {
    let unpacked = tempfile::Builder::new()
        .prefix(".unpack-")
        .tempdir_in(stage)?;
    let output = tokio::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(unpacked.path())
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .context("unpacking needs `tar`")?;
    ensure!(
        output.status.success(),
        "unpacking llama.cpp failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let binary = server_binary(Path::new("")).file_name().unwrap().to_owned();
    let top = walkdir::WalkDir::new(unpacked.path())
        .max_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name() == binary)
        .and_then(|e| e.path().parent().map(Path::to_path_buf))
        .context("the llama.cpp archive did not contain llama-server")?;
    let destination = stage.join(LLAMA_BUILD);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(&top, &destination)?;
    Ok(())
}

// --- The shared server ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Running {
    /// The supervisor, which leads the server's process group.
    pid: u32,
    port: u16,
    model: String,
}
fn running_path(data: &Path) -> PathBuf {
    dir(data).join("server.json")
}
fn used_path(data: &Path) -> PathBuf {
    dir(data).join("used")
}
fn read_running(data: &Path) -> Option<Running> {
    serde_json::from_slice(&std::fs::read(running_path(data)).ok()?).ok()
}
fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}

/// The port of a server that is up for `model`, without starting one.
pub fn running(data: &Path, model: &str) -> Option<u16> {
    read_running(data)
        .filter(|r| r.model == model && alive(r.pid))
        .map(|r| r.port)
}

fn touch(data: &Path) {
    let _ = std::fs::write(used_path(data), b"");
}

async fn healthy(port: u16) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .is_ok_and(|r| r.status().is_success())
}

/// The endpoint of a server for `model`, started if none is up. Starting is serialized by a
/// lock, so two runs at once start one server.
pub async fn ensure(data: &Path, config: &DecisionConfig, model: &Model) -> Result<String> {
    ensure!(
        installed(data, model),
        "{} is not downloaded; run `orochi decision use local`",
        model.name
    );
    touch(data);
    if let Some(port) = running(data, model.name)
        && healthy(port).await
    {
        return Ok(endpoint(port));
    }
    let lock = Lock::take(data)?;
    // Another run may have started it while this one waited for the lock.
    if let Some(port) = running(data, model.name)
        && healthy(port).await
    {
        drop(lock);
        return Ok(endpoint(port));
    }
    if let Some(old) = read_running(data) {
        stop_group(old.pid);
        let _ = std::fs::remove_file(running_path(data));
    }
    let port = free_port()?;
    let gguf = gguf_path(data, &model.gguf.expect("installed"));
    let exe = match SUPERVISOR.get() {
        Some(exe) => exe.clone(),
        None => std::env::current_exe()?,
    };
    let mut command = std::process::Command::new(exe);
    command
        .arg(SERVE_FLAG)
        .arg(data)
        .arg(config.idle_secs.max(60).to_string())
        .arg(model.name)
        .arg(port.to_string())
        .arg(server_binary(data))
        .arg(&gguf)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .context("starting the decision model server")?;
    let deadline = Instant::now() + Duration::from_millis(config.timeout_ms.max(30_000));
    while Instant::now() < deadline {
        if healthy(port).await {
            drop(lock);
            return Ok(endpoint(port));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    drop(lock);
    bail!(
        "the decision model server did not come up; see {}",
        dir(data).join("server.log").display()
    )
}

fn endpoint(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1/systemone")
}

fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

fn stop_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGTERM);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Stops the server, if one is up. `false` when none was.
pub fn stop(data: &Path) -> bool {
    let Some(old) = read_running(data) else {
        return false;
    };
    let was = alive(old.pid);
    stop_group(old.pid);
    let _ = std::fs::remove_file(running_path(data));
    was
}

struct Lock(#[allow(dead_code)] std::fs::File);
impl Lock {
    fn take(data: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir(data).join("server.lock"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        }
        Ok(Self(file))
    }
}

/// `orochi --internal-decision-server <data> <idle secs> <model> <port> <server> <gguf>`:
/// runs the server, records where it is, and stops it when it dies or nothing has asked for
/// the idle time. The server's log goes to `<data>/decision/server.log`, overwritten per start.
pub fn serve(args: &[std::ffi::OsString]) -> std::process::ExitCode {
    match serve_inner(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(_) => std::process::ExitCode::FAILURE,
    }
}

fn serve_inner(args: &[std::ffi::OsString]) -> Result<()> {
    let [data, idle, model, port, server, gguf] = args else {
        bail!("usage");
    };
    let data = PathBuf::from(data);
    let idle = Duration::from_secs(idle.to_string_lossy().parse()?);
    let port: u16 = port.to_string_lossy().parse()?;
    let log = std::fs::File::create(dir(&data).join("server.log"))?;
    let mut child = std::process::Command::new(server)
        .arg("-m")
        .arg(gguf)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let me = Running {
        pid: std::process::id(),
        port,
        model: model.to_string_lossy().into_owned(),
    };
    let path = running_path(&data);
    let fresh = path.with_extension("json.new");
    std::fs::write(&fresh, serde_json::to_vec(&me)?)?;
    std::fs::rename(&fresh, &path)?;
    let mine = |data: &Path| read_running(data).is_some_and(|r| r.pid == me.pid);
    loop {
        std::thread::sleep(Duration::from_secs(5));
        if child.try_wait()?.is_some() {
            break;
        }
        let since = std::fs::metadata(used_path(&data))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .unwrap_or(idle);
        if since >= idle || !mine(&data) {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
    }
    if mine(&data) {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}

// --- What this machine has ------------------------------------------------------------------

pub fn memory_gb() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let mut bytes: u64 = 0;
        let mut size = std::mem::size_of::<u64>();
        let name = c"hw.memsize";
        let ok = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut bytes as *mut u64).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } == 0;
        ok.then_some(bytes / (1 << 30))
    }
    #[cfg(target_os = "linux")]
    {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb: u64 = info
            .lines()
            .find_map(|l| l.strip_prefix("MemTotal:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        Some(kb / (1 << 20))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

pub fn free_disk_gb(data: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut probe = data;
        while !probe.exists() {
            probe = probe.parent()?;
        }
        let path = std::ffi::CString::new(probe.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        (unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0)
            .then(|| (stat.f_bavail as u64 * stat.f_frsize as u64) / 1_000_000_000)
    }
    #[cfg(not(unix))]
    {
        let _ = data;
        None
    }
}
