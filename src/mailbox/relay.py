# Portable stdio MCP transport for agents inside a sandbox. All tool handling stays on host.
import json
import os
from pathlib import Path
import sys
import time
import uuid

root = Path(sys.argv[1])
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    key = uuid.uuid4().hex
    pending = root / (key + ".request")
    temporary = root / (key + ".tmp")
    temporary.write_text(line)
    os.replace(temporary, pending)
    reply = root / (key + ".response")
    deadline = time.monotonic() + 150
    while not reply.exists():
        if not root.is_dir() or time.monotonic() >= deadline:
            raise SystemExit("Orochi mailbox relay closed or timed out")
        time.sleep(0.02)
    print(reply.read_text(), flush=True)
    reply.unlink()
