import { useCallback, useEffect, useSyncExternalStore } from 'react';
import { getSecret, setSecret } from '../lib/storage';
import { useSession } from './session';

/**
 * When the person last looked at each member thread of a conversation, kept on this phone only
 * (the server has no per-thread read marker except mentions): one small record per machine and
 * conversation, `{ member: time of the newest reply seen }`. The secure store is used because it
 * is what the app already has; a native key-value store would need a new native build.
 */
type Seen = Record<string, number>;
const cache = new Map<string, Seen>();
const loading = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;
const bump = () => {
  version++;
  listeners.forEach((l) => l());
};
const clean = (s: string) => s.replace(/[^\w.-]/g, '_');
export const seenKey = (machine: string | undefined, thread: string) =>
  `orochi.seen.${clean(machine || 'x')}.${clean(thread)}`;
const NONE: Seen = {};

function load(key: string) {
  if (cache.has(key) || loading.has(key)) return;
  loading.add(key);
  getSecret(key).then((raw) => {
    let value: Seen = {};
    try {
      const parsed = raw ? JSON.parse(raw) : {};
      if (parsed && typeof parsed === 'object') value = parsed;
    } catch {
      /* an unreadable record starts empty */
    }
    // A mark made while it loaded wins.
    cache.set(key, { ...value, ...(cache.get(key) ?? {}) });
    loading.delete(key);
    bump();
  });
}

export function useThreadSeen(thread: string): {
  seen: Seen;
  ready: boolean;
  mark: (member: string, at: number | null) => void;
} {
  const { active } = useSession();
  const key = seenKey(active?.key, thread);
  useEffect(() => load(key), [key]);
  useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => version,
  );
  const mark = useCallback(
    (member: string, at: number | null) => {
      if (at === null) return;
      const name = member.toLowerCase();
      const now = cache.get(key) ?? {};
      if ((now[name] ?? 0) >= at) return;
      const next = { ...now, [name]: at };
      cache.set(key, next);
      setSecret(key, JSON.stringify(next));
      bump();
    },
    [key],
  );
  return { seen: cache.get(key) ?? NONE, ready: cache.has(key), mark };
}
