import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { Pressable, RefreshControl, ScrollView, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import type { AgentRow, RouteStats, Talk } from '@orochi/protocol';
import {
  activityTree,
  buildHome,
  groupTrail,
  agentNamed,
  hhmm,
  leadRoleOf,
  since,
  span,
  taskTitle,
  toMs,
  whoOf,
} from '@orochi/core';
import { useData } from '../../../src/state/data';
import { usePrefs } from '../../../src/state/prefs';
import { useSession } from '../../../src/state/session';
import { AskCard } from '../../../src/ui/Entries';
import { MiniAvatar } from '../../../src/ui/office/Avatar';
import { PeopleProvider } from '../../../src/ui/people/People';
import { WhoFace } from '../../../src/ui/people/WhoFace';
import { Empty } from '../../../src/ui/Screen';
import { useAttention } from '../../../src/state/attention';
import { OfficeCard } from '../../../src/ui/home/OfficeCard';
import { Serpent } from '../../../src/ui/home/Serpent';
import { Trail } from '../../../src/ui/home/Trail';
import { Disclosure, DisclosureRow } from '../../../src/ui/Disclosure';

type Tab = 'now' | 'mentions' | 'chatter' | 'routes' | 'connection' | 'recent';

export default function Activity() {
  const { projects, prompts, mentions, seats, tasks, refresh, connection, error } = useData();
  const { api, active } = useSession();
  const { snapshots } = useAttention();
  const snapshot = snapshots.find((s) => s.key === active?.key);
  const { agents, recent } = useMemo(() => buildHome(snapshot ? [snapshot] : []), [snapshot]);
  const { colors, t } = usePrefs();
  const router = useRouter();
  const [tab, setTab] = useState<Tab | null>(null);
  const toggle = (key: Tab) => setTab((current) => (current === key ? null : key));
  const [talk, setTalk] = useState<Talk[]>([]);
  const [stats, setStats] = useState<RouteStats[]>([]);
  const [agentRows, setAgentRows] = useState<AgentRow[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const rows = useMemo(
    () => new Map(projects.flatMap((p) => p.threads.map((th) => [th.id, th] as const))),
    [projects],
  );

  const load = useCallback(async () => {
    if (!api) return;
    try {
      const [c, s, a] = await Promise.all([
        api.chatter(Date.now() - 6 * 3600 * 1000),
        api.insights(),
        api.agents().catch(() => [] as AgentRow[]),
      ]);
      setTalk(c);
      setStats(s);
      setAgentRows(a);
    } catch {
      /* the lists stay as they were */
    }
  }, [api]);
  useEffect(() => {
    if (tab === 'chatter' || tab === 'routes') load();
  }, [tab, load, seats.length]);

  const tree = useMemo(() => activityTree(seats, tasks, projects), [seats, tasks, projects]);
  const open = (id: string) => router.push({ pathname: '/thread/[id]', params: { id } });

  const answer = async (id: string, option: string | null) => {
    await api?.answer(id, option);
    await refresh();
  };

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <ScrollView
        contentContainerStyle={{ padding: 16, paddingBottom: 16 }}
        refreshControl={
          <RefreshControl
            refreshing={refreshing}
            tintColor={colors.text}
            onRefresh={async () => {
              setRefreshing(true);
              await Promise.all([refresh(), load()]);
              setRefreshing(false);
            }}
          />
        }
      >
        {snapshot?.state === 'online' ? (
          <OfficeCard snapshots={[snapshot]} onPress={() => router.push('/office')} />
        ) : (
          <DisclosureRow
            title={t('Office')}
            testID="office-card"
            onPress={() => router.push('/office')}
          />
        )}
        {active ? (
          <Disclosure
            title={t('Connection')}
            summary={t(
              connection === 'open'
                ? 'Connected'
                : connection === 'connecting'
                  ? 'Reconnecting'
                  : 'Offline',
            )}
            expanded={tab === 'connection'}
            onPress={() => toggle('connection')}
          >
            <View style={{ paddingHorizontal: 12 }}>
              <Text style={{ color: colors.sub, fontSize: 14 }}>
                {active.host}:{active.port}
              </Text>
              {error ? (
                <Text accessibilityRole="alert" style={{ color: colors.sub }}>
                  {error}
                </Text>
              ) : null}
              <Pressable
                accessibilityRole="button"
                onPress={refresh}
                style={{ minHeight: 44, justifyContent: 'center' }}
              >
                <Text style={{ color: colors.text, fontWeight: '600' }}>{t('Retry')}</Text>
              </Pressable>
            </View>
          </Disclosure>
        ) : null}
        {recent.length ? (
          <Disclosure
            title={t('Recent conversations')}
            expanded={tab === 'recent'}
            onPress={() => toggle('recent')}
          >
            <Trail groups={groupTrail(recent)} many={false} onOpen={(_, id) => open(id)} />
          </Disclosure>
        ) : null}
        <Disclosure title={t('Seats')} expanded={tab === 'now'} onPress={() => toggle('now')}>
          {tab === 'now' ? (
            <>
              {agents.length ? (
                <Serpent agents={agents} onOpen={(agent) => open(agent.thread)} />
              ) : null}
              {prompts.map((p) => (
                <View key={p.id}>
                  <Pressable onPress={() => open(p.thread_id)}>
                    <Text
                      style={{
                        color: colors.sub,
                        fontSize: 12,
                        paddingHorizontal: 16,
                        paddingTop: 8,
                      }}
                    >
                      {p.project} · {p.thread_title}
                    </Text>
                  </Pressable>
                  <PeopleProvider thread={rows.get(p.thread_id)}>
                    <AskCard prompt={p} onAnswer={answer} />
                  </PeopleProvider>
                </View>
              ))}
              {tree.length === 0 && prompts.length === 0 ? <Empty text={t('All quiet.')} /> : null}
              {tree.map((node) => (
                <View
                  key={node.thread_id}
                  style={{
                    marginHorizontal: 12,
                    marginTop: 10,
                    borderRadius: 12,
                    backgroundColor: colors.card,
                    borderWidth: 1,
                    borderColor: colors.line,
                    overflow: 'hidden',
                  }}
                >
                  <Pressable
                    onPress={() => open(node.thread_id)}
                    style={{ padding: 12, minHeight: 48 }}
                  >
                    <Text
                      style={{ color: colors.text, fontWeight: '700', fontSize: 15 }}
                      numberOfLines={1}
                    >
                      {node.title}
                    </Text>
                    <Text style={{ color: colors.sub, fontSize: 12 }}>{node.project}</Text>
                  </Pressable>
                  {node.seats.map(({ seat, tasks: sub }) => (
                    <View key={seat.seat_id}>
                      <View
                        style={{
                          flexDirection: 'row',
                          gap: 10,
                          paddingHorizontal: 12,
                          paddingVertical: 8,
                          alignItems: 'center',
                          borderTopWidth: 1,
                          borderTopColor: colors.line,
                        }}
                      >
                        <WhoFace
                          who={whoOf(
                            {
                              name: seat.role,
                              role: seat.role,
                              agent: seat.agent,
                              provider: seat.provider,
                            },
                            seat.lead ? seat.role : leadRoleOf(rows.get(seat.thread_id)),
                          )}
                          size={30}
                          state={seat.state === 'asking' ? 'asking' : 'working'}
                        />
                        <View style={{ flex: 1 }}>
                          <Text style={{ color: colors.text, fontWeight: '600' }} numberOfLines={1}>
                            {seat.role}
                            {seat.title ? (
                              <Text style={{ color: colors.sub, fontWeight: '400' }}>
                                {' '}
                                · {seat.title}
                              </Text>
                            ) : null}
                          </Text>
                          <Text
                            style={{
                              color: seat.state === 'asking' ? colors.attention : colors.sub,
                              fontSize: 12,
                            }}
                            numberOfLines={1}
                          >
                            {seat.doing?.split('\n')[0] || seat.state} · {since(seat.started_at)}
                          </Text>
                        </View>
                        {seat.state === 'asking' ? (
                          <Text style={{ color: colors.text, fontWeight: '700' }}>●</Text>
                        ) : null}
                      </View>
                      {sub.map((task) => (
                        <View
                          key={task.item_id}
                          style={{
                            paddingLeft: 46,
                            paddingRight: 12,
                            paddingVertical: 6,
                            flexDirection: 'row',
                            alignItems: 'center',
                            gap: 6,
                          }}
                        >
                          <MiniAvatar
                            title={task.title || String(task.item_id)}
                            agent={seat.agent ?? ''}
                          />
                          <Text
                            style={{ color: colors.sub, fontSize: 13, flex: 1 }}
                            numberOfLines={1}
                          >
                            {t('sub-agent')} · {taskTitle(task.title)} · {since(task.started_at)}
                          </Text>
                        </View>
                      ))}
                    </View>
                  ))}
                </View>
              ))}
            </>
          ) : null}
        </Disclosure>
        <Disclosure
          title={t('Mentions')}
          expanded={tab === 'mentions'}
          onPress={() => toggle('mentions')}
        >
          {tab === 'mentions' ? (
            <>
              {mentions.length === 0 ? <Empty text={t('Nothing here.')} /> : null}
              {mentions.map((m) => {
                const th = rows.get(m.thread_id);
                const from = agentNamed(th, m.member ?? m.who);
                const who = whoOf(
                  { name: m.member ?? m.who, role: from?.role ?? m.member ?? m.who, ...from },
                  leadRoleOf(th),
                );
                return (
                  <Pressable
                    key={m.message_id}
                    onPress={() => open(m.thread_id)}
                    style={{
                      marginHorizontal: 12,
                      marginTop: 8,
                      padding: 12,
                      borderRadius: 12,
                      backgroundColor: colors.card,
                      borderWidth: 1,
                      borderColor: colors.attention,
                      gap: 4,
                    }}
                  >
                    <Text style={{ color: colors.sub, fontSize: 12 }}>
                      {m.project} · {m.thread_title}
                    </Text>
                    <View style={{ flexDirection: 'row', gap: 10, alignItems: 'flex-start' }}>
                      <WhoFace who={who} size={34} />
                      <View style={{ flex: 1, gap: 4 }}>
                        <Text style={{ color: colors.text, fontSize: 15 }}>
                          <Text style={{ fontWeight: '700' }}>{m.member ?? m.who}</Text> {m.text}
                        </Text>
                        <Text style={{ color: colors.sub, fontSize: 12 }}>{since(toMs(m.at))}</Text>
                      </View>
                    </View>
                  </Pressable>
                );
              })}
            </>
          ) : null}
        </Disclosure>
        <Disclosure
          title={t('Chatter')}
          expanded={tab === 'chatter'}
          onPress={() => toggle('chatter')}
        >
          {tab === 'chatter' ? (
            <>
              {talk.length === 0 ? <Empty text={t('Nothing here.')} /> : null}
              {talk.map((m) => (
                <View
                  key={m.seq}
                  style={{
                    paddingHorizontal: 16,
                    paddingVertical: 8,
                    borderBottomWidth: 1,
                    borderBottomColor: colors.line,
                  }}
                >
                  <Text style={{ color: colors.sub, fontSize: 12 }}>
                    {m.from_role || m.from} → {m.to_role || m.to} · {hhmm(m.at)}
                  </Text>
                  <Text style={{ color: colors.text, fontSize: 14 }} numberOfLines={4}>
                    {m.text}
                  </Text>
                </View>
              ))}
            </>
          ) : null}
        </Disclosure>
        <Disclosure title={t('Usage')} expanded={tab === 'routes'} onPress={() => toggle('routes')}>
          {tab === 'routes' ? (
            <>
              {stats.length === 0 && agentRows.length === 0 ? (
                <Empty text={t('Nothing here.')} />
              ) : null}
              {agentRows.map((a, i) => (
                <View
                  key={`a${i}`}
                  style={{
                    marginHorizontal: 12,
                    marginTop: 8,
                    padding: 12,
                    borderRadius: 12,
                    backgroundColor: colors.card,
                    borderWidth: 1,
                    borderColor: colors.line,
                    gap: 3,
                  }}
                >
                  <Text style={{ color: colors.text, fontWeight: '700' }}>
                    {a.agent} · {a.model}
                  </Text>
                  <Text style={{ color: colors.sub, fontSize: 13 }}>
                    {a.status}
                    {a.cooling ? ` · cooling ${a.cooling}` : ''}
                    {a.failures ? ` · ✗${a.failures}` : ''}
                  </Text>
                </View>
              ))}
              {stats.map((s, i) => (
                <View
                  key={i}
                  style={{
                    marginHorizontal: 12,
                    marginTop: 8,
                    padding: 12,
                    borderRadius: 12,
                    backgroundColor: colors.card,
                    borderWidth: 1,
                    borderColor: colors.line,
                    gap: 3,
                  }}
                >
                  <Text style={{ color: colors.text, fontWeight: '700' }}>
                    {s.agent} · {s.model}
                    {s.reasoning ? ` · ${s.reasoning}` : ''}
                  </Text>
                  <Text style={{ color: colors.sub, fontSize: 13 }}>{s.task_type}</Text>
                  <Text style={{ color: colors.sub, fontSize: 13 }}>
                    {t('success')} {Math.round(s.success_rate * 100)}% · ✓{s.verified} ✗{s.failures}{' '}
                    ~{s.weak} · {Math.round(s.mean_tokens / 1000)}k · {span(s.mean_duration_ms)}
                  </Text>
                </View>
              ))}
            </>
          ) : null}
        </Disclosure>
      </ScrollView>
    </View>
  );
}
