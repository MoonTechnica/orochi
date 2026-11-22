import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { Platform } from 'react-native';
import Constants from 'expo-constants';
import { newPushId, pushTap, validPushId } from '@orochi/core';
import { getSecret, setSecret } from '../lib/storage';
import { useNotify } from './notify';
import { useOpenAssistant } from './open';
import { useSession } from './session';

/**
 * Push through OneSignal, for a desktop that is not reachable while the app is closed
 * (`docs/resident-agent-design.md` §13.3). Nothing here sees what a notice says: the desktop
 * sends it to OneSignal at the level the user chose, and what arrives in `data` is ids.
 *
 * Off — and the rest of the app unaffected — on the web build, and until a OneSignal app id is
 * set in `app.json` (`extra.oneSignalAppId`). Never run on a device by this change: see the
 * design document's section on what is unverified.
 */
type PushState = 'unavailable' | 'off' | 'on' | 'denied';

interface PushValue {
  state: PushState;
  /** The id notices for this phone are addressed to, once there is one. */
  pushId: string | null;
}

const Ctx = createContext<PushValue>({ state: 'unavailable', pushId: null });
const KEY = 'orochi.push';
const native = Platform.OS !== 'web';
const appId = (): string => {
  const extra = Constants.expoConfig?.extra as { oneSignalAppId?: unknown } | undefined;
  return typeof extra?.oneSignalAppId === 'string' ? extra.oneSignalAppId.trim() : '';
};

/** One random id for this phone, kept in the secure store like the install id. */
export async function pushId(): Promise<string> {
  const had = await getSecret(KEY);
  if (had && validPushId(had)) return had;
  const c = (globalThis as { crypto?: { getRandomValues?: (a: Uint8Array) => void } }).crypto;
  const id = newPushId((bytes) => {
    if (c?.getRandomValues) c.getRandomValues(bytes);
    else for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
  });
  await setSecret(KEY, id);
  return id;
}

// The SDK is a native module: required only where it exists, so the web build never loads it.
function sdk(): any | null {
  if (!native || !appId()) return null;
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    return require('react-native-onesignal').OneSignal ?? null;
  } catch {
    return null;
  }
}

export function PushProvider({ children }: { children: React.ReactNode }) {
  const { enabled } = useNotify();
  const { client, machines, active } = useSession();
  const openAssistant = useOpenAssistant();
  const [state, setState] = useState<PushState>(sdk() ? 'off' : 'unavailable');
  const [id, setId] = useState<string | null>(null);
  const started = useRef(false);
  const told = useRef(new Set<string>());
  const machinesRef = useRef(machines);
  machinesRef.current = machines;

  // Starts OneSignal once the person has turned notifications on, and signs this phone in under
  // its push id: that is what the desktop addresses.
  useEffect(() => {
    const OneSignal = sdk();
    if (!OneSignal || !enabled) return;
    let cancelled = false;
    (async () => {
      const mine = await pushId();
      if (cancelled) return;
      setId(mine);
      if (!started.current) {
        started.current = true;
        OneSignal.initialize(appId());
        // The app is open and already shows what is new: a push on top of it would say it twice.
        OneSignal.Notifications.addEventListener('foregroundWillDisplay', (event: any) =>
          event.preventDefault(),
        );
        OneSignal.Notifications.addEventListener('click', (event: any) => {
          const tap = pushTap(event?.notification?.additionalData);
          if (!tap?.thread) return;
          const machine = machinesRef.current.find((m) => m.machineId === tap.machine);
          // A notice is about a routine or a resident: it opens as the assistant's conversation.
          openAssistant(machine?.key ?? null, tap.thread);
        });
      }
      const granted = await OneSignal.Notifications.getPermissionAsync().catch(() => false);
      if (cancelled) return;
      if (!granted) {
        const asked = await OneSignal.Notifications.requestPermission(true).catch(() => false);
        if (!asked) return setState('denied');
      }
      OneSignal.login(mine);
      setState('on');
    })().catch(() => setState('off'));
    return () => {
      cancelled = true;
    };
  }, [enabled]); // eslint-disable-line react-hooks/exhaustive-deps

  // Each desktop is told the id once per run, as it is paired with or switched to.
  const tell = useCallback(async () => {
    if (state !== 'on' || !id || !client || !active) return;
    if (told.current.has(`${active.key}:${id}`)) return;
    told.current.add(`${active.key}:${id}`);
    await client.registerPush(id).catch(() => told.current.delete(`${active.key}:${id}`));
  }, [state, id, client, active]);
  useEffect(() => {
    tell();
  }, [tell]);

  // Notifications turned off here: this phone signs out of OneSignal, so nothing is pushed.
  useEffect(() => {
    if (enabled || !started.current) return;
    sdk()?.logout?.();
    setState('off');
  }, [enabled]);

  return <Ctx.Provider value={{ state, pushId: id }}>{children}</Ctx.Provider>;
}

export const usePush = () => useContext(Ctx);
