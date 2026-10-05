#!/bin/sh
# One archive and one checksum per target, named the way `install.sh` looks for them. Run after
# `cargo build --release --target <triple>`.
set -eu

cd "$(dirname "$0")/.."

target=${1:?usage: $0 <triple>}
. scripts/read-versions.sh
version=$cli
[ -n "$version" ] || {
	echo "cannot read the version out of Cargo.toml" >&2
	exit 1
}

name="orochi-$version-$target"
rm -rf "dist/$name"
mkdir -p "dist/$name"
cp "target/$target/release/orochi" "dist/$name/orochi"
cp README.md LICENSE "dist/$name/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
rm -rf "dist/$name"

# `shasum` is on macOS and on the Linux runners; `sha256sum` is not on macOS.
(cd dist && if command -v sha256sum >/dev/null; then
	sha256sum "$name.tar.gz"
else
	shasum -a 256 "$name.tar.gz"
fi >"$name.tar.gz.sha256")

cat "dist/$name.tar.gz.sha256"
