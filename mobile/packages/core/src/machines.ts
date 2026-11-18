/**
 * The desktops one phone is paired with. Pure: the app persists what `toStorage` returns and
 * rebuilds with `fromStorage`; nothing here touches a store or the network.
 */
export interface Machine {
  /** Local key: names the machine's cache scope and its token's storage slot. Never sent. */
  key: string;
  /** The desktop's own id (`/health`), absent for a desktop that predates it. */
  machineId?: string;
  /** What the desktop calls itself. */
  name: string;
  /** What the user calls it here; wins over `name`. */
  label?: string;
  host: string;
  port: number;
  token: string;
  addedAt: number;
  lastUsedAt: number;
  /** The token was refused (revoked on the desktop): this machine alone needs pairing again. */
  needsRepair?: boolean;
}

export interface MachineStore {
  machines: Machine[];
  activeKey: string | null;
}

export const emptyStore = (): MachineStore => ({ machines: [], activeKey: null });

export const displayName = (m: Machine) => (m.label?.trim() || m.name || m.host).trim();

/** The name shown in lists: two machines with one name are told apart by their address. */
export const titleIn = (store: MachineStore, m: Machine) =>
  store.machines.some((o) => o.key !== m.key && displayName(o) === displayName(m))
    ? `${displayName(m)} (${m.host}:${m.port})`
    : displayName(m);

export const active = (store: MachineStore): Machine | null =>
  store.machines.find((m) => m.key === store.activeKey) ?? null;

export interface Incoming {
  host: string;
  port: number;
  token: string;
  name: string;
  machineId?: string;
}

const sameAddress = (a: { host: string; port: number }, b: { host: string; port: number }) =>
  a.host.toLowerCase() === b.host.toLowerCase() && a.port === b.port;

/**
 * The machine a pairing is for: by the desktop's id when both sides have one; otherwise by
 * address. Two different ids at one address are two machines (a desktop was replaced).
 */
export function findFor(
  store: MachineStore,
  who: { host: string; port: number; machineId?: string },
): Machine | undefined {
  if (who.machineId) {
    const byId = store.machines.find((m) => m.machineId === who.machineId);
    if (byId) return byId;
    return store.machines.find((m) => !m.machineId && sameAddress(m, who));
  }
  return store.machines.find((m) => sameAddress(m, who));
}

export interface Upserted {
  store: MachineStore;
  machine: Machine;
  outcome: 'added' | 'updated';
}

/** Pairing result in: updates the known machine (new address and token) or adds one. */
export function upsertPaired(
  store: MachineStore,
  incoming: Incoming,
  now: number,
  newKey: () => string,
  /** Pairing again from a machine's own "Pair again": that machine is updated whatever id answers. */
  replaceKey?: string,
): Upserted {
  const known =
    (replaceKey ? store.machines.find((m) => m.key === replaceKey) : undefined) ??
    findFor(store, incoming);
  if (known) {
    const machine: Machine = {
      ...known,
      host: incoming.host,
      port: incoming.port,
      token: incoming.token,
      name: incoming.name || known.name,
      machineId: incoming.machineId ?? known.machineId,
      lastUsedAt: now,
      needsRepair: false,
    };
    return {
      store: {
        ...store,
        machines: store.machines.map((m) => (m.key === known.key ? machine : m)),
      },
      machine,
      outcome: 'updated',
    };
  }
  const machine: Machine = {
    key: newKey(),
    machineId: incoming.machineId,
    name: incoming.name || incoming.host,
    host: incoming.host,
    port: incoming.port,
    token: incoming.token,
    addedAt: now,
    lastUsedAt: now,
  };
  return {
    store: { machines: [...store.machines, machine], activeKey: store.activeKey ?? machine.key },
    machine,
    outcome: 'added',
  };
}

export function setActive(store: MachineStore, key: string, now: number): MachineStore {
  if (!store.machines.some((m) => m.key === key)) return store;
  return {
    activeKey: key,
    machines: store.machines.map((m) => (m.key === key ? { ...m, lastUsedAt: now } : m)),
  };
}

export function rename(store: MachineStore, key: string, label: string): MachineStore {
  const clean = label.trim().slice(0, 40);
  return {
    ...store,
    machines: store.machines.map((m) => (m.key === key ? { ...m, label: clean || undefined } : m)),
  };
}

export function markRepair(store: MachineStore, key: string, needs = true): MachineStore {
  return {
    ...store,
    machines: store.machines.map((m) => (m.key === key ? { ...m, needsRepair: needs } : m)),
  };
}

/** Removing the active machine falls back to the one used most recently, else none. */
export function remove(store: MachineStore, key: string): MachineStore {
  const machines = store.machines.filter((m) => m.key !== key);
  const activeKey =
    store.activeKey !== key
      ? store.activeKey
      : ([...machines].sort((a, b) => b.lastUsedAt - a.lastUsedAt)[0]?.key ?? null);
  return { machines, activeKey };
}

// ---- storage -----------------------------------------------------------------------------

export const META_KEY = 'orochi.machines';
export const LEGACY_KEY = 'orochi.session';
export const tokenKey = (key: string) => `orochi.token.${key}`;

/** Tokens go to the secure slot of each machine; everything else is one small record. */
export function toStorage(store: MachineStore): { meta: string; tokens: Record<string, string> } {
  const tokens: Record<string, string> = {};
  const machines = store.machines.map(({ token, ...rest }) => {
    tokens[tokenKey(rest.key)] = token;
    return rest;
  });
  return { meta: JSON.stringify({ v: 1, activeKey: store.activeKey, machines }), tokens };
}

export function fromStorage(
  meta: string | null,
  tokenOf: (slot: string) => string | null,
): MachineStore {
  if (!meta) return emptyStore();
  try {
    const parsed = JSON.parse(meta) as { activeKey?: string | null; machines?: unknown };
    const machines: Machine[] = [];
    for (const raw of Array.isArray(parsed.machines) ? parsed.machines : []) {
      const m = raw as Partial<Machine>;
      const token = m.key ? tokenOf(tokenKey(m.key)) : null;
      if (!m.key || !m.host || !m.port || !token) continue;
      machines.push({
        key: m.key,
        machineId: m.machineId,
        name: m.name ?? m.host,
        label: m.label,
        host: m.host,
        port: m.port,
        token,
        addedAt: m.addedAt ?? 0,
        lastUsedAt: m.lastUsedAt ?? 0,
        needsRepair: m.needsRepair,
      });
    }
    const activeKey = machines.some((m) => m.key === parsed.activeKey)
      ? (parsed.activeKey as string)
      : (machines[0]?.key ?? null);
    return { machines, activeKey };
  } catch {
    return emptyStore();
  }
}

/** The single-machine record earlier versions kept (`{host, port, token, name}`), if valid. */
export function migrateLegacy(
  raw: string | null,
  now: number,
  newKey: () => string,
): MachineStore | null {
  if (!raw) return null;
  try {
    const v = JSON.parse(raw) as Partial<Incoming>;
    if (!v.host || !v.token || !v.port) return null;
    const machine: Machine = {
      key: newKey(),
      name: v.name || v.host,
      host: v.host,
      port: v.port,
      token: v.token,
      addedAt: now,
      lastUsedAt: now,
    };
    return { machines: [machine], activeKey: machine.key };
  } catch {
    return null;
  }
}
