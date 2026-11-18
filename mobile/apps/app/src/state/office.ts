import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { RemoteClient, api, baseUrl, type Talk } from '@orochi/protocol';
import { CHATTER_MS, buildOffice, newTracker, type Office, type OfficeTracker } from '@orochi/core';
import { useAttention } from './attention';
import { useSession } from './session';

const EMPTY: Office = {
  floors: [],
  stats: { working: 0, assistants: 0, asking: 0, messages: 0, rooms: 0 },
  quiet: true,
  nextChange: 0,
};

/**
 * One desktop's office: its sidebar, seats and tasks (live for the active desktop, polled for the
 * others), what its agents said lately, and the arrivals and departures between one look and the next.
 */
export function useOffice(key: string | null) {
  const { machines } = useSession();
  const { snapshots, refreshAll } = useAttention();
  const machine = machines.find((m) => m.key === key) ?? null;
  const snap = snapshots.find((s) => s.key === key) ?? null;
  const client = useMemo(
    () =>
      machine
        ? api(
            new RemoteClient({
              base: baseUrl(machine.host, machine.port),
              token: machine.token,
              timeoutMs: 6000,
            }),
          )
        : null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [machine?.key, machine?.host, machine?.port, machine?.token],
  );
  const [talk, setTalk] = useState<Talk[]>([]);
  const [icons, setIcons] = useState<Record<string, string>>({});
  const [seen, setSeen] = useState<ReadonlySet<number>>(new Set());
  const [tick, setTick] = useState(0);
  const tracker = useRef<{ key: string | null; value: OfficeTracker }>({
    key,
    value: newTracker(),
  });
  if (tracker.current.key !== key) tracker.current = { key, value: newTracker() };
  const readAt = useRef(0);

  const readTalk = useCallback(async () => {
    if (!client) return;
    try {
      const lines = await client.chatter(Date.now() - CHATTER_MS);
      setTalk(Array.isArray(lines) ? lines : []);
    } catch {
      // The picture stays as it was; the next change or the timer tries again.
    }
  }, [client]);

  useEffect(() => {
    setTalk([]);
    setIcons({});
    setSeen(new Set());
    if (!client) return;
    client
      .agentIcons()
      .then((v) => setIcons(v && typeof v === 'object' ? v : {}))
      .catch(() => {});
    readTalk();
    const id = setInterval(readTalk, 10000);
    return () => clearInterval(id);
  }, [client, readTalk]);

  // What the sidebar and the seats say changed: read what was said, at most every three seconds.
  useEffect(() => {
    if (Date.now() - readAt.current < 3000) return;
    readAt.current = Date.now();
    readTalk();
  }, [snap?.projects, snap?.seats, readTalk]);

  const built = useMemo(
    () =>
      snap && snap.state === 'online'
        ? buildOffice(
            {
              projects: snap.projects,
              seats: snap.seats,
              tasks: snap.tasks,
              talk,
              seen,
            },
            tracker.current.value,
            Date.now(),
          )
        : null,
    // `tick` is the clock the picture changes by itself on.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [snap?.state, snap?.projects, snap?.seats, snap?.tasks, talk, seen, tick],
  );
  useEffect(() => {
    if (built) tracker.current.value = built.tracker;
  }, [built]);
  const next = built?.office.nextChange ?? 0;
  useEffect(() => {
    if (!next) return;
    const id = setTimeout(() => setTick((n) => n + 1), Math.max(50, next - Date.now() + 50));
    return () => clearTimeout(id);
  }, [next, built]);

  const open = useCallback((seq: number) => setSeen((s) => new Set(s).add(seq)), []);
  return {
    machine,
    snap,
    office: built?.office ?? EMPTY,
    icons,
    client,
    refreshAll,
    markSeen: open,
  };
}
