import React, { useCallback, useState } from 'react';
import { Pressable, RefreshControl, ScrollView, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { RemoteClient, api, baseUrl } from '@orochi/protocol';
import { dismissMention, groupTrail, headCounts, type NeedItem } from '@orochi/core';
import { useAttention } from '../../src/state/attention';
import { useOpenThread } from '../../src/state/open';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { Logo } from '../../src/ui/Logo';
import { OfficeCard } from '../../src/ui/home/OfficeCard';
import { Nests } from '../../src/ui/home/Nests';
import { NeedCard } from '../../src/ui/home/Needs';
import { Serpent } from '../../src/ui/home/Serpent';
import { Trail } from '../../src/ui/home/Trail';
import { radius } from '../../src/ui/theme';

function Guide() {
  const { colors, t } = usePrefs();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const steps = [
    t('On your Mac, open Orochi: Settings → Mobile and turn it on.'),
    t('Put this phone and the Mac on the same Tailscale network.'),
    t('Scan the QR code the Mac shows.'),
  ];
  const button = (label: string, onPress: () => void, primary = false) => (
    <Pressable
      accessibilityRole="button"
      onPress={onPress}
      style={{
        minHeight: 52,
        borderRadius: radius.pill,
        alignItems: 'center',
        justifyContent: 'center',
        backgroundColor: primary ? colors.primary : 'transparent',
        borderWidth: primary ? 0 : 1,
        borderColor: colors.line,
      }}
    >
      <Text
        style={{ color: primary ? colors.onPrimary : colors.text, fontSize: 16, fontWeight: '700' }}
      >
        {label}
      </Text>
    </Pressable>
  );
  return (
    <ScrollView
      style={{ flex: 1, backgroundColor: colors.bg }}
      contentContainerStyle={{ padding: 22, paddingTop: insets.top + 28, gap: 18 }}
    >
      <Logo size={84} />
      <Text style={{ color: colors.text, fontSize: 32, fontWeight: '800' }}>
        {t('Pair a desktop to begin')}
      </Text>
      <Text style={{ color: colors.sub, fontSize: 15, lineHeight: 22 }}>
        {t('Orochi on your phone reads and drives the conversations your Mac is running.')}
      </Text>
      <View style={{ paddingVertical: 4 }}>
        <View
          style={{
            position: 'absolute',
            left: 13,
            top: 18,
            bottom: 18,
            width: 2,
            backgroundColor: colors.line,
          }}
        />
        {steps.map((s, i) => (
          <View
            key={i}
            style={{ flexDirection: 'row', gap: 14, alignItems: 'center', minHeight: 56 }}
          >
            <View
              style={{
                width: 28,
                height: 28,
                borderRadius: 10,
                borderWidth: 2,
                borderColor: i === steps.length - 1 ? colors.attention : colors.primary,
                backgroundColor: colors.bg,
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <Text style={{ color: colors.text, fontWeight: '800', fontSize: 12 }}>{i + 1}</Text>
            </View>
            <Text style={{ color: colors.text, fontSize: 15, flex: 1 }}>{s}</Text>
          </View>
        ))}
      </View>
      {button(
        t('Scan QR code'),
        () => router.push({ pathname: '/pair', params: { scan: '1' } }),
        true,
      )}
      {button(t('Paste a link'), () => router.push('/pair'))}
      {button(t('Enter the details'), () => router.push('/pair'))}
    </ScrollView>
  );
}

/** The `reply` param of a conversation: who to address, or '-' to just focus the composer. */
const replyTo = (n: NeedItem) =>
  n.mention && /^[A-Za-z0-9._-]{1,40}$/.test(n.mention.who) ? n.mention.who : '-';

function Label({ children }: { children: string }) {
  const { colors } = usePrefs();
  return (
    <Text
      style={{
        color: colors.sub,
        fontSize: 12,
        fontWeight: '700',
        letterSpacing: 1.2,
        paddingHorizontal: 18,
        marginTop: 28,
        marginBottom: 12,
      }}
    >
      {children.toUpperCase()}
    </Text>
  );
}

export default function Home() {
  const { colors, t } = usePrefs();
  const { machines, switchTo } = useSession();
  const { home, snapshots, refreshAll } = useAttention();
  const open = useOpenThread();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [refreshing, setRefreshing] = useState(false);
  const [seen, setSeen] = useState<string[]>([]);
  const [all, setAll] = useState(false);

  const clientFor = useCallback(
    (key: string) => {
      const m = machines.find((x) => x.key === key);
      return m ? api(new RemoteClient({ base: baseUrl(m.host, m.port), token: m.token })) : null;
    },
    [machines],
  );
  const answer = async (n: NeedItem, option: string | null) => {
    const c = clientFor(n.machineKey);
    if (!c || !n.prompt) throw new Error('no desktop');
    await c.answer(n.prompt.id, option);
    await refreshAll();
  };
  const dismiss = async (n: NeedItem) => {
    const plan = dismissMention(home.needs, seen, n);
    setSeen(plan.dismissed);
    if (!plan.seeThread) return;
    await clientFor(n.machineKey)
      ?.seeMentions(n.thread)
      .catch(() => {});
    await refreshAll();
  };

  if (machines.length === 0) return <Guide />;
  const many = machines.length > 1;
  const needs = home.needs.filter((n) => !seen.includes(n.id));
  const counts = headCounts(home.agents);
  const reachable = home.desktops.some((d) => d.state === 'online');
  const fill = (key: string, n: number) => t(key).replace('{n}', String(n));
  const parts = [
    counts.need
      ? fill(counts.need === 1 ? '{n} head needs you' : '{n} heads need you', counts.need)
      : '',
    counts.working ? fill('{n} working', counts.working) : '',
    counts.background ? fill('{n} watching', counts.background) : '',
  ].filter(Boolean);
  const headline = !reachable
    ? t('Out of reach')
    : parts.length
      ? parts.join(' · ')
      : t('All quiet');
  const sub = !reachable
    ? t('No desktop answers right now.')
    : parts.length
      ? t('Tap a head to open it')
      : t('Nothing is running or waiting.');

  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <ScrollView
        contentContainerStyle={{ paddingTop: insets.top + 8, paddingBottom: 36 }}
        refreshControl={
          <RefreshControl
            refreshing={refreshing}
            tintColor={colors.text}
            onRefresh={async () => {
              setRefreshing(true);
              await refreshAll();
              setRefreshing(false);
            }}
          />
        }
      >
        <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 18 }}>
          <Logo size={40} />
          <Text style={{ color: colors.text, fontWeight: '800', fontSize: 20 }}>Orochi</Text>
        </View>

        <View style={{ marginTop: 10 }}>
          <Serpent agents={home.agents} onOpen={(a) => open(a.machineKey, a.thread)} />
        </View>
        <View style={{ paddingHorizontal: 18, marginTop: 6 }} accessibilityRole="header">
          <Text
            testID="headline"
            style={{
              color: colors.text,
              fontWeight: '800',
              letterSpacing: -0.6,
              fontSize: 26,
              lineHeight: 31,
            }}
          >
            {headline}
          </Text>
          <Text style={{ color: colors.sub, fontSize: 14, marginTop: 4 }}>{sub}</Text>
        </View>

        {needs.length ? (
          <View style={{ paddingHorizontal: 18, marginTop: 22, gap: 12 }}>
            {(all ? needs : needs.slice(0, 2)).map((n) => (
              <NeedCard
                key={n.id}
                item={n}
                many={many}
                onAnswer={answer}
                onOpen={(x) => open(x.machineKey, x.thread)}
                onReply={(x) => open(x.machineKey, x.thread, replyTo(x))}
                onDismiss={dismiss}
              />
            ))}
            {needs.length > 2 ? (
              <Pressable
                testID="needs-more"
                accessibilityRole="button"
                onPress={() => setAll(!all)}
                style={{ minHeight: 44, justifyContent: 'center' }}
              >
                <Text style={{ color: colors.text, fontWeight: '700' }}>
                  {all
                    ? t('Show fewer')
                    : t('{n} more waiting').replace('{n}', String(needs.length - 2))}
                </Text>
              </Pressable>
            ) : null}
          </View>
        ) : null}

        {reachable ? (
          <OfficeCard snapshots={snapshots} onPress={() => router.push('/office')} />
        ) : null}

        <Label>{t('Nests')}</Label>
        <Nests
          desktops={home.desktops}
          onOpen={async (d) => {
            await switchTo(d.key);
            router.navigate('/conversations');
          }}
          onRetry={() => refreshAll()}
          onRepair={(d) => router.push({ pathname: '/pair', params: { repair: d.key } })}
          onAdd={() => router.push({ pathname: '/pair', params: { add: '1' } })}
        />

        <Label>{t('The trail')}</Label>
        {home.recent.length === 0 ? (
          <Text style={{ color: colors.sub, paddingHorizontal: 18 }}>{t('Nothing here.')}</Text>
        ) : (
          <Trail groups={groupTrail(home.recent)} many={many} onOpen={(k, id) => open(k, id)} />
        )}
        <Pressable
          accessibilityRole="button"
          onPress={() => router.navigate('/conversations')}
          style={{ minHeight: 44, justifyContent: 'center', paddingHorizontal: 18, marginTop: 8 }}
        >
          <Text style={{ color: colors.text, fontWeight: '700' }}>{t('All conversations')} →</Text>
        </Pressable>
      </ScrollView>
    </View>
  );
}
