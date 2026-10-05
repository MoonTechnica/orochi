## Install

**CLI** — macOS and Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/MoonTechnica/orochi/main/install.sh | sh
```

It picks the archive for this machine, checks it against the `.sha256` published beside it, and
puts `orochi` in `~/.local/bin`. The archives below can also be unpacked by hand.

**Desktop window** — download the bundle for this machine and open it. It carries its own
`orochi`, so the CLI is not required first.

| | |
|---|---|
| macOS | the `.dmg` — one bundle, Apple Silicon and Intel |
| Linux | the `.AppImage`, or the `.deb` |
| Windows | the `-setup.exe`, or the `.msi` |

## Before it can do anything

Orochi does not talk to a model. It chooses among the coding-agent CLIs **you** have installed
and authenticated, and drives the one it picks. Install at least one of `claude`, `codex`,
`gemini` or `antigravity` and sign in with that CLI's own login. Node.js 22 or later is needed
the first time, to fetch the ACP adapters. Then:

```sh
orochi config init
orochi agents        # what was found, and what is still missing
```

## What is not covered on each platform

- **Windows: the window only.** The terminal console is not ported — `chat/term.rs` reads no
  keyboard input outside Unix, so `orochi` run in a Windows terminal would accept nothing, and
  Esc and Ctrl-C would not interrupt a turn. The window does not go through that path and is
  unaffected. No CLI archive is published for Windows for this reason; the installer carries the
  binary the window runs. Process supervision is also weaker there: a lease whose owner died is
  not reclaimed.
- **Linux:** built against glibc 2.35 (Ubuntu 22.04), so an older distribution will not run it.
- **The window's appearance has not been checked against a screen.** Its behavior is covered by
  tests over fixture stores and recorded view output; how it looks is not.
