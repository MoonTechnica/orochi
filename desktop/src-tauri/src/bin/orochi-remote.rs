//! The phone's door with no window (`docs/mobile-remote-design.md`): the same server the
//! desktop's Settings → Mobile turns on, started from a terminal. It links the library only, so
//! nothing here opens a webview.
use anyhow::{Context, Result, bail};
use orochi_desktop::{remote::Remote, view::Client};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

const USAGE: &str =
    "orochi-remote [--data DIR] [--bind ADDR[:PORT]] [--pair] [--keep-pairing] [--json]
              [--demo-data DIR] [--seed-only] [--no-qr]

  --data DIR        the Orochi data directory (default: the same one the CLI and window use)
  --bind ADDR       listen here instead of the Tailscale address (OROCHI_REMOTE_BIND), e.g.
                    127.0.0.1:47821, for the iOS simulator or a test; port 0 picks a free one
  --pair            print a pairing link and QR (implied by --demo-data and --keep-pairing)
  --keep-pairing    issue a fresh code whenever the last one is used or runs out; typing
                    `pair` and Enter on stdin issues one at any time
  --json            print one JSON object per pairing on stdout instead of text and QR:
                    {\"url\",\"host\",\"port\",\"code\",\"name\",\"expires_at\",\"base\"}
  --demo-data DIR   write a demo store and repository into DIR, then serve it. Live seats
                    belong to this process, so they read as live while it runs
  --seed-only       with --demo-data: write and print where, then exit
  --no-qr           print the link only";

#[derive(Default)]
struct Args {
    data: Option<PathBuf>,
    bind: Option<String>,
    pair: bool,
    keep: bool,
    json: bool,
    demo: Option<PathBuf>,
    seed_only: bool,
    no_qr: bool,
}

fn parse() -> Result<Args> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_owned(), Some(v.to_owned())),
            _ => (arg, None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| it.next())
                .with_context(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "--data" => args.data = Some(value()?.into()),
            "--bind" => args.bind = Some(value()?),
            "--demo-data" => args.demo = Some(value()?.into()),
            "--pair" => args.pair = true,
            "--keep-pairing" => args.keep = true,
            "--json" => args.json = true,
            "--seed-only" => args.seed_only = true,
            "--no-qr" => args.no_qr = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
}

fn orochi() -> PathBuf {
    std::env::var_os("OROCHI_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("orochi")))
                .filter(|path| path.is_file())
        })
        .unwrap_or_else(|| "orochi".into())
}

fn qr(url: &str) -> Result<String> {
    use qrcode::render::unicode::Dense1x2;
    // Light modules on a dark ground, which is what a terminal usually is.
    Ok(qrcode::QrCode::new(url.as_bytes())?
        .render::<Dense1x2>()
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

fn announce(remote: &Remote, args: &Args) -> Result<i64> {
    let pairing = remote.pair()?;
    let status = remote.status();
    let address = status.address.clone().unwrap_or_default();
    let (host, port) = address.rsplit_once(':').unwrap_or((&address, ""));
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "url": pairing.url,
                "host": host,
                "port": port.parse::<u16>().unwrap_or(0),
                "code": pairing.code,
                "name": status.name,
                "id": status.id,
                "expires_at": pairing.expires_at,
                "base": format!("http://{address}"),
            })
        );
    } else {
        println!("Pairing link (valid 5 minutes, one use):");
        println!("{}", pairing.url);
        println!("\nBy hand: host {host}  port {port}  code {}", pairing.code);
        println!("Machine id: {}", status.id);
        if !args.no_qr {
            println!("\n{}", qr(&pairing.url)?);
        }
    }
    Ok(pairing.expires_at)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

async fn run(args: Args) -> Result<()> {
    let data = match (&args.demo, &args.data) {
        (Some(dir), _) => {
            let seeded = orochi_desktop::demo::seed(dir)?;
            eprintln!("demo data in {}", seeded.data.display());
            for (what, id) in &seeded.threads {
                eprintln!("  {what}: {id}");
            }
            if args.seed_only {
                println!("{}", serde_json::to_string(&seeded)?);
                return Ok(());
            }
            seeded.data
        }
        (None, Some(dir)) => dir.clone(),
        (None, None) => {
            orochi::config::Paths::resolve(
                None,
                std::env::var_os("OROCHI_DATA_DIR").map(PathBuf::from),
            )?
            .data
        }
    };
    let client = Client::open(&data)?;
    let remote = Remote::new(&data, Arc::new(Mutex::new(client)), orochi());
    let status = remote.enable().await?;
    eprintln!(
        "listening on {} as {}",
        status.address.unwrap_or_default(),
        status.name
    );
    let pairing = args.pair || args.keep || args.demo.is_some();
    let mut expires = if pairing {
        announce(&remote, &args)?
    } else {
        0
    };
    let mut devices = remote.devices().len();

    let (ask, mut asked) = tokio::sync::mpsc::unbounded_channel::<()>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            if line.trim() == "pair" && ask.send(()).is_err() {
                break;
            }
        }
    });
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            Some(()) = asked.recv() => expires = announce(&remote, &args)?,
            _ = tick.tick() => {
                let now = remote.devices().len();
                let used = now > devices;
                devices = now;
                if args.keep && (used || now_ms() >= expires) {
                    expires = announce(&remote, &args)?;
                }
            }
        }
    }
}

fn main() -> Result<()> {
    let args = parse()?;
    if let Some(bind) = &args.bind {
        // SAFETY: before any thread exists.
        unsafe { std::env::set_var("OROCHI_REMOTE_BIND", bind) };
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(args))
}
