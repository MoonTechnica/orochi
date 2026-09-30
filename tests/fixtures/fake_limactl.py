"""A stand-in for `limactl`: one VM whose state is a file, `start` / `stop` / `list --json`,
and `shell … incus …` handed to the fake `incus` beside it."""
import json
import os
import sys

state = os.environ["FAKE_LIMA_STATE"]
log = os.environ["FAKE_LIMA_LOG"]
args = sys.argv[1:]
with open(log, "a") as f:
    f.write(json.dumps(args) + "\n")
status = open(state).read().strip() if os.path.exists(state) else "Stopped"
if args[:1] == ["list"]:
    print(json.dumps({"name": "sbx-host", "status": status}))
elif args[:1] == ["start"]:
    open(state, "w").write("Running")
elif args[:1] == ["stop"]:
    open(state, "w").write("Stopped")
elif args[:1] == ["shell"]:
    if status != "Running":
        sys.stderr.write("instance is not running\n")
        sys.exit(1)
    rest = args[args.index("incus") + 1:]
    os.execv(os.environ["FAKE_INCUS"], [os.environ["FAKE_INCUS"]] + rest)
else:
    sys.exit(2)
