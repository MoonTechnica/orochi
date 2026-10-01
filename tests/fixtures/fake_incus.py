"""A stand-in for the `incus` client. It remembers instances, devices and volumes in a JSON
file, logs every invocation as one JSON line, and runs `exec`'d commands on this machine in
place of the instance, so an ACP agent "inside" is a real process whose stdio is the pipe."""
import json
import os
import sys

state_path = os.environ["FAKE_INCUS_STATE"]
log_path = os.environ["FAKE_INCUS_LOG"]


def load():
    try:
        with open(state_path) as f:
            return json.load(f)
    except FileNotFoundError:
        return {"images": ["sbx-golden"], "instances": {}, "volumes": {}}


def save(state):
    with open(state_path, "w") as f:
        json.dump(state, f)


def fail(message):
    sys.stderr.write("Error: " + message + "\n")
    sys.exit(1)


args = sys.argv[1:]
with open(log_path, "a") as f:
    f.write(json.dumps(args) + "\n")
if args[:1] == ["--project"]:
    args = args[2:]
state = load()
instances = state["instances"]
command, rest = args[0], args[1:]
if os.environ.get("FAKE_INCUS_FAIL") and os.environ["FAKE_INCUS_FAIL"] == command:
    fail("injected failure")

if command == "list":
    names = [a for a in rest if not a.startswith("--") and a != "json"]
    listed = [
        {"name": n, "status": i["status"],
         "state": {"network": {"eth0": {"addresses": [{"family": "inet", "address": "10.203.0.7"}]}}}}
        for n, i in instances.items() if not names or n in names
    ]
    print(json.dumps(listed))
elif command == "project":
    print("name: sbx")
elif command == "image":
    if rest[1] not in state["images"]:
        fail("Image not found")
elif command == "init":
    image, name = rest[0], rest[1]
    if image not in state["images"]:
        fail("Image not found")
    profiles = [rest[i + 1] for i, a in enumerate(rest) if a == "--profile"]
    instances[name] = {"status": "Stopped", "profiles": profiles, "vm": "--vm" in rest,
                       "config": {}, "devices": {}, "snapshots": []}
elif command == "config":
    sub = rest[0]
    if sub == "set":
        instances[rest[1]]["config"][rest[2]] = rest[3]
    elif sub == "device":
        action, name = rest[1], rest[2]
        devices = instances[name]["devices"]
        if action == "add":
            devices[rest[3]] = {"type": rest[4], **dict(kv.split("=", 1) for kv in rest[5:])}
        elif action == "override":
            devices[rest[3]] = {"type": "disk", "override": True, **dict(kv.split("=", 1) for kv in rest[4:])}
        elif action == "remove":
            devices.pop(rest[3])
        elif action == "list":
            print("\n".join(devices))
elif command == "storage":
    action, pool = rest[1], rest[2]
    volumes = state["volumes"]
    if action == "copy":
        volumes[rest[3]] = dict(volumes[rest[2]])
    elif action == "create":
        volumes[f"{pool}/{rest[3]}"] = {}
    elif action == "set":
        key, value = rest[4].split("=", 1)
        volumes[f"{pool}/{rest[3]}"][key] = value
    elif action == "show":
        if f"{pool}/{rest[3]}" not in volumes:
            fail("Storage volume not found")
    elif action == "delete":
        volumes.pop(f"{pool}/{rest[3]}")
    elif action == "snapshot":
        pass
elif command in ("start", "stop"):
    instances[rest[0]]["status"] = "Running" if command == "start" else "Stopped"
elif command == "delete":
    name = [a for a in rest if not a.startswith("--")][0]
    instances.pop(name)
elif command == "snapshot":
    if rest[0] == "create":
        instances[rest[1]]["snapshots"].append(rest[2])
    elif rest[2] not in instances[rest[1]]["snapshots"]:
        fail("Snapshot not found")
elif command == "exec":
    name = rest[0]
    if instances.get(name, {}).get("status") != "Running":
        fail("Instance is not running")
    split = rest.index("--")
    options, argv = rest[1:split], rest[split + 1:]
    env = dict(os.environ)
    cwd = None
    i = 0
    while i < len(options):
        if options[i] == "--cwd":
            cwd = options[i + 1]
            i += 1
        elif options[i] == "--env":
            key, value = options[i + 1].split("=", 1)
            # The sandbox's home is a directory of the test's own: a test never writes to the
            # home of the machine it runs on.
            env[key] = os.environ["FAKE_INCUS_HOME"] if key == "HOME" else value
            i += 1
        i += 1
    if argv[:1] == ["sbx-run"]:
        argv = argv[1:]
    # So is the sandbox's memory.
    shm = os.environ["FAKE_INCUS_SHM"]
    argv = [a.replace("/dev/shm/", shm + "/") for a in argv]
    os.makedirs(env.get("HOME", shm), exist_ok=True)
    os.makedirs(shm, exist_ok=True)
    if argv[:1] == ["ss"]:
        for port in os.environ.get("FAKE_SS", "").split(","):
            if port:
                print(f"LISTEN 0 4096 0.0.0.0:{port} 0.0.0.0:*")
        sys.exit(0)
    save(state)
    if cwd:
        os.chdir(cwd)
    os.execvpe(argv[0], argv, env)
else:
    fail("unknown command " + command)
save(state)
