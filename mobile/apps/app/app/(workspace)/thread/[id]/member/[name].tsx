import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  Text,
  View,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from 'react-native';
import { useLocalSearchParams } from 'expo-router';
import * as Clipboard from 'expo-clipboard';
import {
  buildMemberThread,
  latestReplyAt,
  mentionCandidates,
  newReplyCount,
  needsLabel,
  plural,
  firstUnreadIndex,
  threadView,
  type ThreadLine,
  type Voice,
} from '@orochi/core';
import { useNow } from '../../../../../src/state/now';
import { usePrefs } from '../../../../../src/state/prefs';
import { useSession } from '../../../../../src/state/session';
import { useThreadData, type ThreadData } from '../../../../../src/state/thread';
import { useThreadSeen } from '../../../../../src/state/threadSeen';
import { Composer } from '../../../../../src/ui/Composer';
import { DayRow, DividerView, PostView, WorkView } from '../../../../../src/ui/Entries';
import { LiveFace } from '../../../../../src/ui/people/LiveFace';
import { PeopleProvider, usePeople } from '../../../../../src/ui/people/People';
import { Empty, Header } from '../../../../../src/ui/Screen';
import { Sheet } from '../../../../../src/ui/Sheet';

export default function MemberThread() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const data = useThreadData(id);
  return (
    <PeopleProvider thread={data.thread?.thread} members={data.members}>
      <MemberBody data={data} />
    </PeopleProvider>
  );
}

function MemberBody({ data }: { data: ThreadData }) {
  const { id, name } = useLocalSearchParams<{ id: string; name: string }>();
  const { api, active } = useSession();
  const { colors, t, lang } = usePrefs();
  const now = useNow();
  const [also, setAlso] = useState(false);
  const [pressed, setPressed] = useState<Voice | null>(null);
  const { thread, said, members } = data;
  const { seen, ready, mark } = useThreadSeen(id);
  const built = useMemo(
    () => (thread ? buildMemberThread({ thread, said, members, name }) : null),
    [thread, said, members, name],
  );
  const view = useMemo(() => (built ? threadView(built.lines) : null), [built]);
  const known = useMemo(
    () => new Set(['lead', ...members.map((m) => m.name.toLowerCase())]),
    [members],
  );
  const ctx = useMemo(
    () => ({ thread: id, cwd: thread?.thread.cwd, known }),
    [id, thread?.thread.cwd, known],
  );

  // Where the reader is: at the bottom, or scrolled up with replies that arrived since.
  const list = useRef<FlatList<ThreadLine>>(null);
  // `stick`: the reader was at the bottom when the last scroll ended, so new content is followed.
  const stick = useRef(true);
  const geo = useRef({ y: 0, view: 0, content: 0 });
  const done = useRef(false);
  const initialSeen = useRef<number | null | undefined>(undefined);
  const [laidOut, setLaidOut] = useState(false);
  const [anchor, setAnchor] = useState<number | null>(null);
  const [away, setAway] = useState(false);
  const latest = built ? latestReplyAt(built.lines) : null;
  const replies = view?.replies ?? [];

  // The web build never reports a content size; a short wait stands in for it.
  useEffect(() => {
    if (!view) return;
    const timer = setTimeout(() => setLaidOut(true), 120);
    return () => clearTimeout(timer);
  }, [!!view]); // eslint-disable-line react-hooks/exhaustive-deps

  // Opening: the first reply not seen yet (or the bottom), then everything counts as seen.
  useEffect(() => {
    if (!ready || !view || initialSeen.current !== undefined) return;
    initialSeen.current = seen[name.toLowerCase()] ?? null;
  }, [ready, view, seen, name]);
  useEffect(() => {
    if (done.current || !laidOut || !ready || !view || initialSeen.current === undefined) return;
    done.current = true;
    const at = firstUnreadIndex(replies, initialSeen.current);
    if (at !== null && at > 1) {
      list.current?.scrollToIndex({ index: at, viewPosition: 0.1, animated: false });
      stick.current = false;
    } else if (at !== null) {
      list.current?.scrollToOffset({ offset: 0, animated: false });
      stick.current = false;
    } else {
      list.current?.scrollToEnd({ animated: false });
    }
    setAnchor(latest);
    mark(name, latest);
    setAway(geo.current.content > 0 ? !atEnd() : !stick.current);
  }, [laidOut, ready, view, replies, latest, mark, name]);

  // New replies: follow them at the bottom, otherwise offer to jump.
  useEffect(() => {
    if (!done.current || latest === null) return;
    if (stick.current) {
      setAnchor(latest);
      mark(name, latest);
    }
  }, [latest, mark, name]);
  const pending = built && away ? newReplyCount(built.lines, anchor) : 0;

  const atEnd = () => geo.current.y + geo.current.view >= geo.current.content - 48;
  const onScroll = useCallback(
    (e: NativeSyntheticEvent<NativeScrollEvent>) => {
      const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
      geo.current = {
        y: contentOffset.y,
        view: layoutMeasurement.height,
        content: contentSize.height,
      };
      const end = atEnd();
      stick.current = end;
      setAway(!end);
      if (end && latest !== null) {
        setAnchor(latest);
        mark(name, latest);
      }
    },
    [latest, mark, name],
  );
  const onContent = (_w: number, h: number) => {
    geo.current.content = h;
    setLaidOut(true);
    if (done.current && stick.current) list.current?.scrollToEnd({ animated: false });
    else if (done.current) setAway(!atEnd());
  };

  const render = ({ item }: { item: ThreadLine }) => {
    switch (item.type) {
      case 'post':
        return (
          <PostView
            voice={item.voice}
            run={item.run}
            ctx={ctx}
            report={item.report}
            inThread
            onLongPress={setPressed}
          />
        );
      case 'work':
        return <WorkView items={item.items} tools={item.tools} thoughts={item.thoughts} quiet />;
      case 'divider':
        return <DividerView at={item.at} where={item.where} />;
      case 'day':
        return <DayRow at={item.at} />;
      case 'parent':
        return null;
    }
  };

  const count = view?.count ?? 0;
  const head = (
    <View>
      {view?.parent ? (
        <PostView voice={view.parent} run={false} ctx={ctx} inThread onLongPress={setPressed} />
      ) : null}
      <View
        style={{
          flexDirection: 'row',
          alignItems: 'center',
          gap: 10,
          marginTop: 12,
          marginBottom: 6,
          paddingHorizontal: 12,
        }}
      >
        <Text testID="thread-count" style={{ color: colors.sub, fontSize: 13, fontWeight: '700' }}>
          {count === 0 ? t('No replies yet') : plural(count, 'reply', 'replies', lang)}
        </Text>
        <View style={{ flex: 1, height: 1, backgroundColor: colors.line }} />
      </View>
    </View>
  );
  const foot = <LiveLine name={name} />;

  const needs = built?.head.needs;
  return (
    <KeyboardAvoidingView
      style={{ flex: 1, backgroundColor: colors.bg }}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    >
      <Header title={t('Thread')} subtitle={thread?.thread.title || t('New conversation')} />
      {needs ? (
        <View style={{ backgroundColor: colors.code, padding: 8, paddingHorizontal: 12 }}>
          <Text style={{ color: colors.text, fontWeight: '700', fontSize: 13 }}>
            {needsLabel(needs, lang)}
          </Text>
        </View>
      ) : null}
      {built && built.lines.length === 0 ? <Empty text={t('Nothing here.')} /> : null}
      <View style={{ flex: 1 }}>
        <FlatList
          ref={list}
          data={replies}
          keyExtractor={(l) => l.key}
          renderItem={render}
          ListHeaderComponent={head}
          ListFooterComponent={foot}
          contentContainerStyle={{ paddingBottom: 12 }}
          keyboardShouldPersistTaps="handled"
          initialNumToRender={Math.max(30, replies.length)}
          onContentSizeChange={onContent}
          onLayout={(e) => {
            geo.current.view = e.nativeEvent.layout.height;
          }}
          onScroll={onScroll}
          scrollEventThrottle={64}
          onScrollToIndexFailed={(info) =>
            list.current?.scrollToOffset({
              offset: info.averageItemLength * info.index,
              animated: false,
            })
          }
        />
        {pending > 0 && away ? (
          <Pressable
            onPress={() => {
              stick.current = true;
              setAway(false);
              setAnchor(latest);
              mark(name, latest);
              list.current?.scrollToEnd({ animated: true });
            }}
            accessibilityRole="button"
            accessibilityLabel={`${t('New replies')}: ${pending}`}
            testID="new-replies"
            style={{
              position: 'absolute',
              alignSelf: 'center',
              bottom: 10,
              minHeight: 36,
              paddingHorizontal: 16,
              borderRadius: 18,
              justifyContent: 'center',
              backgroundColor: colors.primary,
            }}
          >
            <Text style={{ color: colors.onPrimary, fontWeight: '700', fontSize: 13 }}>
              ↓ {t('New replies')} · {pending}
            </Text>
          </Pressable>
        ) : null}
      </View>
      <Composer
        placeholder={t('Reply…')}
        draftKey={`${active?.key ?? ''}|${id}|${name.toLowerCase()}`}
        candidates={(q) => mentionCandidates(q, members, thread)}
        also={{ label: t('Also send to the channel'), value: also, onChange: setAlso }}
        onSend={async (text) => {
          await api!.say(id, text, null, name);
          if (also) {
            await api!.send(id, `${name}: ${text}`);
            await api!.ensureHost(id).catch(() => {});
          }
          setAlso(false);
          await api!.seeMentions(id);
          stick.current = true;
          await data.reload();
        }}
      />
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
              ]
            : []
        }
      />
    </KeyboardAvoidingView>
  );
}

/** The member's live work, quietly: its face and what it is doing, only while it works. */
function LiveLine({ name }: { name: string }) {
  const { colors, t } = usePrefs();
  const people = usePeople();
  const m = people.member(name);
  if (!m?.live) return null;
  const doing = (m.doing || '').split('\n')[0];
  return (
    <View
      testID="thread-live"
      style={{ flexDirection: 'row', alignItems: 'center', gap: 8, padding: 12, opacity: 0.75 }}
    >
      <LiveFace who={people.who(name)} state={people.state(name)} size={20} />
      <Text style={{ color: colors.sub, fontSize: 12, flexShrink: 1 }} numberOfLines={1}>
        {name} {t('working')}
        {doing ? ` — ${doing}` : '…'}
      </Text>
    </View>
  );
}
