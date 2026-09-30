#!/usr/bin/python3
"""Powers the sandbox VM off once nothing has used it for a while, run every minute by a
systemd timer that `orochi sandbox up` installs. Orochi starts the VM again when a run, or any
`orochi sandbox` command that needs it, finds it stopped, so stopping is never a loss.

Used means an agent's work is alive: any process in any sandbox carrying `OROCHI_SANDBOX`,
which Orochi sets on everything it starts there (the agent, its checks, `enter`'s shell) and
which every descendant inherits, including work an agent left running in the background after
its turn. An Incus operation in progress (an image being built or published) is use too. A connection from the Mac through the gateway or a focused port is use too, so a page
someone is looking at stays up. A sandbox that merely runs (Supabase idling) is not use. When
it cannot tell, it counts as use: stopping the VM is the one mistake that loses work.

What it decided is written to /run for `orochi sandbox status`. The clock starts at boot.
"""
import json
import os
import subprocess
import sys
import time

MARK = os.environ.get("SBX_IDLE_MARK", "/run/orochi-sandbox-active")
REPORT = os.environ.get("SBX_IDLE_REPORT", "/run/orochi-sandbox-idle.json")
CONF = os.environ.get("SBX_IDLE_CONF", "/etc/orochi-sandbox.conf")
PROC = os.environ.get("SBX_IDLE_PROC", "/proc")
TAG = b"OROCHI_SANDBOX="


def settings(path=CONF):
    values = {}
    try:
        with open(path) as f:
            for line in f:
                key, _, value = line.strip().partition("=")
                if key:
                    values[key] = value
    except FileNotFoundError:
        pass
    return int(values.get("SBX_VM_IDLE_MINUTES", "30")), int(values.get("SBX_GATEWAY_PORT", "1355"))


def agent_work(proc=PROC):
    """Live processes Orochi started in a sandbox, or that such a process started, as
    (pid, command name, sandbox). A process that exits while it is read is simply not there."""
    found = []
    for entry in sorted((e for e in os.listdir(proc) if e.isdigit()), key=int):
        try:
            with open(os.path.join(proc, entry, "environ"), "rb") as f:
                environ = f.read()
            with open(os.path.join(proc, entry, "stat"), "rb") as f:
                state = f.read().rsplit(b")", 1)[-1].split()[0]
            with open(os.path.join(proc, entry, "comm"), "rb") as f:
                name = f.read().strip().decode(errors="replace")
        except OSError:
            continue
        if state == b"Z":
            continue
        for variable in environ.split(b"\0"):
            if variable.startswith(TAG):
                found.append((int(entry), name, variable[len(TAG):].decode(errors="replace")))
                break
    return found


def incus_busy(operations):
    """What Incus is doing right now (building an image, publishing it, copying a volume, a
    command inside): work Orochi started that no sandbox process may carry the mark for."""
    return sorted({
        op.get("description", "?")
        for op in (operations or {}).get("running", []) or []
        if op.get("status") == "Running"
    })


def focused_ports(instances):
    """The loopback ports focus added (proxy devices), which a Mac client connects through."""
    ports = set()
    for instance in instances or []:
        for name, device in (instance.get("devices") or {}).items():
            if name.startswith("sbx-port-") and device.get("type") == "proxy":
                port = device.get("listen", "").rsplit(":", 1)[-1]
                if port.isdigit():
                    ports.add(int(port))
    return ports


def connected(ss_output, ports):
    """Whether an established connection has one of `ports` as its local port."""
    for line in ss_output.splitlines():
        fields = line.split()
        if len(fields) < 4:
            continue
        local = fields[2] if fields[0].isdigit() else fields[3]
        port = local.rsplit(":", 1)[-1]
        if port.isdigit() and int(port) in ports:
            return True
    return False


def decide(now, last_active, idle_minutes, busy):
    """'active', 'wait' or 'stop'."""
    if busy:
        return "active"
    if idle_minutes <= 0 or now - last_active < idle_minutes * 60:
        return "wait"
    return "stop"


def summary(work):
    """What is working, per sandbox: {'demo': ['claude-agent-a', 'node'], …}, a few names each."""
    by = {}
    for _, name, sandbox in work:
        names = by.setdefault(sandbox, [])
        if name not in names and len(names) < 5:
            names.append(name)
    return by


def query(path):
    out = subprocess.run(["incus", "query", path], capture_output=True, text=True, timeout=30)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "incus query failed")
    return json.loads(out.stdout) if out.stdout.strip() else None


def main():
    idle_minutes, gateway = settings()
    if not os.path.exists(MARK):
        open(MARK, "w").close()
    reasons = []
    try:
        work = agent_work()
        if work:
            reasons.append({"agents": summary(work), "processes": len(work)})
        busy = incus_busy(query("/1.0/operations?recursion=1&all-projects=true"))
        if busy:
            reasons.append({"incus": busy})
        instances = query("/1.0/instances?recursion=1&all-projects=true")
        ss = subprocess.run(["ss", "-tnH", "state", "established"], capture_output=True, text=True, check=True).stdout
        if connected(ss, {gateway} | focused_ports(instances)):
            reasons.append({"connected": True})
    except Exception as error:  # noqa: BLE001 — unsure is used: never stop on a failed look
        reasons.append({"unknown": str(error)[:200]})
    now = time.time()
    verdict = decide(now, os.path.getmtime(MARK), idle_minutes, bool(reasons))
    if verdict == "active":
        os.utime(MARK)
    idle_for = 0 if verdict == "active" else now - os.path.getmtime(MARK)
    with open(REPORT, "w") as f:
        json.dump({"at": int(now), "idle_minutes": idle_minutes, "idle_for": int(idle_for),
                   "verdict": verdict, "why": reasons}, f)
    if verdict == "stop":
        print(f"orochi-sandbox-idle: no agent work and no connection for {idle_minutes} min; powering off", flush=True)
        subprocess.run(["systemctl", "poweroff"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
