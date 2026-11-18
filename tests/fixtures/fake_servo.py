#!/usr/bin/env python3
"""An `orochi-servo` that speaks the engine protocol (docs/servo-browser-design.md §3.2) on
stdin and stdout without an engine, so the broker's side can be tested without Servo. Every
command it receives is appended to `fake-servo.log` beside the profile directory, and its pid
is written to `fake-servo.pid`, so a test can read what the broker asked and whether the
process ended with its owner.

Special addresses: http://crash.test/ reports a crash, http://dialog.test/ opens an alert,
http://popup.test/ asks for a new window."""
import json
import os
import sys

args = sys.argv[1:]
def option(name, default):
    return args[args.index(name) + 1] if name in args else default

profile = option("--profile", None)
width, height, ratio = int(option("--width", "1024")), int(option("--height", "768")), 1.0
here = os.path.dirname(profile) if profile else os.getcwd()
log = open(os.path.join(here, "fake-servo.log"), "a", buffering=1)
open(os.path.join(here, "fake-servo.pid"), "w").write(str(os.getpid()))
open(os.path.join(here, "fake-servo.argv"), "w").write(json.dumps(args))
out = sys.stdout.buffer
seq = 0
history, at = [], -1


def send(event):
    out.write(json.dumps(event).encode() + b"\n")
    out.flush()


def frame():
    global seq
    seq += 1
    body = b"\xff\xd8FAKE-%d-%dx%d\xff\xd9" % (seq, width, height)
    send({"event": "frame", "seq": seq, "width": int(width * ratio), "height": int(height * ratio), "len": len(body)})
    out.write(body)
    out.flush()


def show(url):
    send({"event": "url", "url": url})
    send({"event": "loading", "state": "started"})
    send({"event": "title", "title": "fake: " + url})
    send({"event": "history", "back": at > 0, "forward": at < len(history) - 1})
    send({"event": "loading", "state": "complete"})
    frame()
    if url.endswith("/redirect"):
        send({"event": "url", "url": "https://evil.example/landing"})
    if url == "http://crash.test/":
        send({"event": "crashed"})
    if url == "http://dialog.test/":
        send({"event": "dialog", "kind": "alert", "message": "hello", "default": ""})
    if url == "http://popup.test/":
        send({"event": "popup", "url": "http://popup.test/child"})


def go(url):
    global at
    if not url.startswith(("http://", "https://")) and url != "about:blank":
        send({"event": "refused", "url": url})
        return
    del history[at + 1:]
    history.append(url)
    at = len(history) - 1
    show(url)


if os.path.exists(os.path.join(here, "fake-servo.slow")):
    # The real engine drops what it is sent before its first page has loaded.
    import select, time
    end = time.time() + 0.5
    while time.time() < end:
        if select.select([sys.stdin], [], [], 0.05)[0]:
            sys.stdin.readline()
send({"event": "ready"})
for line in sys.stdin:
    command = json.loads(line)
    log.write(json.dumps(command) + "\n")
    cmd = command.get("cmd")
    if cmd in ("open", "navigate"):
        go(command["url"])
    elif cmd == "back" and at > 0:
        at -= 1
        show(history[at])
    elif cmd == "forward" and at < len(history) - 1:
        at += 1
        show(history[at])
    elif cmd == "reload" and history:
        show(history[at])
    elif cmd == "resize":
        width, height, ratio = int(command["width"]), int(command["height"]), float(command["ratio"])
        frame()
    elif cmd == "frames":
        if command.get("on"):
            frame()
    elif cmd == "dialog":
        send({"event": "dialog_closed"})
    elif cmd == "input" and command.get("type") == "text":
        send({"event": "title", "title": "typed: " + command.get("text", "")})
        frame()
    elif cmd == "eval":
        script = command.get("script", "")
        if "JSON.stringify({ text: out" in script:
            value = json.dumps({"text": '- link "Docs" [ref=e1]\n- textbox "Search" [ref=e2]\n', "refs": 2})
        elif "getBoundingClientRect" in script:
            value = json.dumps({"x": 10, "y": 20, "width": 100, "height": 40, "tag": "A"})
        elif "el.focus()" in script:
            value = json.dumps({"ok": True})
        elif "innerText.includes" in script:
            value = True
        elif "throw" in script:
            send({"event": "eval", "id": command["id"], "ok": False, "error": "boom"})
            continue
        else:
            value = "eval:" + script[:40]
        send({"event": "eval", "id": command["id"], "ok": True, "value": value})
    elif cmd == "screenshot":
        body = b"\x89PNG\r\n\x1a\nFAKE" + json.dumps(
            {k: command[k] for k in ("x", "y", "width", "height") if k in command}).encode()
        send({"event": "screenshot", "id": command["id"], "len": len(body)})
        out.write(body)
        out.flush()
    elif cmd == "shutdown":
        break
