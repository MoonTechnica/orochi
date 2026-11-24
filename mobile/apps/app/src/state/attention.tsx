import React, { createContext, useContext, useEffect, useMemo, useRef, useState } from 'react';
import { AppState } from 'react-native';
import {
  RemoteClient,
  api,
  baseUrl,
  type LiveSeat,
  type LiveTask,
  type Mention,
  type ProjectRow,
  type Prompt,
} from '@orochi/protocol';
import { buildHome, titleIn, type Home, type Machine, type MachineSnapshot } from '@orochi/core';
import { useData } from './data';
import { useSession } from './session';

export type MachineState = 'online' | 'connecting' | 'offline' | 'repair';

export interface Waiting {
  machineKey: string;
  machineName: string;
  kind: 'prompt' | 'mention';
  id: string;
  thread: string;
  title: string;
  body: string;
}

interface Seen {
  state: MachineState;
  error: string | null;
  projects: ProjectRow[];
  prompts: Prompt[];
  mentions: Mention[];
  seats: LiveSeat[];
  tasks: LiveTask[];
}

const blank = (state: MachineState, error: string | null = null): Seen => ({
  state,
  error,
  projects: [],
  prompts: [],
  mentions: [],
  seats: [],
  tasks: [],
});

interface AttentionValue {
  states: Record<string, MachineState>;
  /** What the machines other than the active one have waiting, read by polling. */
  others: Record<string, Seen>;
  waiting: Waiting[];
  /** Every desktop's last word, the active one live and the others polled. */
  home: Home;
  snapshots: MachineSnapshot[];
  /** Reads the active machine again and polls the others now. */
  refreshAll(): Promise<void>;
}

const Ctx = createContext<AttentionValue | null>(null);
const EVERY = 20000;

/**
 * The active machine is read live by `DataProvider` (WebSocket). The others are only polled
 * (open questions and mentions), and only while the app is in the foreground: no socket is
 * held for a machine nobody is looking at.
 */
export function AttentionProvider({ children }: { children: React.ReactNode }) {
  const { machines, active, revoked } = useSession();
  const { connection, error, loaded, prompts, mentions, projects, seats, tasks, refresh } =
    useData();
  const [others, setOthers] = useState<Record<string, Seen>>({});
  const watch = useMemo(
    () => machines.filter((m) => m.key !== active?.key),
    [machines, active?.key],
  );
  const signature = watch
    .map((m) => `${m.key}|${m.host}|${m.port}|${m.token}|${m.needsRepair ? 1 : 0}`)
    .join(',');
  const live = useRef(true);
  const pollNow = useRef<() => Promise<void>>(async () => {});

  useEffect(() => {
    live.current = true;
    const one = async (m: Machine) => {
      if (m.needsRepair) {
        setOthers((o) => ({ ...o, [m.key]: blank('repair') }));
        return;
      }
      const a = api(
        new RemoteClient({ base: baseUrl(m.host, m.port), token: m.token, timeoutMs: 6000 }),
      );
      try {
        const [sidebar, p, me, seatList, taskList] = await Promise.all([
          a.sidebar(),
          a.openPrompts(),
          a.mentions(),
          a.liveSeats(),
          a.liveTasks(),
        ]);
        if (live.current)
          setOthers((o) => ({
            ...o,
            [m.key]: {
              state: 'online',
              error: null,
              projects: sidebar,
              prompts: p,
              mentions: me,
              seats: seatList,
              tasks: taskList,
            },
          }));
      } catch (e: any) {
        if (!live.current) return;
        if (e?.status === 401) revoked(m.key);
        setOthers((o) => ({
          ...o,
          [m.key]: {
            ...(o[m.key] ?? blank('offline')),
            state: e?.status === 401 ? 'repair' : 'offline',
            error: String(e?.message ?? e),
          },
        }));
      }
    };
    const all = () => AppState.currentState === 'active' && watch.forEach(one);
    pollNow.current = async () => {
      await Promise.all(watch.map(one));
    };
    all();
    const timer = setInterval(all, EVERY);
    const app = AppState.addEventListener('change', (s) => s === 'active' && all());
    return () => {
      live.current = false;
      clearInterval(timer);
      app.remove();
    };
  }, [signature]); // eslint-disable-line react-hooks/exhaustive-deps

  const value = useMemo<AttentionValue>(() => {
    const states: Record<string, MachineState> = {};
    const store = { machines, activeKey: active?.key ?? null };
    const waiting: Waiting[] = [];
    const add = (m: Machine, p: Prompt[], me: Mention[]) => {
      const machineName = titleIn(store, m);
      for (const x of p)
        waiting.push({
          machineKey: m.key,
          machineName,
          kind: 'prompt',
          id: `p${x.id}`,
          thread: x.thread_id,
          title: x.thread_title || x.project,
          body: x.title || '',
        });
      for (const x of me)
        waiting.push({
          machineKey: m.key,
          machineName,
          kind: 'mention',
          id: `m${x.message_id}`,
          thread: x.thread_id,
          title: `${x.who} → you`,
          body: x.text.slice(0, 140),
        });
    };
    for (const m of machines) {
      if (m.key === active?.key) {
        states[m.key] = m.needsRepair
          ? 'repair'
          : error && connection !== 'open'
            ? 'offline'
            : connection === 'open'
              ? 'online'
              : connection === 'connecting' || !loaded
                ? 'connecting'
                : 'offline';
        if (!m.needsRepair) add(m, prompts, mentions);
      } else {
        const seen = others[m.key];
        states[m.key] = m.needsRepair ? 'repair' : (seen?.state ?? 'connecting');
        if (seen && !m.needsRepair) add(m, seen.prompts, seen.mentions);
      }
    }
    const snapshots: MachineSnapshot[] = machines.map((m) => {
      const name = titleIn(store, m);
      if (m.key === active?.key)
        return {
          key: m.key,
          name,
          state: states[m.key],
          error: m.needsRepair ? null : error,
          projects: m.needsRepair ? [] : projects,
          prompts: m.needsRepair ? [] : prompts,
          mentions: m.needsRepair ? [] : mentions,
          seats: m.needsRepair ? [] : seats,
          tasks: m.needsRepair ? [] : tasks,
        };
      const seen = others[m.key] ?? blank(m.needsRepair ? 'repair' : 'connecting');
      return { key: m.key, name, ...seen, state: states[m.key] };
    });
    const refreshAll = async () => {
      await Promise.all([refresh(), pollNow.current()]);
    };
    return { states, others, waiting, snapshots, home: buildHome(snapshots), refreshAll };
  }, [
    machines,
    active?.key,
    others,
    connection,
    error,
    loaded,
    prompts,
    mentions,
    projects,
    seats,
    tasks,
    refresh,
  ]);

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAttention(): AttentionValue {
  const v = useContext(Ctx);
  if (!v) throw new Error('AttentionProvider missing');
  return v;
}
