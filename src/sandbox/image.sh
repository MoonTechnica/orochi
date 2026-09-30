#!/bin/bash
# Builds the golden sandbox image, run as root inside a fresh Ubuntu 24.04 instance by
# `orochi sandbox image build`. No credential and no project file ever enters it.
set -euo pipefail
: "${SBX_UID:?}" "${SBX_GID:?}" "${SBX_BRIDGES:?}" "${SBX_DOCKER:=1}"
export DEBIAN_FRONTEND=noninteractive
for i in $(seq 1 60); do getent hosts deb.debian.org >/dev/null 2>&1 && break; sleep 1; done

apt-get update -qq
apt-get install -y -qq ca-certificates curl git gnupg unzip xz-utils ripgrep jq build-essential \
  python3 python3-venv iproute2 util-linux sudo

# The user agents run as: this machine's own IDs, so the mounted tree stays the user's.
getent group "$SBX_GID" >/dev/null || groupadd -g "$SBX_GID" dev
id -u dev >/dev/null 2>&1 || useradd -m -d /home/dev -s /bin/bash -u "$SBX_UID" -g "$SBX_GID" dev
echo "dev ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/dev

if [ "$SBX_DOCKER" = 1 ]; then
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -qq
  apt-get install -y -qq docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
  usermod -aG docker dev
  # `incus exec --user --group` starts a process with that one group and no supplementary
  # ones, so membership of `docker` never applies: the socket belongs to the user's own group.
  # systemd's docker.socket creates it, so that is where its group is set.
  mkdir -p /etc/systemd/system/docker.socket.d
  printf '[Socket]\nSocketGroup=%s\n' "$(getent group "$SBX_GID" | cut -d: -f1)" \
    > /etc/systemd/system/docker.socket.d/orochi.conf
  systemctl daemon-reload
  systemctl enable docker.socket docker
  systemctl restart docker.socket docker
fi

curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
apt-get install -y -qq nodejs
npm install -g --ignore-scripts=false @anthropic-ai/claude-code @openai/codex supabase
# The ACP bridges, pinned to the versions this Orochi drives on the Mac.
npm install -g --ignore-scripts $SBX_BRIDGES
sudo -u dev bash -lc 'curl -fsSL https://bun.sh/install | bash' || true
sudo -u dev bash -lc 'curl -fsSL https://deno.land/install.sh | sh -s -- -y' || true
sudo -u dev bash -lc 'curl -LsSf https://astral.sh/uv/install.sh | sh' || true
cat > /etc/profile.d/sbx-path.sh <<'PROFILE'
export PATH="$HOME/.bun/bin:$HOME/.deno/bin:$HOME/.local/bin:$PATH"
PROFILE

# Every agent and check starts through this. `incus exec` leaves a process running when the
# client on the other machine is killed (measured 2026-09-30), so the end of the connection
# has to be seen from here: the command runs in a group of its own, and whatever it leaves in
# that group dies with it. An agent reads its stdin (ACP) and sees the end of it; a check does
# not read stdin, so with SBX_HOLD=1 this process watches the stdin Orochi holds open instead.
cat > /usr/local/bin/sbx-run <<'RUN'
#!/usr/bin/python3
import os, signal, sys, threading
home = os.environ.get("HOME", "/home/dev")
os.environ["PATH"] = ":".join([f"{home}/.bun/bin", f"{home}/.deno/bin", f"{home}/.local/bin",
    "/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin"])
hold = os.environ.pop("SBX_HOLD", "") == "1"
pid = os.fork()
if pid == 0:
    os.setsid()
    if hold:
        null = os.open(os.devnull, os.O_RDONLY)
        os.dup2(null, 0)
    try:
        os.execvp(sys.argv[1], sys.argv[1:])
    except OSError as error:
        sys.stderr.write(f"sbx-run: {sys.argv[1]}: {error.strerror}\n")
        os._exit(127)
def end(*_):
    try:
        os.killpg(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
    signal.signal(sig, lambda *_: (end(), os._exit(143)))
if hold:
    def watch():
        while os.read(0, 4096):
            pass
        end()
    threading.Thread(target=watch, daemon=True).start()
_, status = os.waitpid(pid, 0)
end()
sys.exit(os.waitstatus_to_exitcode(status) if os.WIFEXITED(status) else 128 + os.WTERMSIG(status))
RUN
chmod 0755 /usr/local/bin/sbx-run

if [ "$SBX_DOCKER" = 1 ]; then
  # Pull Supabase's images once, into the volume every project's Docker data is cloned from.
  work=$(mktemp -d); chown dev "$work"
  sudo -u dev bash -lc "cd $work && supabase init --force >/dev/null && supabase start && supabase stop --no-backup" \
    || echo "image: supabase pre-pull failed; sandboxes will pull on first start"
  rm -rf "$work"
fi
apt-get clean
echo "image: built"
