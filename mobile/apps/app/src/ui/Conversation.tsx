import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { FlatList, KeyboardAvoidingView, Platform, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import {
  STATE_LABELS,
  STATUS_TONE,
  buildTimeline,
  galleryOf,
  mentionCandidates,
  summarizeThreads,
  unreadThreads,
  withDays,
  type Entry,
  type Voice,
} from '@orochi/core';
import type { ResidentRow, RoutineRow } from '@orochi/protocol';
import * as Clipboard from 'expo-clipboard';
import { useData } from '../state/data';
import { usePrefs } from '../state/prefs';
import { toneFill } from './theme';
import { useSession } from '../state/session';
import { useThreadData, type ThreadData } from '../state/thread';
import { Composer } from './Composer';
import {
  AskCard,
  AsideView,
  ChecksView,
  DayRow,
  EntryMedia,
  FootView,
  LiveView,
  PostView,
  ReportsLine,
  RouteChip,
  WorkView,
} from './Entries';
import { Empty, Header, HeaderButton } from './Screen';
import { EdgeOpen } from './Drawer';
import { PeopleProvider } from './people/People';
import { Sheet } from './Sheet';
import { ThreadFooter, ThreadsPill } from './Threads';
import { useNow } from '../state/now';
import { useThreadSeen } from '../state/threadSeen';
import { ThreadMenu } from './ThreadMenu';
import { ProposalCard } from './assistant/Bubbles';

/**
 * One conversation, as the phone draws it: the channel, its members and their threads. An
 * assistant is the same screen — its team is the conversation's own — with what only it has: a
 * pause switch, and the routines an agent proposed for the person to allow. It has no gallery,
 * changes, files, git or work, which are for code.
 */
export function ConversationView({
  id,
  reply,
  switchable,
}: {
  id: string;
  reply?: string;
  switchable?: { residents: ResidentRow[]; onSwitch: (thread: string) => void };
}) {
  const data = useThreadData(id);
  return (
    <PeopleProvider thread={data.thread?.thread} members={data.members}>
      <ConversationBody id={id} reply={reply} data={data} switchable={switchable} />
    </PeopleProvider>
  );
}

function ConversationBody({
  id,
  reply,
  data,
  switchable,
}: {
  id: string;
  reply?: string;
  data: ThreadData;
  switchable?: { residents: ResidentRow[]; onSwitch: (thread: string) => void };
}) {
  const { api } = useSession();
  const { prompts, refresh } = useData();
  const { colors, t, lang } = usePrefs();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [menu, setMenu] = useState(false);
  const [pressed, setPressed] = useState<Voice | null>(null);
  const { thread, said, members } = data;
  const now = useNow();
  const { seen } = useThreadSeen(id);
  const [me, setMe] = useState<ResidentRow | null>(null);
  const [routines, setRoutines] = useState<RoutineRow[]>([]);
  const assistant = me !== null;
  const loadResident = useCallback(async () => {
    if (!api) return;
    const [all, rows] = await Promise.all([
      api.residents().catch(() => null),
      api.routines().catch(() => null),
    ]);
    setMe(all?.find((r) => r.thread_id === id) ?? null);
    if (rows) setRoutines(rows.filter((r) => r.thread_id === id && r.proposed));
  }, [api, id]);
  // A proposal arrives with the reply that made it, so it is read again as the conversation
  // moves, and now and then for a run that ended elsewhere.
  useEffect(() => {
    loadResident();
  }, [loadResident, data.thread?.items.length, data.thread?.thread.status]);
  useEffect(() => {
    const timer = setInterval(loadResident, 15000);
    return () => clearInterval(timer);
  }, [loadResident]);

  useEffect(() => {
    // Opening a conversation reads it: the unread count and mentions clear.
    if (api && thread && (thread.thread.unread > 0 || thread.thread.mentions)) {
      api
        .seen(id)
        .then(() => api.seeMentions(id))
        .then(refresh)
        .catch(() => {});
    }
  }, [api, id, thread?.thread.unread, thread?.thread.mentions]); // eslint-disable-line react-hooks/exhaustive-deps

  const entries = useMemo(
    () => (thread ? withDays(buildTimeline({ thread, said, members, prompts })) : []),
    [thread, said, members, prompts],
  );
  const summaries = useMemo(
    () => (thread ? summarizeThreads({ thread, said, members }, lang) : []),
    [thread, said, members, lang],
  );
  const unread = unreadThreads(summaries, seen) > 0;
  const known = useMemo(
    () => new Set(['lead', ...members.map((m) => m.name.toLowerCase())]),
    [members],
  );
  const ctx = useMemo(
    () => ({ thread: id, cwd: thread?.thread.cwd, known }),
    [id, thread?.thread.cwd, known],
  );
  const gallery = useMemo(
    () => (thread ? galleryOf(thread.items, { thread: id, cwd: thread.thread.cwd }) : []),
    [thread, id],
  );
  const working = thread?.thread.status === 'working';

  const member = (name: string) =>
    router.push({ pathname: '/thread/[id]/member/[name]', params: { id, name } });
  const openMedia = (mid: string) =>
    router.push({ pathname: '/viewer/media', params: { thread: id, id: mid } });
  const answer = useCallback(
    async (prompt: string, option: string | null) => {
      await api?.answer(prompt, option);
      await api?.seeMentions(id);
      await data.reload();
      await refresh();
    },
    [api, id, data, refresh],
  );

  const render = ({ item }: { item: Entry }) => {
    switch (item.type) {
      case 'post':
        return (
          <PostView
            voice={item.voice}
            run={item.run}
            ctx={ctx}
            onLongPress={setPressed}
            onOpenThread={member}
          />
        );
      case 'day':
        return <DayRow at={item.at} />;
      case 'bar': {
        const s = summaries.find((x) => x.name.toLowerCase() === item.bar.name.toLowerCase());
        return s ? (
          <ThreadFooter
            s={s}
            seenAt={seen[s.name.toLowerCase()]}
            now={now}
            onPress={() => member(s.name)}
          />
        ) : null;
      }
      case 'route':
        return <RouteChip text={item.text} resumed={item.resumed} />;
      case 'work':
        return <WorkView items={item.items} tools={item.tools} thoughts={item.thoughts} />;
      case 'aside':
        return <AsideView notes={item.notes} label={item.label} />;
      case 'checks':
        return <ChecksView entry={item} />;
      case 'media':
        return <EntryMedia entry={item} thread={id} onOpen={openMedia} />;
      case 'reports':
        return <ReportsLine entry={item} onMember={member} />;
      case 'foot':
        return (
          <FootView
            entry={item}
            onFiles={() => router.push({ pathname: '/thread/[id]/changes', params: { id } })}
          />
        );
      case 'live':
        return <LiveView entry={item} />;
      case 'ask':
        return <AskCard prompt={item.prompt} onAnswer={answer} />;
    }
  };

  const tone = thread ? toneFill(colors, STATUS_TONE[thread.thread.status]) : colors.sub;
  return (
    <KeyboardAvoidingView
      style={{ flex: 1, backgroundColor: colors.bg }}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      keyboardVerticalOffset={0}
    >
      <Header
        // In the Assistant tab there is nowhere to go back to.
        back={!switchable}
        title={me?.name ?? (thread?.thread.title || t('New conversation'))}
        subtitle={
          assistant
            ? working
              ? t('Working…')
              : me?.paused
                ? t('paused')
                : undefined
            : thread
              ? `${thread.thread.project} · ${t(STATE_LABELS[thread.thread.status])}`
              : undefined
        }
        right={
          <>
            {assistant ? null : (
              <>
                <HeaderButton
                  icon="images"
                  label={t('Gallery')}
                  badge={gallery.length || undefined}
                  onPress={() => router.push({ pathname: '/thread/[id]/gallery', params: { id } })}
                />
                <HeaderButton
                  icon="git-compare"
                  label={t('Changes')}
                  onPress={() => router.push({ pathname: '/thread/[id]/changes', params: { id } })}
                />
              </>
            )}
            <HeaderButton
              icon="ellipsis-horizontal"
              label={t('More')}
              onPress={() => setMenu(true)}
            />
          </>
        }
      />
      {summaries.length > 0 ? (
        <ThreadsPill
          count={summaries.length}
          unread={unread}
          onPress={() => router.push({ pathname: '/thread/[id]/threads', params: { id } })}
        />
      ) : null}
      {!thread && !data.loading ? <Empty text={data.error ?? t('Nothing here.')} /> : null}
      <FlatList
        style={{ flex: 1 }}
        inverted
        data={[...entries].reverse()}
        keyExtractor={(e) => e.key}
        renderItem={render}
        keyboardShouldPersistTaps="handled"
        // A long conversation keeps about seven screens of rows (and faces) mounted, not twenty-one.
        initialNumToRender={14}
        windowSize={7}
        maxToRenderPerBatch={8}
        contentContainerStyle={{ paddingVertical: 8 }}
        ListHeaderComponent={
          routines.length ? (
            <View>
              {routines.map((r) => (
                <ProposalCard
                  key={r.id}
                  routine={r}
                  onAnswer={async (allow) => {
                    await api?.routineAction(r.id, allow ? 'approve' : 'decline');
                    await loadResident();
                    await data.reload();
                  }}
                />
              ))}
            </View>
          ) : undefined
        }
      />
      <Composer
        placeholder={t('Message…')}
        focus={Boolean(reply)}
        initial={reply && reply !== '-' ? `@${reply} ` : undefined}
        working={working}
        candidates={(q) => mentionCandidates(q, members, thread)}
        onInterrupt={() => api?.interrupt(id)}
        onSend={async (text) => {
          await api!.send(id, text);
          await api!.ensureHost(id).catch(() => {});
          await data.reload();
        }}
      />
      <View style={{ height: 0, backgroundColor: tone }} />
      <EdgeOpen onOpen={() => setMenu(true)} />
      <Sheet
        visible={pressed !== null}
        title={pressed?.who}
        onClose={() => setPressed(null)}
        actions={
          pressed
            ? [
                {
                  label: t('Copy text'),
                  onPress: () => {
                    Clipboard.setStringAsync(pressed.text).catch(() => {});
                  },
                },
                ...(pressed.thread && summaries.some((x) => x.name === pressed.thread)
                  ? [{ label: t('Reply in thread'), onPress: () => member(pressed.thread!) }]
                  : []),
              ]
            : []
        }
      />
      <ThreadMenu
        thread={thread?.thread ?? null}
        visible={menu}
        onClose={() => setMenu(false)}
        onChanged={data.reload}
        drawer={{
          statusLabel: thread ? t(STATE_LABELS[thread.thread.status]) : '',
          nav: [
            {
              label: `${t('Threads')} (${summaries.length})`,
              badge: unread ? '•' : undefined,
              onPress: () => router.push({ pathname: '/thread/[id]/threads', params: { id } }),
            },
            ...(assistant
              ? [
                  // Pausing is the one switch; anything finer is asked of the assistant.
                  {
                    label: me?.paused ? t('Resume') : t('Pause everything it has scheduled'),
                    onPress: async () => {
                      if (!me) return;
                      await api?.residentAction(me.name, me.paused ? 'resume' : 'pause');
                      await loadResident();
                    },
                  },
                  ...(switchable?.residents ?? [])
                    .filter((r) => r.thread_id !== id)
                    .map((r) => ({
                      label: t('Talk to {name}').replace('{name}', r.name),
                      onPress: () => switchable?.onSwitch(r.thread_id),
                    })),
                ]
              : [
                  {
                    label: t('Files'),
                    onPress: () => router.push({ pathname: '/thread/[id]/files', params: { id } }),
                  },
                  {
                    label: 'Git',
                    onPress: () => router.push({ pathname: '/thread/[id]/git', params: { id } }),
                  },
                  {
                    label: t('Work'),
                    onPress: () => router.push({ pathname: '/thread/[id]/work', params: { id } }),
                  },
                  {
                    label: t('Gallery'),
                    badge: gallery.length ? String(gallery.length) : undefined,
                    onPress: () =>
                      router.push({ pathname: '/thread/[id]/gallery', params: { id } }),
                  },
                ]),
          ],
        }}
      />
    </KeyboardAvoidingView>
  );
}
