import { useCallback, useRef, useState } from 'react';
import { AppState } from 'react-native';
import { useFocusEffect } from 'expo-router';
import { api, baseUrl, RemoteClient, type Api } from '@orochi/protocol';
import { useSession } from './session';

type Reading<T> = { state: 'loaded'; data: T } | { state: 'unavailable' };

export function useWorkspaceReadings<T>(read: (remote: Api) => Promise<T>) {
  const { machines } = useSession();
  const [readings, setReadings] = useState<Record<string, Reading<T>>>({});
  const refresh = useRef<() => Promise<void>>(async () => {});
  useFocusEffect(
    useCallback(() => {
      let current = true;
      let inFlight: Promise<void> | undefined;
      const poll = () => {
        if (AppState.currentState !== 'active') return Promise.resolve();
        if (inFlight) return inFlight;
        inFlight = Promise.all(
          machines
            .filter((machine) => !machine.needsRepair)
            .map(async (machine) => {
              let reading: Reading<T>;
              try {
                const remote = api(
                  new RemoteClient({
                    base: baseUrl(machine.host, machine.port),
                    token: machine.token,
                    timeoutMs: 6000,
                  }),
                );
                reading = { state: 'loaded', data: await read(remote) };
              } catch {
                reading = { state: 'unavailable' };
              }
              if (current) setReadings((previous) => ({ ...previous, [machine.key]: reading }));
            }),
        ).then(() => {
          inFlight = undefined;
        });
        return inFlight;
      };
      refresh.current = poll;
      void poll();
      const timer = setInterval(poll, 20000);
      const appState = AppState.addEventListener('change', (state) => {
        if (state === 'active') void poll();
      });
      return () => {
        current = false;
        refresh.current = async () => {};
        clearInterval(timer);
        appState.remove();
      };
    }, [machines, read]),
  );
  return { readings, refresh: () => refresh.current() };
}
