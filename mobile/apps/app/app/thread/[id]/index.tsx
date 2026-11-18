import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { FlatList, KeyboardAvoidingView, Platform, View } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
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
import * as Clipboard from 'expo-clipboard';
import { useData } from '../../../src/state/data';
import { usePrefs } from '../../../src/state/prefs';
import { toneFill } from '../../../src/ui/theme';
import { useSession } from '../../../src/state/session';
import { useThreadData, type ThreadData } from '../../../src/state/thread';
import { Composer } from '../../../src/ui/Composer';
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
} from '../../../src/ui/Entries';
import { Empty, HEADER, Header, HeaderButton } from '../../../src/ui/Screen';
import { EdgeOpen } from '../../../src/ui/Drawer';
import { PeopleProvider } from '../../../src/ui/people/People';
import { Sheet } from '../../../src/ui/Sheet';
import { ThreadFooter, ThreadsPill } from '../../../src/ui/Threads';
import { useNow } from '../../../src/state/now';
import { useThreadSeen } from '../../../src/state/threadSeen';
import { ThreadMenu } from '../../../src/ui/ThreadMenu';

export default function Conversation() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const data = useThreadData(id);
  return (
    <PeopleProvider thread={data.thread?.thread} members={data.members}>
      <ConversationBody data={data} />
    </PeopleProvider>
  );
}

function ConversationBody({ data }: { data: ThreadData }) {
  const { id, reply } = useLocalSearchParams<{ id: string; reply?: string }>();
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
        title={thread?.thread.title || t('New conversation')}
        subtitle={
          thread ? `${thread.thread.project} · ${t(STATE_LABELS[thread.thread.status])}` : undefined
        }
        right={
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
              onPress: () => router.push({ pathname: '/thread/[id]/gallery', params: { id } }),
            },
          ],
        }}
      />
    </KeyboardAvoidingView>
  );
}
export { HEADER };
