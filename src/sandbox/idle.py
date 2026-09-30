#!/usr/bin/python3
"""Powers the sandbox VM off once nothing has used it for a while, run every minute by a
systemd timer that `orochi sandbox up` installs. Orochi starts the VM again when a run, or any
`orochi sandbox` command that needs it, finds it stopped, so stopping is never a loss.

Used means one of: a command running inside a sandbox (an agent, a check, a shell: each is an
Incus exec operation), or a connection from the Mac through the gateway or a focused port.
A sandbox that merely runs (Supabase idling) is not use. The clock starts at boot, since the
mark lives in /run.
"""
import json
import os
import subprocess
import sys
import time

MARK = os.environ.get("SBX_IDLE_MARK", "/run/orochi-sandbox-active")
CONF = os.environ.get("SBX_IDLE_CONF", "/etc/orochi-sandbox.conf")


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


def running_exec(operations):
    """Whether any Incus operation is a command running inside an instance."""
    return any(
        op.get("status") == "Running" and "exec" in (op.get("description", "") + op.get("class", "")).lower()
        for op in (operations or {}).get("running", []) or []
    )


def focused_ports(instances):
    """The loopback ports focus added (proxy devices), which a Mac client connects through."""
    ports = set()
    for instance in instances or []:
        for name, device in (instance.get("devices") or {}).items():
            if name.startswith("sbx-port-") and device.get("type") == "proxy":
                listen = device.get("listen", "")
                port = listen.rsplit(":", 1)[-1]
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


def query(path):
    out = subprocess.run(["incus", "query", path], capture_output=True, text=True, timeout=30)
    return json.loads(out.stdout) if out.returncode == 0 and out.stdout.strip() else None


def main():
    idle_minutes, gateway = settings()
    if idle_minutes <= 0:
        return 0
    if not os.path.exists(MARK):
        open(MARK, "w").close()
    operations = query("/1.0/operations?recursion=1&all-projects=true")
    instances = query("/1.0/instances?recursion=1&all-projects=true")
    ss = subprocess.run(["ss", "-tnH", "state", "established"], capture_output=True, text=True).stdout
    busy = running_exec(operations) or connected(ss, {gateway} | focused_ports(instances))
    verdict = decide(time.time(), os.path.getmtime(MARK), idle_minutes, busy)
    if verdict == "active":
        os.utime(MARK)
    elif verdict == "stop":
        print(f"orochi-sandbox-idle: unused for {idle_minutes} min; powering off", flush=True)
        subprocess.run(["systemctl", "poweroff"])
    return 0


if __name__ == "__main__":
    sys.exit(main())
