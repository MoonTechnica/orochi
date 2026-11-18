import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { AppState, Platform } from 'react-native';
import * as Notifications from 'expo-notifications';
import {
  badgeCount,
  noticesFor,
  snapshotOf,
  titleIn,
  withMachine,
  type Notice,
  type Snapshot,
} from '@orochi/core';
import { getSecret, setSecret } from '../lib/storage';
import { useAttention } from './attention';
import { useData } from './data';
import { useOpenThread } from './open';
import { useSession } from './session';
import { usePrefs } from './prefs';

interface NotifyValue {
  enabled: boolean;
  /** 'denied' when the system refused; the toggle then cannot turn on. */
  permission: 'granted' | 'denied' | 'unknown';
  setEnabled(on: boolean): Promise<void>;
}

const Ctx = createContext<NotifyValue | null>(null);
const KEY = 'orochi.notify';
const native = Platform.OS !== 'web';

if (native) {
  Notifications.setNotificationHandler({
    handleNotification: async () => ({
      shouldShowBanner: true,
      shouldShowList: true,
      shouldPlaySound: false,
      shouldSetBadge: true,
    }),
  });
}

/**
 * Local notifications only: they are raised from what this app already reads over the tailnet,
 * never through a push service, so no content leaves the tailnet. They exist while the app is
 * running or recently backgrounded; a killed app receives none.
 */
export function NotifyProvider({ children }: { children: React.ReactNode }) {
  const { projects, prompts, mentions, loaded } = useData();
  const { t } = usePrefs();
  const { machines, active } = useSession();
  const { others } = useAttention();
  const openThread = useOpenThread();
  const [enabled, setOn] = useState(false);
  const [permission, setPermission] = useState<NotifyValue['permission']>('unknown');
  const before = useRef<Snapshot | null>(null);
  const fired = useRef(new Set<string>());

  useEffect(() => {
    getSecret(KEY).then((v) => setOn(v === 'on'));
    if (native) {
      Notifications.getPermissionsAsync()
        .then((p) => setPermission(p.granted ? 'granted' : p.canAskAgain ? 'unknown' : 'denied'))
        .catch(() => {});
    }
  }, []);

  const setEnabled = useCallback(async (on: boolean) => {
    if (on && native) {
      let p = await Notifications.getPermissionsAsync();
      if (!p.granted) p = await Notifications.requestPermissionsAsync();
      setPermission(p.granted ? 'granted' : 'denied');
      if (!p.granted) return;
    }
    setOn(on);
    await setSecret(KEY, on ? 'on' : 'off');
    if (!on && native) await Notifications.setBadgeCountAsync(0).catch(() => {});
  }, []);

  const fire = (n: Notice, machine?: string) => {
    if (fired.current.has(`${machine}:${n.key}`)) return;
    fired.current.add(`${machine}:${n.key}`);
    Notifications.scheduleNotificationAsync({
      content: {
        title: n.title,
        body:
          n.kind === 'prompt'
            ? `${t('Needs you')}: ${n.body}`
            : n.kind === 'finished'
              ? t(n.body)
              : n.body,
        data: { thread: n.thread, machine },
      },
      trigger: null,
    }).catch(() => {});
  };

  // The other machines: only questions and mentions are known for them (polled).
  const beforeOthers = useRef<Record<string, Snapshot>>({});
  useEffect(() => {
    for (const m of machines) {
      const seen = others[m.key];
      if (!seen || seen.state !== 'online') continue;
      const notices = noticesFor(
        beforeOthers.current[m.key] ?? null,
        [],
        seen.prompts,
        seen.mentions,
      );
      beforeOthers.current[m.key] = snapshotOf([], seen.prompts, seen.mentions);
      if (!enabled || !native) continue;
      const name = titleIn({ machines, activeKey: active?.key ?? null }, m);
      for (const n of notices) fire(withMachine(n, name, machines.length), m.key);
    }
  }, [others]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!loaded) return;
    const next = snapshotOf(projects, prompts, mentions);
    const notices = noticesFor(before.current, projects, prompts, mentions);
    before.current = next;
    if (!enabled || !native) return;
    Notifications.setBadgeCountAsync(
      badgeCount(prompts, mentions) +
        Object.values(others).reduce((n, o) => n + badgeCount(o.prompts, o.mentions), 0),
    ).catch(() => {});
    const mine = active ? titleIn({ machines, activeKey: active.key }, active) : '';
    for (const n of notices) fire(withMachine(n, mine, machines.length), active?.key);
  }, [projects, prompts, mentions, loaded, enabled]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!native) return;
    const open = (data: any) => {
      if (typeof data?.thread === 'string') openThread(data.machine, data.thread);
    };
    Notifications.getLastNotificationResponseAsync()
      .then((r) => r && open(r.notification.request.content.data))
      .catch(() => {});
    const sub = Notifications.addNotificationResponseReceivedListener((r) =>
      open(r.notification.request.content.data),
    );
    const app = AppState.addEventListener('change', (s) => {
      if (s === 'active') Notifications.dismissAllNotificationsAsync().catch(() => {});
    });
    return () => {
      sub.remove();
      app.remove();
    };
  }, [openThread]);

  return <Ctx.Provider value={{ enabled, permission, setEnabled }}>{children}</Ctx.Provider>;
}

export function useNotify(): NotifyValue {
  const v = useContext(Ctx);
  if (!v) throw new Error('NotifyProvider missing');
  return v;
}
