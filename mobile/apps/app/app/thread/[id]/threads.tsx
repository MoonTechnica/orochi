import React, { useMemo } from 'react';
import { FlatList, View } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { orderThreads, summarizeThreads } from '@orochi/core';
import { useNow } from '../../../src/state/now';
import { usePrefs } from '../../../src/state/prefs';
import { useThreadData } from '../../../src/state/thread';
import { useThreadSeen } from '../../../src/state/threadSeen';
import { PeopleProvider } from '../../../src/ui/people/People';
import { Empty, Header } from '../../../src/ui/Screen';
import { ThreadCard } from '../../../src/ui/Threads';

/** Slack's Threads view for one conversation: every member thread, what needs you and what is unread first. */
export default function Threads() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const data = useThreadData(id);
  const { colors, t, lang } = usePrefs();
  const router = useRouter();
  const now = useNow();
  const { seen } = useThreadSeen(id);
  const { thread, said, members } = data;
  const rows = useMemo(
    () => (thread ? orderThreads(summarizeThreads({ thread, said, members }, lang), seen) : []),
    [thread, said, members, lang, seen],
  );
  return (
    <PeopleProvider thread={thread?.thread} members={members}>
      <View style={{ flex: 1, backgroundColor: colors.bg }}>
        <Header title={t('Threads')} subtitle={thread?.thread.title || t('New conversation')} />
        {thread && rows.length === 0 ? <Empty text={t('No threads yet.')} /> : null}
        <FlatList
          data={rows}
          keyExtractor={(s) => s.name}
          renderItem={({ item }) => (
            <ThreadCard
              s={item}
              seenAt={seen[item.name.toLowerCase()]}
              now={now}
              onPress={() =>
                router.push({
                  pathname: '/thread/[id]/member/[name]',
                  params: { id, name: item.name },
                })
              }
            />
          )}
          contentContainerStyle={{ paddingVertical: 6 }}
        />
      </View>
    </PeopleProvider>
  );
}
