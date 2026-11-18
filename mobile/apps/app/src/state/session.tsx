import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { Platform } from 'react-native';
import { RemoteClient, api, baseUrl, type Api } from '@orochi/protocol';
import {
  LEGACY_KEY,
  META_KEY,
  active as activeOf,
  emptyStore,
  fromStorage,
  markRepair,
  migrateLegacy,
  remove as removeFrom,
  rename as renameIn,
  setActive,
  toStorage,
  tokenKey,
  upsertPaired,
  type Incoming,
  type Machine,
  type MachineStore,
  type Upserted,
} from '@orochi/core';
import { titleIn } from '@orochi/core';
import { getSecret, setSecret } from '../lib/storage';

interface SessionValue {
  ready: boolean;
  machines: Machine[];
  /** The name to show for a machine: two with one name are told apart by their address. */
  titleOf(m: Machine): string;
  /** The machine every screen reads; null while none is paired. */
  active: Machine | null;
  /** Kept for screens that only need the active machine's address and name. */
  paired: Machine | null;
  client: RemoteClient | null;
  api: Api | null;
  /** Records a pairing: updates the known machine or adds one (and activates the first). */
  pair(next: Incoming, replaceKey?: string): Promise<Upserted>;
  switchTo(key: string): Promise<void>;
  rename(key: string, label: string): Promise<void>;
  remove(key: string): Promise<void>;
  /** A 401 from `key` (default: the active machine): that machine alone needs pairing again. */
  revoked(key?: string): Promise<void>;
}

const Ctx = createContext<SessionValue | null>(null);
const newKey = () => `m${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;

async function loadStore(): Promise<MachineStore> {
  const meta = await getSecret(META_KEY);
  if (meta) {
    let keys: string[] = [];
    try {
      keys = (JSON.parse(meta).machines ?? []).map((m: { key: string }) => m.key);
    } catch {
      /* fromStorage treats a damaged record as empty */
    }
    const tokens: Record<string, string | null> = {};
    for (const k of keys) tokens[tokenKey(k)] = await getSecret(tokenKey(k));
    return fromStorage(meta, (slot) => tokens[slot] ?? null);
  }
  // One machine, as earlier versions kept it: carried over without a re-pair.
  const migrated = migrateLegacy(await getSecret(LEGACY_KEY), Date.now(), newKey);
  if (!migrated) return emptyStore();
  await persist(emptyStore(), migrated);
  await setSecret(LEGACY_KEY, null);
  return migrated;
}

async function persist(before: MachineStore, after: MachineStore) {
  const { meta, tokens } = toStorage(after);
  for (const [slot, token] of Object.entries(tokens)) await setSecret(slot, token);
  for (const gone of before.machines)
    if (!after.machines.some((m) => m.key === gone.key)) await setSecret(tokenKey(gone.key), null);
  await setSecret(META_KEY, meta);
}

export function SessionProvider({ children }: { children: React.ReactNode }) {
  const [ready, setReady] = useState(false);
  const [store, setStoreState] = useState<MachineStore>(emptyStore());
  const ref = useRef<MachineStore>(store);

  useEffect(() => {
    loadStore().then((loaded) => {
      ref.current = loaded;
      setStoreState(loaded);
      setReady(true);
    });
  }, []);

  const update = useCallback(async (change: (s: MachineStore) => MachineStore) => {
    const before = ref.current;
    const after = change(before);
    if (after === before) return;
    ref.current = after;
    setStoreState(after);
    await persist(before, after);
  }, []);

  const active = activeOf(store);
  // A new client only when the machine's address or token changes, never for a label.
  const client = useMemo(
    () =>
      active
        ? new RemoteClient({ base: baseUrl(active.host, active.port), token: active.token })
        : null,
    [active?.key, active?.host, active?.port, active?.token], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const apis = useMemo(() => (client ? api(client) : null), [client]);

  const pair = useCallback(
    async (next: Incoming, replaceKey?: string) => {
      const result = upsertPaired(ref.current, next, Date.now(), newKey, replaceKey);
      await update(() => result.store);
      return result;
    },
    [update],
  );
  const switchTo = useCallback(
    (key: string) => update((s) => setActive(s, key, Date.now())),
    [update],
  );
  const rename = useCallback(
    (key: string, label: string) => update((s) => renameIn(s, key, label)),
    [update],
  );
  const remove = useCallback((key: string) => update((s) => removeFrom(s, key)), [update]);
  const revoked = useCallback(
    (key?: string) => {
      const target = key ?? ref.current.activeKey;
      return target ? update((s) => markRepair(s, target)) : Promise.resolve();
    },
    [update],
  );

  const value = useMemo<SessionValue>(
    () => ({
      ready,
      machines: store.machines,
      titleOf: (m: Machine) => titleIn(store, m),
      active,
      paired: active,
      client,
      api: apis,
      pair,
      switchTo,
      rename,
      remove,
      revoked,
    }),
    [ready, store, active, client, apis, pair, switchTo, rename, remove, revoked],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useSession(): SessionValue {
  const v = useContext(Ctx);
  if (!v) throw new Error('SessionProvider missing');
  return v;
}

const INSTALL_KEY = 'orochi.install';

/** A random id for this install, kept in the secure store: pairing again replaces the old row. */
export async function installId(): Promise<string> {
  const had = await getSecret(INSTALL_KEY);
  if (had) return had;
  const bytes = new Uint8Array(16);
  const c = (globalThis as { crypto?: { getRandomValues?: (a: Uint8Array) => void } }).crypto;
  if (c?.getRandomValues) c.getRandomValues(bytes);
  else for (let i = 0; i < 16; i++) bytes[i] = Math.floor(Math.random() * 256);
  const id = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
  await setSecret(INSTALL_KEY, id);
  return id;
}

export const deviceName = () =>
  Platform.OS === 'ios' ? 'iPhone' : Platform.OS === 'android' ? 'Android phone' : 'Browser';
