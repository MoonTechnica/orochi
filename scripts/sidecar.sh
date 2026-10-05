#!/bin/sh
# Puts the CLI where the window can find it. The app looks for `orochi` next to its own
# executable (desktop/src-tauri/src/main.rs), because a window opened from Finder has no shell
# `PATH`; Tauri's `externalBin` is what places a sidecar there, and it wants the file named for
# the target it was built for. Without this the desktop crate does not even `cargo check`.
set -eu

cd "$(dirname "$0")/.."

host=$(rustc -vV | sed -n 's/^host: //p')
profile=debug
target=$host

while [ $# -gt 0 ]; do
	case $1 in
	--release) profile=release ;;
	--target)
		target=${2:?--target needs a triple}
		shift
		;;
	*)
		echo "usage: $0 [--release] [--target <triple>]" >&2
		exit 2
		;;
	esac
	shift
done

flags=--locked
if [ "$profile" = release ]; then
	flags="$flags --release"
fi

# `cargo build --target <host>` would move the output out of `target/<profile>/`, where the demo
# and the tests expect to find it, and build the whole tree a second time.
build() {
	if [ "$1" = "$host" ]; then
		# shellcheck disable=SC2086
		cargo build $flags
		echo "target/$profile/orochi"
	else
		# shellcheck disable=SC2086
		cargo build $flags --target "$1"
		echo "target/$1/$profile/orochi"
	fi
}

mkdir -p desktop/src-tauri/binaries
out="desktop/src-tauri/binaries/orochi-$target"

# A universal app needs a universal sidecar: a bundle the loader opens on either architecture
# cannot carry a CLI built for one of them.
if [ "$target" = universal-apple-darwin ]; then
	arm=$(build aarch64-apple-darwin)
	intel=$(build x86_64-apple-darwin)
	lipo -create -output "$out" "$arm" "$intel"
	# The bundler has been seen to look for the universal name and to lipo the two single
	# architectures itself. Leave both, so it finds whichever it asks for.
	cp "$arm" "desktop/src-tauri/binaries/orochi-aarch64-apple-darwin"
	cp "$intel" "desktop/src-tauri/binaries/orochi-x86_64-apple-darwin"
	echo "$out"
	exit 0
fi

case $target in
*-windows-*) exe=.exe ;;
*) exe= ;;
esac

built=$(build "$target")
cp "$built$exe" "$out$exe"
echo "$out$exe"
