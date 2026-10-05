#!/bin/bash
# Invoked through sudo/pkexec with the original user's IDs in SBX_*.
set -euo pipefail
install -m 0644 "$1" /var/lib/orochi-sandbox-host.sh
install -m 0644 "$2" /var/lib/orochi-sandbox-gateway.py
install -m 0644 "$3" /var/lib/orochi-sandbox-idle.py
bash /var/lib/orochi-sandbox-host.sh
# sg consults the updated group database, so this works before the next login and does not
# require a permanent sudo rule. Quote every argument, including user-provided paths.
cat > /usr/local/bin/orochi-incus <<'WRAPPER'
#!/usr/bin/python3
import os
import shlex
import sys
os.execvp("sg", ["sg", "incus-admin", "-c", shlex.join(["incus", *sys.argv[1:]])])
WRAPPER
chmod 0755 /usr/local/bin/orochi-incus
touch "/var/lib/orochi-sandbox-${SBX_PROJECT}.ready"
