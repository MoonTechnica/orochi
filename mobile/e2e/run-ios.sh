#!/usr/bin/env bash
# iOS simulator E2E: two real orochi-remote instances with demo data on loopback, Metro, the dev
# build paired by deep link (xcrun simctl openurl), the flows in flows/ run by maestro.
# Needs: Xcode + a simulator, maestro and a JDK (JAVA_HOME or openjdk on PATH), and the dev build
# installed once (cd mobile/apps/app && npx expo run:ios --device <udid> --no-bundler).
# Screenshots end up in e2e/screenshots/ios-NN-*.png (maestro writes them under --test-output-dir
# and they are copied; `simctl io screenshot` hangs when pointed at an external volume).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/../.."
udid="${SIM_UDID:-$(xcrun simctl list devices available | grep 'iPhone 17 (' | tail -1 | grep -o '[0-9A-F-]\{36\}')}"
# The XCTest driver sometimes takes longer than maestro's default to come up after many flows.
export MAESTRO_DRIVER_STARTUP_TIMEOUT="${MAESTRO_DRIVER_STARTUP_TIMEOUT:-240000}"
work="$(mktemp -d)"
failed=""
[ -n "${JAVA_HOME:-}" ] || { [ -d /opt/homebrew/opt/openjdk ] && export JAVA_HOME=/opt/homebrew/opt/openjdk; }
[ -z "${JAVA_HOME:-}" ] || export PATH="$JAVA_HOME/bin:$PATH"
bin="$root/desktop/src-tauri/target/debug/orochi-remote"
pids=""
# Only what this script started is stopped: its recorded PIDs and their children, and the
# simulator only when this run booted it.
booted_before="$(xcrun simctl list devices | grep "$udid" | grep -c Booted || true)"
cleanup() { [ -n "${KEEP:-}" ] && return; for p in $pids; do for c in $(pgrep -P "$p" 2>/dev/null); do kill $(pgrep -P "$c" 2>/dev/null) "$c" 2>/dev/null || true; done; kill "$p" 2>/dev/null || true; done; [ "$booted_before" != 0 ] || xcrun simctl shutdown "$udid" || true; }
trap cleanup EXIT

case "${ONLY:-}" in office|threads) ;; *) (cd "$root/desktop/src-tauri" && cargo build --locked --bin orochi-remote) ;; esac
start_remote() { # name port extra...
  local n=$1 p=$2; shift 2
  nohup "$bin" "$@" --bind "127.0.0.1:$p" --json --keep-pairing > "$work/pair$n.json" 2> "$work/remote$n.err" < /dev/null &
  eval "pid_$n=$!"; pids="$pids $!"
  for _ in $(seq 40); do [ -s "$work/pair$n.json" ] && break; sleep 0.5; done
}
link() { tail -1 "$work/pair$1.json" | python3 -c 'import json,sys;print(json.loads(sys.stdin.read())["url"])'; }
open_link() { perl -e 'alarm 20; exec @ARGV' xcrun simctl openurl "$udid" "$(link "$1")"; sleep 5; }
threads_link='orochi://pair?host=127.0.0.1&port=47825&code=THREADS-1&name=Threads%20demo'
office_link='orochi://pair?host=127.0.0.1&port=47823&code=OFFICE-1&name=Office%20demo'
large_link='orochi://pair?host=127.0.0.1&port=47824&code=LARGE-1&name=Large%20office'
open_large() { perl -e 'alarm 20; exec @ARGV' xcrun simctl openurl "$udid" "$large_link"; sleep 5; }
open_threads() { perl -e 'alarm 20; exec @ARGV' xcrun simctl openurl "$udid" "$threads_link"; sleep 5; }
open_office() { perl -e 'alarm 20; exec @ARGV' xcrun simctl openurl "$udid" "$office_link"; sleep 5; }
flow() { # file
  maestro test --udid "$udid" --test-output-dir "$work/out" "$here/flows/$1" || failed="$failed $1"
  cp "$work"/out/*/*/takeScreenshot/*.png "$here/screenshots/" 2>/dev/null || true
  rm -rf "$work/out"
}

# ONLY=threads: just the threads flow (a mock desktop with threads: tools/mock-threads.mjs).
# ONLY=office: just the Office flows (mock desktops with the recorded and the large Office data), nothing else.
case "${ONLY:-}" in office|threads) ;; *)
start_remote 1 47821 --demo-data "$work/d1"
start_remote 2 47822 --demo-data "$work/d2"
;; esac
(cd "$here/.." && nohup node tools/mock-server.mjs --office --port 47823 --code OFFICE-1 > "$work/mock-office.log" 2>&1 < /dev/null & echo $! > "$work/mock.pid")
pids="$pids $(cat "$work/mock.pid")"
(cd "$here/.." && nohup node tools/mock-server.mjs --office-large --port 47824 --code LARGE-1 > "$work/mock-large.log" 2>&1 < /dev/null & echo $! > "$work/mock-large.pid")
pids="$pids $(cat "$work/mock-large.pid")"
(cd "$here/.." && nohup node tools/mock-threads.mjs --port 47825 --code THREADS-1 > "$work/mock-threads.log" 2>&1 < /dev/null & echo $! > "$work/mock-threads.pid")
pids="$pids $(cat "$work/mock-threads.pid")"
(cd "$here/../apps/app" && nohup npx expo start --localhost --port 8081 > "$work/metro.log" 2>&1 < /dev/null & echo $! > "$work/metro.pid")
sleep 8
pids="$pids $(cat "$work/metro.pid")"
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b

echo "$pids" > "$work/pids"
echo "work dir: $work (pids in $work/pids)"
case "${ONLY:-}" in office|threads) ;; *)
flow 1-empty.yaml
open_link 1
flow 2-main.yaml
open_link 2
flow 3-multi.yaml
# Revoke this phone on the second desktop: empty its device list and start it again.
kill "$pid_2"; sleep 1
echo '{"devices":[]}' > "$work/d2/remote/devices.json"
start_remote 2b 47822 --data "$work/d2"
flow 4-revoked.yaml
open_link 2b
flow 5-repair-unpair.yaml
flow 6-home-screen.yaml
;; esac
flow 0-reset.yaml
if [ "${ONLY:-}" != threads ]; then
open_office
flow 7-office.yaml
xcrun simctl ui "$udid" appearance dark
flow 7b-office-ja-dark.yaml
xcrun simctl ui "$udid" appearance light
flow 0-reset.yaml
open_large
flow 7c-office-large.yaml
fi
flow 0-reset.yaml
open_threads
flow 8-threads.yaml
[ -z "$failed" ] || { echo "FAILED flows:$failed"; exit 1; }
echo "screenshots in $here/screenshots, logs in $work"
