#!/bin/bash
# Prepares the sandbox host: Incus, a ZFS pool, the bridge and the profiles every sandbox is
# made from. Run as root by `orochi sandbox up`; safe to run again, and applies changed sizes.
set -euo pipefail
: "${SBX_PROJECT:?}" "${SBX_SUBNET:?}" "${SBX_POOL_GIB:?}" "${SBX_UID:?}" "${SBX_GID:?}"
: "${SBX_CPU:?}" "${SBX_MEM_GIB:?}" "${SBX_USER:=}"
export DEBIAN_FRONTEND=noninteractive

if ! command -v incus >/dev/null; then
  echo "sandbox host: installing Incus and ZFS"
  apt-get update -qq
  apt-get install -y -qq curl gnupg zfsutils-linux
  mkdir -p /etc/apt/keyrings
  curl -fsSL https://pkgs.zabbly.com/key.asc | gpg --dearmor -o /etc/apt/keyrings/zabbly.gpg
  cat > /etc/apt/sources.list.d/zabbly-incus-stable.sources <<SRC
Enabled: yes
Types: deb
URIs: https://pkgs.zabbly.com/incus/stable
Suites: $(. /etc/os-release && echo "$VERSION_CODENAME")
Components: main
Architectures: $(dpkg --print-architecture)
Signed-By: /etc/apt/keyrings/zabbly.gpg
SRC
  apt-get update -qq
  apt-get install -y -qq incus
fi
command -v zfs >/dev/null || apt-get install -y -qq zfsutils-linux

# Docker inside a container cannot load modules; the host loads them.
cat > /etc/modules-load.d/orochi-sandbox.conf <<MOD
overlay
br_netfilter
ip_tables
ip6_tables
iptable_nat
nf_nat
xt_conntrack
veth
MOD
for m in $(cat /etc/modules-load.d/orochi-sandbox.conf); do modprobe "$m" 2>/dev/null || true; done

# ZFS's cache counts as used memory and by default grows to half the VM (measured: 2.85 of
# 3.87 GiB on an 8 GiB VM with one Supabase stack). The sandboxes need that memory more.
ARC_MAX=$((1024 * 1024 * 1024))
echo "options zfs zfs_arc_max=$ARC_MAX" > /etc/modprobe.d/orochi-zfs.conf
[ -w /sys/module/zfs/parameters/zfs_arc_max ] && echo "$ARC_MAX" > /sys/module/zfs/parameters/zfs_arc_max

# raw.idmap may only name IDs root is delegated.
grep -qx "root:${SBX_UID}:1" /etc/subuid || echo "root:${SBX_UID}:1" >> /etc/subuid
grep -qx "root:${SBX_GID}:1" /etc/subgid || echo "root:${SBX_GID}:1" >> /etc/subgid
systemctl restart incus 2>/dev/null || true
[ -n "$SBX_USER" ] && usermod -aG incus-admin "$SBX_USER" || true

if ! incus storage show sbx-pool >/dev/null 2>&1; then
  incus admin init --minimal 2>/dev/null || true
  incus storage create sbx-pool zfs size="${SBX_POOL_GIB}GiB"
else
  incus storage set sbx-pool size="${SBX_POOL_GIB}GiB" 2>/dev/null || true
fi

if ! incus network show sbxbr0 >/dev/null 2>&1; then
  incus network create sbxbr0 ipv4.address="$SBX_SUBNET" ipv4.nat=true ipv6.address=none dns.domain=sbx
else
  incus network set sbxbr0 ipv4.address="$SBX_SUBNET"
fi

incus project show "$SBX_PROJECT" >/dev/null 2>&1 || incus project create "$SBX_PROJECT" \
  -c features.images=true -c features.profiles=true -c features.storage.volumes=true
P="--project $SBX_PROJECT"

incus $P profile show sbx-base >/dev/null 2>&1 || incus $P profile create sbx-base
incus $P profile edit sbx-base <<PROFILE
description: Orochi sandbox
config:
  limits.cpu: "${SBX_CPU}"
  limits.memory: ${SBX_MEM_GIB}GiB
  limits.memory.enforce: soft
devices:
  eth0: {name: eth0, network: sbxbr0, type: nic}
  root: {path: /, pool: sbx-pool, type: disk}
PROFILE

incus $P profile show sbx-docker >/dev/null 2>&1 || incus $P profile create sbx-docker
incus $P profile edit sbx-docker <<PROFILE
description: Orochi sandbox with Docker inside
config:
  security.nesting: "true"
  security.syscalls.intercept.mknod: "true"
  security.syscalls.intercept.setxattr: "true"
devices: {}
PROFILE

# One login inside serves every sandbox: the CLIs' own homes live here, mounted into each.
install -d -o "$SBX_UID" -g "$SBX_GID" -m 0700 /var/lib/sbx/creds/claude /var/lib/sbx/creds/codex
echo "sandbox host: ready"
