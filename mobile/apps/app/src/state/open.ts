import { useCallback } from 'react';
import { useRouter } from 'expo-router';
import { useSession } from './session';

/** Opens a conversation of a given machine, switching to that machine first when needed. */
export function useOpenThread() {
  const router = useRouter();
  const { active, switchTo } = useSession();
  return useCallback(
    async (machineKey: string | null | undefined, thread: string, reply?: string) => {
      // `reply` opens the conversation with the composer focused and the person addressed.
      const params = reply ? { id: thread, reply } : { id: thread };
      if (machineKey && machineKey !== active?.key) {
        await switchTo(machineKey);
        router.replace('/');
        setTimeout(() => router.push({ pathname: '/thread/[id]', params }), 120);
      } else router.push({ pathname: '/thread/[id]', params });
    },
    [active?.key, switchTo, router],
  );
}
