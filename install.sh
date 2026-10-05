#!/bin/sh
# Installs the `orochi` CLI from the published release for this machine.
#
#   curl -fsSL https://raw.githubusercontent.com/MoonTechnica/orochi/main/install.sh | sh
#
# OROCHI_VERSION picks a release other than the latest; OROCHI_BIN_DIR picks where it lands.
# The desktop window is a separate download and carries its own copy of this binary.
set -eu

repo=MoonTechnica/orochi
bin_dir=${OROCHI_BIN_DIR:-$HOME/.local/bin}

say() { printf '%s\n' "$*"; }
die() {
	printf '%s\n' "$*" >&2
	exit 1
}

case $(uname -s) in
Darwin) os=apple-darwin ;;
Linux) os=unknown-linux-gnu ;;
*) die "no published build for $(uname -s). The desktop window is available for Windows; the CLI's terminal console is not ported there." ;;
esac

case $(uname -m) in
arm64 | aarch64) arch=aarch64 ;;
x86_64 | amd64) arch=x86_64 ;;
*) die "no published build for $(uname -m)" ;;
esac

target="$arch-$os"

command -v curl >/dev/null || die "curl is required"
command -v tar >/dev/null || die "tar is required"

version=${OROCHI_VERSION:-}
if [ -z "$version" ]; then
	version=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" |
		sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -n 1)
	[ -n "$version" ] || die "cannot tell which release is the latest; set OROCHI_VERSION"
fi
version=${version#v}

name="orochi-$version-$target"
base="https://github.com/$repo/releases/download/v$version/$name.tar.gz"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading orochi $version for $target"
curl -fsSL -o "$tmp/$name.tar.gz" "$base" ||
	die "no archive at $base"
curl -fsSL -o "$tmp/$name.tar.gz.sha256" "$base.sha256" ||
	die "the archive is published without a checksum; refusing to install it"

# What was downloaded is checked before anything is unpacked, not after it is on the PATH.
(
	cd "$tmp"
	if command -v sha256sum >/dev/null; then
		sha256sum -c "$name.tar.gz.sha256"
	elif command -v shasum >/dev/null; then
		shasum -a 256 -c "$name.tar.gz.sha256"
	else
		die "neither sha256sum nor shasum is available to check the download"
	fi
) >/dev/null || die "the download does not match its published checksum"

tar -C "$tmp" -xzf "$tmp/$name.tar.gz"
mkdir -p "$bin_dir"
# Replacing the file in place would rewrite a binary that is running; a rename does not.
mv "$tmp/$name/orochi" "$tmp/orochi.new"
chmod +x "$tmp/orochi.new"
mv "$tmp/orochi.new" "$bin_dir/orochi"

say "installed $bin_dir/orochi"

case ":$PATH:" in
*":$bin_dir:"*) ;;
*) say "$bin_dir is not on your PATH. Add it, or run $bin_dir/orochi by its full path." ;;
esac

say ""
say "orochi drives the coding-agent CLIs you have installed and signed in to; it does not"
say "talk to a model itself. Install at least one of claude, codex, gemini or antigravity,"
say "then:"
say ""
say "    orochi config init"
say "    orochi agents"
