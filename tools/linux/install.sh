#!/bin/bash
# Install from a checkout on native Linux or WSL2. Builds stay on the Linux filesystem.
set -euo pipefail
desktop=0
case "${1:-}" in --desktop) desktop=1;; '') ;; *) echo 'usage: install.sh [--desktop]' >&2; exit 2;; esac
[ "$(uname -s)" = Linux ] || { echo 'This installer requires Linux or WSL2.' >&2; exit 1; }
[ "$(id -u)" != 0 ] || { echo 'Run as your normal Linux user; sudo is used only for package installation.' >&2; exit 1; }
source_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
if [ ! -d /run/systemd/system ]; then
  echo 'systemd is required. On WSL2, enable [boot] systemd=true in /etc/wsl.conf, restart this distribution from Windows, and rerun -Setup.' >&2
  exit 1
fi
if command -v apt-get >/dev/null; then
  sudo apt-get update -qq
  sudo apt-get install -y build-essential curl git pkg-config libssl-dev rsync
  if [ "$desktop" = 1 ]; then
    sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev file patchelf
  fi
elif command -v dnf >/dev/null; then
  sudo dnf install -y gcc gcc-c++ make curl git pkgconf-pkg-config openssl-devel rsync
  if [ "$desktop" = 1 ]; then sudo dnf install -y webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel libxdo-devel file patchelf; fi
elif command -v pacman >/dev/null; then
  sudo pacman -S --needed --noconfirm base-devel curl git openssl rsync
  if [ "$desktop" = 1 ]; then sudo pacman -S --needed --noconfirm webkit2gtk-4.1 libappindicator-gtk3 librsvg xdotool file patchelf; fi
else
  echo 'Automatic build dependencies support apt, dnf and pacman. Install Rust 1.96, Git and the Tauri Linux dependencies, then build with cargo.' >&2
  exit 1
fi
if [ ! -x "$HOME/.cargo/bin/rustup" ]; then
  installer=$(mktemp)
  trap 'rm -f "$installer"' EXIT
  curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$installer"
  sh "$installer" -y --profile minimal --default-toolchain 1.96.0
else
  "$HOME/.cargo/bin/rustup" toolchain install 1.96.0 --profile minimal
fi
export PATH="$HOME/.cargo/bin:$PATH"
build_dir="$HOME/.local/share/orochi/build/source"
mkdir -p "$build_dir" "$HOME/.local/bin"
if [ "$source_dir" != "$build_dir" ]; then
  rsync -a --exclude target --exclude node_modules --exclude .git --exclude .orochi \
    --exclude .env --exclude '.env.*' --exclude .agents --exclude .codex --exclude .aws \
    --include '/Cargo.toml' --include '/Cargo.lock' --include '/src/***' \
    --include '/policies/***' --include '/desktop/***' --exclude '*' "$source_dir/" "$build_dir/"
fi
cargo +1.96.0 build --locked --release --manifest-path "$build_dir/Cargo.toml"
install -m 0755 "$build_dir/target/release/orochi" "$HOME/.local/bin/orochi"
if [ "$desktop" = 1 ]; then
  cargo +1.96.0 build --locked --release --manifest-path "$build_dir/desktop/src-tauri/Cargo.toml"
  install -m 0755 "$build_dir/desktop/src-tauri/target/release/orochi-desktop" "$HOME/.local/bin/orochi-desktop"
fi
"$HOME/.local/bin/orochi" init
echo 'Installed Orochi under ~/.local/bin. On Windows use tools/windows/orochi.ps1; add ~/.local/bin to PATH on native Linux.'
