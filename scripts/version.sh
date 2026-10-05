#!/bin/sh
# One version for the CLI and the window. It lives in three files and two lockfiles, so setting it
# by hand drifts: the tag would say one thing, `orochi --version` another and the About box a
# third. Run this, review the diff, then tag.
#
# Everything here is `awk`, not `sed -i`: BSD and GNU sed disagree about both `-i` and the `0,/re/`
# address, and the version that got written on the wrong one was written nowhere at all, silently.
# Which is why this reads every file back at the end and refuses to report a version it did not set.
set -eu

cd "$(dirname "$0")/.."

version=${1:-}
case $version in
[0-9]*.[0-9]*.[0-9]*) ;;
*)
	echo "usage: $0 <major.minor.patch>" >&2
	exit 2
	;;
esac

write() {
	awk -v v="$version" "$2" "$1" >"$1.new"
	mv "$1.new" "$1"
}

toml='
	/^\[/ { section = $0 }
	section == "[package]" && /^version = "/ && !done { print "version = \"" v "\""; done = 1; next }
	{ print }
'
json='
	!done && /"version":/ { sub(/"version":[ \t]*"[^"]*"/, "\"version\": \"" v "\""); done = 1 }
	{ print }
'

write Cargo.toml "$toml"
write desktop/src-tauri/Cargo.toml "$toml"
write desktop/src-tauri/tauri.conf.json "$json"

# The lockfiles carry the member versions too, and every command runs `--locked`.
cargo update --workspace --quiet
(cd desktop/src-tauri && cargo update --workspace --quiet)

cargo metadata --locked --format-version 1 >/dev/null
(cd desktop/src-tauri && cargo metadata --locked --format-version 1 >/dev/null)

. scripts/read-versions.sh
for name in cli desktop bundle; do
	eval "got=\$$name"
	[ "$got" = "$version" ] || {
		echo "$name still reads $got, not $version" >&2
		exit 1
	}
done

git --no-pager diff --stat -- Cargo.toml Cargo.lock desktop/src-tauri
echo
echo "set to $version. review, commit, then: git tag v$version && git push origin v$version"
