import React, { useCallback, useEffect, useState } from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { since } from '@orochi/core';
import type { GitOverview } from '@orochi/protocol';
import { usePrefs } from '../../../src/state/prefs';
import { useSession } from '../../../src/state/session';
import { useData } from '../../../src/state/data';
import { Empty, Header } from '../../../src/ui/Screen';
import { mono } from '../../../src/ui/theme';

export default function Git() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { api } = useSession();
  const { colors, t } = usePrefs();
  const { onChanged } = useData();
  const router = useRouter();
  const [git, setGit] = useState<GitOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    if (!api) return;
    try {
      setGit(await api.gitOverview(id));
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }, [api, id]);
  useEffect(() => {
    load();
  }, [load]);
  useEffect(
    () => onChanged((ids) => (ids.length === 0 || ids.includes(id)) && load()),
    [onChanged, id, load],
  );

  const head = (text: string) => (
    <Text
      style={{
        color: colors.sub,
        fontSize: 13,
        fontWeight: '700',
        textTransform: 'uppercase',
        padding: 16,
        paddingBottom: 6,
      }}
    >
      {text}
    </Text>
  );
  const pill = (text: string, color: string) => (
    <View
      style={{
        borderRadius: 8,
        paddingHorizontal: 6,
        paddingVertical: 1,
        backgroundColor: color + '22',
      }}
    >
      <Text style={{ color, fontSize: 11, fontWeight: '700' }}>{text}</Text>
    </View>
  );
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title="Git" subtitle={git?.branch ?? undefined} />
      {error ? <Empty text={error} /> : null}
      {git ? (
        <ScrollView contentContainerStyle={{ paddingBottom: 40 }}>
          <View style={{ flexDirection: 'row', gap: 10, padding: 16 }}>
            {pill(`${git.staged} ${t('staged')}`, colors.text)}
            {pill(`${git.unstaged} ${t('unstaged')}`, colors.text)}
            {pill(`${git.untracked} ${t('untracked')}`, colors.sub)}
          </View>
          {git.worktrees.length > 1 ? (
            <>
              {head(t('Worktrees'))}
              {git.worktrees.map((w) => (
                <View
                  key={w.path}
                  style={{
                    padding: 12,
                    paddingHorizontal: 16,
                    backgroundColor: colors.card,
                    borderBottomWidth: 1,
                    borderBottomColor: colors.line,
                  }}
                >
                  <Text
                    style={{ color: colors.text, fontWeight: w.current ? '700' : '400' }}
                    numberOfLines={1}
                  >
                    {w.branch ?? w.head.slice(0, 8)} {w.primary ? '· primary' : ''}
                  </Text>
                  <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={1}>
                    {w.path}
                  </Text>
                </View>
              ))}
            </>
          ) : null}
          {head(t('Branches'))}
          {git.branches.map((b) => (
            <View
              key={b.name}
              style={{
                padding: 12,
                paddingHorizontal: 16,
                backgroundColor: colors.card,
                borderBottomWidth: 1,
                borderBottomColor: colors.line,
              }}
            >
              <Text style={{ color: colors.text, fontWeight: b.current ? '700' : '400' }}>
                {b.current ? '● ' : ''}
                {b.name}
              </Text>
              <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={1}>
                {b.subject}
              </Text>
            </View>
          ))}
          {head(t('Commits'))}
          {git.commits.map((c) => (
            <Pressable
              key={c.hash}
              onPress={() =>
                router.push({ pathname: '/viewer/file', params: { thread: id, commit: c.hash } })
              }
              accessibilityRole="button"
              accessibilityLabel={c.subject}
              style={{
                padding: 12,
                paddingHorizontal: 16,
                backgroundColor: colors.card,
                borderBottomWidth: 1,
                borderBottomColor: colors.line,
                gap: 3,
              }}
            >
              <Text style={{ color: colors.text, fontSize: 15 }} numberOfLines={2}>
                {c.subject}
              </Text>
              <View
                style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap', alignItems: 'center' }}
              >
                <Text style={{ color: colors.sub, fontFamily: mono, fontSize: 12 }}>
                  {c.hash.slice(0, 8)}
                </Text>
                {c.refs.map((r) => (
                  <React.Fragment key={r}>
                    {pill(r.replace('HEAD -> ', ''), colors.text)}
                  </React.Fragment>
                ))}
                <Text style={{ color: colors.sub, fontSize: 12 }}>{since(c.at * 1000)}</Text>
              </View>
            </Pressable>
          ))}
        </ScrollView>
      ) : null}
    </View>
  );
}
