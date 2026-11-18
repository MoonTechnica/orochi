import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { AppState } from 'react-native';
import type {
  ConnectionState,
  LiveSeat,
  LiveTask,
  Mention,
  ProjectRow,
  Prompt,
  ServerEvent,
} from '@orochi/protocol';
import { useSession } from './session';

interface DataValue {
  projects: ProjectRow[];
  prompts: Prompt[];
  mentions: Mention[];
  seats: LiveSeat[];
  tasks: LiveTask[];
  connection: ConnectionState;
  loaded: boolean;
  error: string | null;
  refresh(): Promise<void>;
  /** Called with the thread ids a `changed` event named (empty list: everything). */
  onChanged(listener: (ids: string[]) => void): () => void;
}

const Ctx = createContext<DataValue | null>(null);

export function DataProvider({ children }: { children: React.ReactNode }) {
  const { api, client, revoked, active } = useSession();
  const key = active?.key;
  const repair = !!active?.needsRepair;
  const [projects, setProjects] = useState<ProjectRow[]>([]);
  const [prompts, setPrompts] = useState<Prompt[]>([]);
  const [mentions, setMentions] = useState<Mention[]>([]);
  const [seats, setSeats] = useState<LiveSeat[]>([]);
  const [tasks, setTasks] = useState<LiveTask[]>([]);
  const [connection, setConnection] = useState<ConnectionState>('closed');
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const listeners = useRef(new Set<(ids: string[]) => void>());

  const refresh = useCallback(async () => {
    if (!api || repair) return;
    try {
      const [p, pr, m, s, k] = await Promise.all([
        api.sidebar(),
        api.openPrompts(),
        api.mentions(),
        api.liveSeats(),
        api.liveTasks(),
      ]);
      setProjects(p);
      setPrompts(pr);
      setMentions(m);
      setSeats(s);
      setTasks(k);
      setError(null);
      setLoaded(true);
    } catch (e: any) {
      setError(String(e?.message ?? e));
      if (e?.status === 401) await revoked(key);
    }
  }, [api, revoked, key, repair]);

  useEffect(() => {
    setLoaded(false);
    refresh();
  }, [refresh]);

  useEffect(() => {
    if (!client || repair) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const soon = (ids: string[]) => {
      listeners.current.forEach((l) => l(ids));
      if (timer) return;
      timer = setTimeout(() => {
        timer = null;
        refresh();
      }, 250);
    };
    const sub = client.subscribe(
      (event: ServerEvent) => {
        if (event.type === 'changed') soon(event.threads);
        else if (event.type === 'prompts') soon([]);
        else if (event.type === 'hello') soon([]);
      },
      setConnection,
      () => revoked(key),
    );
    // A socket that is down must not leave the lists stale.
    const poll = setInterval(() => refresh(), 20000);
    const app = AppState.addEventListener('change', (s) => s === 'active' && soon([]));
    return () => {
      sub.close();
      clearInterval(poll);
      app.remove();
      if (timer) clearTimeout(timer);
    };
  }, [client, refresh, revoked, key, repair]);

  const onChanged = useCallback((listener: (ids: string[]) => void) => {
    listeners.current.add(listener);
    return () => {
      listeners.current.delete(listener);
    };
  }, []);

  const value = useMemo(
    () => ({
      projects,
      prompts,
      mentions,
      seats,
      tasks,
      connection,
      loaded,
      error,
      refresh,
      onChanged,
    }),
    [projects, prompts, mentions, seats, tasks, connection, loaded, error, refresh, onChanged],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useData(): DataValue {
  const v = useContext(Ctx);
  if (!v) throw new Error('DataProvider missing');
  return v;
}
