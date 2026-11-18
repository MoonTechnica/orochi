#!/bin/sh
# The browser engine's archive and checksum for one target, named the way `orochi setup` looks
# for them (`setup::install_servo`): `orochi-servo-<version>-<target>.tar.gz` holding one
# directory of that name with `orochi-servo` in it, and the `.sha256` beside it. The engine has
# the CLI's version, so the engine and the CLI that speaks to it are always one build. Run
# after `cargo build --release --locked --manifest-path browser-servo/Cargo.toml --target
# <triple>`. The engine is Unix-only for now.
set -eu

cd "$(dirname "$0")/.."

target=${1:?usage: $0 <triple>}
. scripts/read-versions.sh
version=$cli
[ -n "$version" ] || {
	echo "cannot read the version out of Cargo.toml" >&2
	exit 1
}

name="orochi-servo-$version-$target"
rm -rf "dist/$name"
mkdir -p "dist/$name"
cp "browser-servo/target/$target/release/orochi-servo" "dist/$name/orochi-servo"
tar -C dist -czf "dist/$name.tar.gz" "$name"
rm -rf "dist/$name"

(cd dist && if command -v sha256sum >/dev/null; then
	sha256sum "$name.tar.gz"
else
	shasum -a 256 "$name.tar.gz"
fi >"$name.tar.gz.sha256")

cat "dist/$name.tar.gz.sha256"
