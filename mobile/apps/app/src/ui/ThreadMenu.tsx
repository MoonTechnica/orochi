import React, { useRef, useState } from 'react';
import type { Member, Overrides, ThreadRow } from '@orochi/protocol';
import { EFFORTS, routeChoices, routeWith } from '@orochi/core';
import { useData } from '../state/data';
import { usePrefs } from '../state/prefs';
import { useSession } from '../state/session';
import { Drawer, DrawerItem, DrawerSection } from './Drawer';
import { Sheet, TextPrompt, type SheetAction } from './Sheet';
import { Text, View } from 'react-native';

/**
 * Rename, pin, archive and route for one conversation. `extra` are screen-specific entries
 * (Files, Git, Work) put first.
 */
export function ThreadMenu({
  thread: current,
  visible,
  onClose,
  extra = [],
  onChanged,
  drawer,
}: {
  thread: Pick<
    ThreadRow,
    'id' | 'title' | 'pinned' | 'archived' | 'overrides' | 'project' | 'branch' | 'status'
  > | null;
  /** Shown as a right-hand drawer (a conversation's own panel) instead of a bottom sheet. */
  drawer?: {
    statusLabel: string;
    nav: (SheetAction & { badge?: string })[];
  };
  visible: boolean;
  onClose: () => void;
  extra?: SheetAction[];
  onChanged?: () => void;
}) {
  const { api } = useSession();
  const { refresh } = useData();
  const { t, colors } = usePrefs();
  const [renaming, setRenaming] = useState(false);
  const [routing, setRouting] = useState(false);
  const [efforting, setEfforting] = useState(false);
  const [choices, setChoices] = useState<{ label: string; route: string | null }[]>([]);
  // The parent drops its selection when the sheet closes; the follow-up prompts still need it.
  const kept = useRef(current);
  if (current) kept.current = current;
  const thread = current ?? kept.current;
  if (!thread) return null;
  const done = async (work: Promise<unknown> | undefined) => {
    try {
      await work;
    } catch {
      /* the next refresh shows what is true */
    }
    await refresh();
    onChanged?.();
  };
  const pinned: Overrides = thread.overrides ?? ({} as Overrides);
  const openRoutes = async () => {
    if (!api) return;
    try {
      const [routes, agents] = await Promise.all([api.insights(), api.agents()]);
      setChoices(routeChoices(routes, agents));
    } catch {
      setChoices(routeChoices([], []));
    }
    setRouting(true);
  };
  const pick = (route: string | null) =>
    done(api?.setRoute(thread.id, route).then(() => api?.ensureHost(thread.id)));
  const actions: SheetAction[] = [
    ...extra,
    { label: t('Rename'), onPress: () => setRenaming(true) },
    {
      label: thread.pinned ? t('Unpin') : t('Pin'),
      onPress: () => done(api?.pinThread(thread.id, !thread.pinned)),
    },
    {
      label: thread.archived ? t('Unarchive') : t('Archive'),
      onPress: () => done(api?.archiveThread(thread.id, !thread.archived)),
    },
    {
      label: `${t('Route')}: ${pinned.agent ? [pinned.agent, pinned.model].filter(Boolean).join(' · ') : t('Auto')}`,
      onPress: openRoutes,
    },
  ];
  if (pinned.agent)
    actions.push({
      label: `${t('Effort')}: ${pinned.reasoning ?? t('default')}`,
      onPress: () => setEfforting(true),
    });
  const manage = actions.slice(extra.length);
  const sheetOrDrawer = drawer ? (
    <Drawer visible={visible} onClose={onClose} label={thread.title}>
      <View style={{ padding: 18, gap: 4 }}>
        <Text numberOfLines={3} style={{ color: colors.text, fontSize: 18, fontWeight: '800' }}>
          {thread.title || t('New conversation')}
        </Text>
        <Text style={{ color: colors.sub, fontSize: 13 }}>{thread.project}</Text>
        {thread.branch ? (
          <Text style={{ color: colors.sub, fontSize: 13 }}>⎇ {thread.branch}</Text>
        ) : null}
        <Text style={{ color: colors.sub, fontSize: 13 }}>{drawer.statusLabel}</Text>
      </View>
      <DrawerSection title={t('Open')}>
        {drawer.nav.map((a) => (
          <DrawerItem
            key={a.label}
            label={a.label}
            badge={a.badge}
            onPress={() => {
              onClose();
              a.onPress();
            }}
          />
        ))}
      </DrawerSection>
      <DrawerSection title={t('Manage')}>
        {manage.map((a, i) => (
          <DrawerItem
            key={`${i}-${a.label}`}
            label={a.label}
            danger={a.danger}
            onPress={() => {
              onClose();
              a.onPress();
            }}
          />
        ))}
      </DrawerSection>
    </Drawer>
  ) : (
    <Sheet visible={visible} title={thread.title} actions={actions} onClose={onClose} />
  );
  return (
    <>
      {sheetOrDrawer}
      <TextPrompt
        visible={renaming}
        title={t('Rename')}
        initial={thread.title}
        confirm={t('Save')}
        onClose={() => setRenaming(false)}
        onSubmit={(text) => text && done(api?.renameThread(thread.id, text))}
      />
      <Sheet
        visible={routing}
        title={t('Route')}
        onClose={() => setRouting(false)}
        actions={choices.map((c) => ({
          label: c.route === null ? t('Auto') : c.label,
          id: c.route ?? 'auto',
          selected:
            c.route === null
              ? !pinned.agent
              : c.route === [pinned.agent, pinned.model].filter(Boolean).join('/'),
          onPress: () => pick(c.route),
        }))}
      />
      <Sheet
        visible={efforting}
        title={t('Effort')}
        onClose={() => setEfforting(false)}
        actions={EFFORTS.map((e) => ({
          label: e ?? t('default'),
          selected: (pinned.reasoning ?? null) === e,
          onPress: () => pick(routeWith(pinned.agent!, pinned.model, e)),
        }))}
      />
    </>
  );
}
