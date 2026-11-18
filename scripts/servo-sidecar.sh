#!/bin/sh
# Puts the browser engine where the window finds it: next to the app's own executable, as
# `orochi-servo` (the broker the app runs looks beside its executable, `browser::engine::
# find_servo`). Servo is built apart from the CLI, in its own workspace, and takes minutes the
# CLI's checks must never wait for, so the bundle that carries it is a separate config:
# `desktop/src-tauri/tauri.servo.conf.json`, passed as `tauri build --config`.
# Servo's engine is Unix-only for now (the browser is `#[cfg(unix)]`).
set -eu

cd "$(dirname "$0")/.."

host=$(rustc -vV | sed -n 's/^host: //p')
target=$host

while [ $# -gt 0 ]; do
	case $1 in
	--target)
		target=${2:?--target needs a triple}
		shift
		;;
	*)
		echo "usage: $0 [--target <triple>]" >&2
		exit 2
		;;
	esac
	shift
done

case $target in
*-windows-*)
	echo "orochi-servo is not built for Windows" >&2
	exit 1
	;;
esac

manifest=browser-servo/Cargo.toml
build() {
	if [ "$1" = "$host" ]; then
		cargo build --release --locked --manifest-path "$manifest" >&2
		echo "browser-servo/target/release/orochi-servo"
	else
		cargo build --release --locked --manifest-path "$manifest" --target "$1" >&2
		echo "browser-servo/target/$1/release/orochi-servo"
	fi
}

mkdir -p desktop/src-tauri/binaries
out="desktop/src-tauri/binaries/orochi-servo-$target"

if [ "$target" = universal-apple-darwin ]; then
	arm=$(build aarch64-apple-darwin)
	intel=$(build x86_64-apple-darwin)
	lipo -create -output "$out" "$arm" "$intel"
	cp "$arm" "desktop/src-tauri/binaries/orochi-servo-aarch64-apple-darwin"
	cp "$intel" "desktop/src-tauri/binaries/orochi-servo-x86_64-apple-darwin"
	echo "$out"
	exit 0
fi

built=$(build "$target")
cp "$built" "$out"
echo "$out"
