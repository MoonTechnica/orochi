#!/usr/bin/env python3
"""Drives orochi-servo over its protocol (docs/servo-browser-design.md §3.2) against local
pages and prints what happened. By hand, not part of the test suite.

    python3 browser-servo/tools/drive.py [--binary PATH] [--out DIR] [--only NAME,...]
"""
import argparse, http.server, json, os, queue, shutil, subprocess, sys, tempfile, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_BINARY = os.path.join(HERE, "..", "target", "release", "orochi-servo")

FORM = """<!doctype html><meta charset=utf-8><title>form</title>
<style>body{margin:0;font:16px sans-serif}
#t{position:absolute;left:20px;top:20px;width:300px;height:30px}
#b{position:absolute;left:20px;top:70px;width:120px;height:36px}
#c{position:absolute;left:20px;top:130px;width:300px;height:40px;border:1px solid #888}
#box{position:absolute;left:400px;top:20px;width:200px;height:200px;background:#c33}
</style>
<input id=t><button id=b>go</button><div id=c contenteditable></div><div id=box></div><a id=l style="position:absolute;left:400px;top:300px;font-size:30px" href="file:///etc/hosts">link</a>
<script>
let flip=0;box.addEventListener('mousedown',()=>{flip^=1;box.style.background=flip?'#3c3':'#c33'});
var ev=[],al=0;function log(n){return e=>{ev.push(n+':'+(e.data||''));mirror()}}
function mirror(){document.title='in:'+t.value+'|ce:'+c.textContent+'|ae:'+(document.activeElement||{}).id+'|ce_editable:'+c.isContentEditable+'|al:'+al+'|ev:'+ev.join(',')}
for(const n of ['compositionstart','compositionupdate','compositionend'])for(const x of [t,c])x.addEventListener(n,log(n.slice(11)));
t.addEventListener('input',mirror);c.addEventListener('input',mirror);setInterval(mirror,100);
b.addEventListener('click',()=>{box.style.background='#3c3';alert('hello '+t.value);al=1;mirror()})
</script>"""

ANIM = """<!doctype html><meta charset=utf-8><title>anim</title>
<style>body{margin:0;overflow:hidden}#a{position:absolute;top:100px;width:120px;height:120px;background:#36c}
#s{position:absolute;top:300px;left:0;font:20px monospace}</style>
<div id=a></div><div id=s></div>
<script>let n=0;(function f(){n++;a.style.left=(n*4%900)+'px';s.textContent='frame '+n;
document.title='raf:'+n;requestAnimationFrame(f)})()</script>"""

SCROLL = """<!doctype html><meta charset=utf-8><title>scroll</title>
<body style="margin:0"><div style="height:6000px;background:linear-gradient(#fff,#06c)">tall</div>
<script>addEventListener('scroll',()=>document.title='scroll:'+Math.round(scrollY))</script>"""

COOKIE = """<!doctype html><meta charset=utf-8><title>cookie</title>
<script>
const n=(+localStorage.getItem('n')||0)+1;localStorage.setItem('n',n);
document.title='cookie:'+document.cookie+'|n:'+n;
</script>"""


class Handler(http.server.BaseHTTPRequestHandler):
    seen = []

    def log_message(self, *a):
        pass

    def do_GET(self):
        pages = {"/form": FORM, "/anim": ANIM, "/scroll": SCROLL, "/cookie": COOKIE}
        body = pages.get(self.path.split("?")[0])
        if body is None:
            self.send_response(404); self.end_headers(); return
        Handler.seen.append((self.path, self.headers.get("Cookie")))
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        if self.path.startswith("/cookie") and "sid=" not in (self.headers.get("Cookie") or ""):
            self.send_header("Set-Cookie", "sid=abc123; Max-Age=3600; Path=/")
        self.end_headers()
        self.wfile.write(body.encode())


def serve():
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_address[1]}"


class Engine:
    def __init__(self, binary, profile=None, width=1024, height=768, quality=70, out=None):
        args = [binary, "--width", str(width), "--height", str(height), "--quality", str(quality)]
        if profile:
            args += ["--profile", profile]
        self.out = out
        self.proc = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=open(os.path.join(out or tempfile.gettempdir(), "engine.err"), "ab"))
        self.events = queue.Queue()
        self.frames = []  # (time, seq, width, height, bytes)
        self.title = self.url = ""
        self.history = {}
        self.lock = threading.Lock()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        out = self.proc.stdout
        while True:
            line = out.readline()
            if not line:
                self.events.put({"event": "__eof"})
                return
            try:
                event = json.loads(line)
            except ValueError:
                self.events.put({"event": "__garbage", "line": line[:200].decode("utf-8", "replace")})
                continue
            if event.get("event") == "frame":
                data = out.read(event["len"])
                with self.lock:
                    self.frames.append((time.time(), event["seq"], event["width"], event["height"], data))
                if self.out and len(self.frames) % 25 == 1:
                    with open(os.path.join(self.out, f"frame-{event['seq']}.jpg"), "wb") as f:
                        f.write(data)
                continue
            if event.get("event") == "title":
                self.title = event["title"]
            if event.get("event") == "url":
                self.url = event["url"]
            self.events.put(event)

    def send(self, **command):
        self.proc.stdin.write((json.dumps(command) + "\n").encode())
        self.proc.stdin.flush()

    def wait(self, predicate, timeout=20):
        end = time.time() + timeout
        seen = []
        while time.time() < end:
            try:
                event = self.events.get(timeout=0.1)
            except queue.Empty:
                continue
            seen.append(event)
            if predicate(event):
                return event
        return None

    def wait_title(self, text, timeout=15):
        end = time.time() + timeout
        while time.time() < end:
            if text in self.title:
                return True
            time.sleep(0.05)
        return False

    def open(self, url, title=None, timeout=30):
        self.send(cmd="open", url=url)
        return self.wait_title(title, timeout) if title else bool(self.wait(
            lambda e: e["event"] == "loading" and e["state"] == "complete", timeout))

    def click(self, x, y, button="left"):
        for kind, extra in (("mouseMoved", {}), ("mousePressed", {"clickCount": 1, "buttons": 1}),
                            ("mouseReleased", {"clickCount": 1})):
            self.send(cmd="input", type="mouse", kind=kind, x=x, y=y, button=button, **extra)

    def type(self, text):
        for ch in text:
            self.send(cmd="input", type="key", kind="keyDown", key=ch, code="", text=ch)
            self.send(cmd="input", type="key", kind="keyUp", key=ch, code="")

    def rss_kb(self):
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(self.proc.pid)], capture_output=True, text=True)
        return int(out.stdout.strip() or 0)

    def close(self, how="shutdown"):
        start = time.time()
        if self.out and self.frames:
            with open(os.path.join(self.out, f"last-{self.proc.pid}.jpg"), "wb") as f:
                f.write(self.frames[-1][4])
        if how == "shutdown":
            self.send(cmd="shutdown")
        else:
            self.proc.stdin.close()
        try:
            code = self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill(); code = "killed"
        return code, time.time() - start


def settle(engine, seconds):
    time.sleep(seconds)


def t_basics(ctx):
    e = Engine(ctx.binary, out=ctx.out)
    r = {}
    ready = e.wait(lambda x: x["event"] == "ready", 30)
    r["ready"] = bool(ready)
    r["load_form"] = e.open(ctx.base + "/form", title="form")
    time.sleep(0.5)
    r["url_event"] = e.url
    r["first_frame_size"] = [e.frames[-1][2], e.frames[-1][3], len(e.frames[-1][4])] if e.frames else None
    r["frames_after_load"] = len(e.frames)
    # click the input, type, check the page saw it
    e.click(100, 35)
    e.type("hello")
    r["typed_into_input"] = e.wait_title("in:hello")
    # click the button -> alert
    e.send(cmd="input", type="key", kind="rawKeyDown", key="Tab", code="Tab", vk=9)
    e.send(cmd="input", type="key", kind="keyUp", key="Tab", code="Tab", vk=9)
    start = time.time()
    n = len(e.frames)
    e.click(80, 88)
    dialog = e.wait(lambda x: x["event"] == "dialog", 10)
    r["dialog_event"] = dialog
    if dialog:
        e.send(cmd="dialog", accept=True)
        r["dialog_closed"] = bool(e.wait(lambda x: x["event"] == "dialog_closed", 5))
        r["title_after_alert"] = e.wait_title("al:1", 10)
    # click -> next frame latency (box turns green on click)
    r["rss_kb"] = e.rss_kb()
    code, secs = e.close("shutdown")
    r["shutdown"] = [code, round(secs, 2)]
    return r


def t_latency(ctx):
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/form", title="form")
    time.sleep(1.0)
    samples = []
    for i in range(10):
        time.sleep(0.4)
        before = len(e.frames)
        sent = time.time()
        e.click(500, 100)  # the page flips the box's colour on mousedown
        end = sent + 3
        while time.time() < end and len(e.frames) == before:
            time.sleep(0.002)
        if len(e.frames) > before:
            samples.append(round((e.frames[before][0] - sent) * 1000))
    code, secs = e.close()
    return {"click_to_frame_ms": samples, "median_ms": sorted(samples)[len(samples) // 2] if samples else None}


def t_fps(ctx):
    r = {}
    for ratio in (1, 2):
        e = Engine(ctx.binary, out=ctx.out)
        e.wait(lambda x: x["event"] == "ready", 30)
        e.send(cmd="resize", width=1024, height=768, ratio=ratio)
        e.open(ctx.base + "/anim", title="raf:")
        time.sleep(1.5)
        a = len(e.frames); t0 = time.time()
        time.sleep(4)
        n = len(e.frames) - a
        with e.lock:
            recent = e.frames[-n:] if n else []
        r[f"ratio{ratio}"] = {
            "fps": round(n / (time.time() - t0), 1),
            "frame": [recent[-1][2], recent[-1][3]] if recent else None,
            "avg_jpeg_kb": round(sum(len(f[4]) for f in recent) / max(n, 1) / 1024, 1),
            "rss_mb": round(e.rss_kb() / 1024),
        }
        e.close()
    # idle cost: a static page, no frames wanted
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/scroll", title="scroll")
    time.sleep(2)
    pid = e.proc.pid
    cpu = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    time.sleep(10)
    cpu2 = subprocess.run(["ps", "-o", "time=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    r["idle_cpu_time_over_10s"] = [cpu, cpu2]
    r["idle_frames_in_10s"] = len(e.frames)
    e.close()
    return r


def t_scroll(ctx):
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/scroll", title="scroll")
    time.sleep(1)
    e.send(cmd="input", type="mouse", kind="mouseWheel", x=200, y=200, deltaX=0, deltaY=300, button="none")
    ok = e.wait_title("scroll:", 10)
    first = e.title
    e.close()
    return {"scrolled": ok, "title": first}


def t_ime(ctx):
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/form", title="form")
    time.sleep(0.5)
    r = {}
    e.click(100, 35)
    time.sleep(0.2)
    e.send(cmd="input", type="ime", text="に", commit=False)
    e.send(cmd="input", type="ime", text="にほん", commit=False)
    r["input_compstart_seen"] = e.wait_title("ev:start", 5)
    e.send(cmd="input", type="ime", text="日本", commit=True)
    r["input_committed"] = e.wait_title("in:日本", 5)
    r["title_after_input"] = e.title
    e.click(100, 150)
    time.sleep(0.2)
    e.send(cmd="input", type="ime", text="にほん", commit=False)
    e.send(cmd="input", type="ime", text="日本語", commit=True)
    r["contenteditable_committed"] = e.wait_title("ce:日本語", 5)
    r["title_after_ce"] = e.title
    e.send(cmd="input", type="text", text="abc")
    r["text_command"] = e.wait_title("abc", 5)
    e.close()
    return r


def t_typing(ctx):
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/form", title="form")
    time.sleep(0.5)
    r = {}
    e.click(100, 35)
    time.sleep(0.2)
    e.send(cmd="input", type="text", text="abc")
    r["text_command_in_input"] = e.wait_title("in:abc", 5)
    e.click(100, 150)
    time.sleep(0.3)
    e.type("xyz")
    r["keys_in_contenteditable"] = e.wait_title("ce:xyz", 5)
    r["title"] = e.title
    e.click(100, 150)
    e.send(cmd="input", type="text", text="pq")
    r["text_command_in_contenteditable"] = e.wait_title("pq", 5)
    r["title_end"] = e.title
    e.close()
    return r


def t_policy(ctx):
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    r = {}
    for bad in ("file:///etc/hosts", "javascript:alert(1)", "data:text/html,hi", "about:config"):
        e.send(cmd="navigate", url=bad)
        r[bad] = bool(e.wait(lambda x: x["event"] == "refused", 5))
    r["http_allowed"] = e.open(ctx.base + "/form", title="form")
    time.sleep(0.5)
    e.click(420, 315)
    r["link_to_file_refused_in_page"] = bool(e.wait(lambda x: x["event"] == "refused", 5))
    time.sleep(0.5)
    r["url_still"] = e.url
    e.close()
    return r


def t_persist(ctx):
    profile = tempfile.mkdtemp(prefix="orochi-servo-profile-")
    r = {}
    Handler.seen.clear()
    for run in (1, 2):
        e = Engine(ctx.binary, profile=profile, out=ctx.out)
        e.wait(lambda x: x["event"] == "ready", 30)
        e.open(ctx.base + "/cookie", title="cookie:")
        time.sleep(1)
        r[f"run{run}_title"] = e.title
        code, secs = e.close("shutdown")
        r[f"run{run}_exit"] = [code, round(secs, 2)]
        r[f"run{run}_files"] = sorted(os.listdir(profile))
    r["server_saw_cookie_header"] = [c for p, c in Handler.seen]
    shutil.rmtree(profile, ignore_errors=True)
    return r


def t_exit(ctx):
    r = {}
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/form", title="form")
    code, secs = e.close("eof")
    r["stdin_eof"] = [code, round(secs, 2)]
    e = Engine(ctx.binary, out=ctx.out)
    e.wait(lambda x: x["event"] == "ready", 30)
    e.open(ctx.base + "/anim", title="raf:")
    code, secs = e.close("shutdown")
    r["shutdown_while_animating"] = [code, round(secs, 2)]
    garbage = []
    while not e.events.empty():
        ev = e.events.get()
        if ev["event"] == "__garbage":
            garbage.append(ev)
    r["non_protocol_stdout_lines"] = len(garbage)
    return r


TESTS = {f.__name__[2:]: f for f in (t_basics, t_latency, t_fps, t_scroll, t_ime, t_typing, t_policy, t_persist, t_exit)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=DEFAULT_BINARY)
    parser.add_argument("--out", default=tempfile.mkdtemp(prefix="orochi-servo-drive-"))
    parser.add_argument("--only", default="")
    ctx = parser.parse_args()
    os.makedirs(ctx.out, exist_ok=True)
    server, ctx.base = serve()
    names = [n for n in ctx.only.split(",") if n] or list(TESTS)
    print("output:", ctx.out)
    for name in names:
        started = time.time()
        try:
            result = TESTS[name](ctx)
        except Exception as error:  # noqa: BLE001
            result = {"error": repr(error)}
        print(f"== {name} ({time.time() - started:.1f}s)")
        print(json.dumps(result, ensure_ascii=False, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
