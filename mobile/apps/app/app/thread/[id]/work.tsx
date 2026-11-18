import React, { useCallback, useEffect, useState } from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import type { BoardPart, Team } from '@orochi/protocol';
import { usePrefs } from '../../../src/state/prefs';
import { useSession } from '../../../src/state/session';
import { useData } from '../../../src/state/data';
import { Empty, Header } from '../../../src/ui/Screen';

/** The team: helpers' worktrees, trials and the plan. Approvals show up as prompts. */
export default function Work() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const { api } = useSession();
  const { colors, t } = usePrefs();
  const { onChanged, prompts } = useData();
  const router = useRouter();
  const [team, setTeam] = useState<Team | null>(null);
  const [board, setBoard] = useState<BoardPart[]>([]);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(async () => {
    if (!api) return;
    try {
      const [tm, bd] = await Promise.all([api.team(id), api.board(id)]);
      setTeam(tm);
      setBoard(bd ?? []);
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

  const asks = prompts.filter((p) => p.thread_id === id);
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
  const card = (key: string, children: React.ReactNode) => (
    <View
      key={key}
      style={{
        padding: 12,
        paddingHorizontal: 16,
        backgroundColor: colors.card,
        borderBottomWidth: 1,
        borderBottomColor: colors.line,
        gap: 3,
      }}
    >
      {children}
    </View>
  );
  const empty =
    !team || (team.workspaces.length === 0 && team.merges.length === 0 && board.length === 0);
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Work')} />
      {error ? <Empty text={error} /> : null}
      {team && empty && asks.length === 0 ? (
        <Empty text={t('No helpers in this conversation.')} />
      ) : null}
      <ScrollView contentContainerStyle={{ paddingBottom: 40 }}>
        {asks.length > 0 ? (
          <>
            {head(t('Waiting on you'))}
            {asks.map((p) =>
              card(
                p.id,
                <Pressable onPress={() => router.back()}>
                  <Text style={{ color: colors.text, fontWeight: '700' }}>{p.title}</Text>
                  <Text style={{ color: colors.sub, fontSize: 12 }}>
                    {t('Answer it in the conversation.')}
                  </Text>
                </Pressable>,
              ),
            )}
          </>
        ) : null}
        {team && team.workspaces.length > 0 ? head(t('Worktrees')) : null}
        {team?.workspaces.map((w) =>
          card(
            w.id,
            <>
              <Text style={{ color: colors.text, fontWeight: '700' }}>
                {w.name} · {w.state}
              </Text>
              {w.title ? <Text style={{ color: colors.sub, fontSize: 13 }}>{w.title}</Text> : null}
              <Text style={{ color: colors.sub, fontSize: 12 }}>
                {[w.agent, w.model, w.seat_state].filter(Boolean).join(' · ')}
                {w.trial ? ` · ${t('trial')} ${w.trial_state ?? ''}` : ''}
              </Text>
              {w.doing ? (
                <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={2}>
                  {w.doing}
                </Text>
              ) : null}
            </>,
          ),
        )}
        {team && team.merges.length > 0 ? head(t('Trials')) : null}
        {team?.merges.map((m) =>
          card(
            m.id,
            <>
              <Text style={{ color: colors.text, fontWeight: '700' }}>
                {m.workspace} #{m.n} · {m.state}
              </Text>
              <Text style={{ color: colors.sub, fontSize: 12 }}>
                {m.files.length} {t('files')}
                {m.overlap.length ? ` · ${m.overlap.length} ${t('overlap')}` : ''}
                {m.conflicts.length ? ` · ${m.conflicts.length} ${t('conflicts')}` : ''}
              </Text>
              {m.checks.map((c) => (
                <Text
                  key={c.name}
                  style={{ color: c.passed ? colors.text : colors.dangerInk, fontSize: 12 }}
                >
                  {c.passed ? '✓' : '✗'} {c.name}
                </Text>
              ))}
              {m.error ? (
                <Text style={{ color: colors.dangerInk, fontSize: 12 }}>{m.error}</Text>
              ) : null}
            </>,
          ),
        )}
        {board.length > 0 ? head(t('Plan')) : null}
        {board.map((b) =>
          card(
            b.id,
            <>
              <Text style={{ color: colors.text }}>{b.brief}</Text>
              <Text style={{ color: colors.sub, fontSize: 12 }}>
                {b.wave != null ? `wave ${b.wave} · ` : ''}
                {b.state}
              </Text>
            </>,
          ),
        )}
      </ScrollView>
    </View>
  );
}
