import { useCallback, useEffect, useRef, useState } from 'react';
import type { Member, Said, Thread } from '@orochi/protocol';
import { useData } from './data';
import { useSession } from './session';

export interface ThreadData {
  thread: Thread | null;
  said: Said[];
  members: Member[];
  loading: boolean;
  error: string | null;
  reload(): Promise<void>;
}

/** One conversation, kept fresh by the change feed. */
export function useThreadData(id: string): ThreadData {
  const { api, revoked, active } = useSession();
  const key = active?.key;
  const { onChanged } = useData();
  const [thread, setThread] = useState<Thread | null>(null);
  const [said, setSaid] = useState<Said[]>([]);
  const [members, setMembers] = useState<Member[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);
  const again = useRef(false);

  const reload = useCallback(async () => {
    if (!api) return;
    if (busy.current) {
      again.current = true;
      return;
    }
    busy.current = true;
    try {
      const [th, room, mem] = await Promise.all([api.thread(id), api.room(id), api.members(id)]);
      setThread(th);
      setSaid(room);
      setMembers(mem);
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
      if (e?.status === 401) revoked(key);
    } finally {
      busy.current = false;
      setLoading(false);
      if (again.current) {
        again.current = false;
        reload();
      }
    }
  }, [api, id, revoked, key]);

  useEffect(() => {
    setLoading(true);
    reload();
  }, [reload]);

  useEffect(
    () => onChanged((ids) => (ids.length === 0 || ids.includes(id)) && reload()),
    [onChanged, id, reload],
  );

  return { thread, said, members, loading, error, reload };
}
