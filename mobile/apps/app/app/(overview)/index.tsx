import React, { useCallback, useState } from 'react';
import { Pressable, RefreshControl, ScrollView, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { RemoteClient, api, baseUrl } from '@orochi/protocol';
import { dismissMention, type NeedItem } from '@orochi/core';
import { useAttention } from '../../src/state/attention';
import { useOpenThread } from '../../src/state/open';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { Disclosure, DisclosureRow } from '../../src/ui/Disclosure';
import { ConnectionBadge, useStateLabel } from '../../src/ui/Machines';
import { HeaderButton } from '../../src/ui/Screen';
import { Icon } from '../../src/ui/Icon';
import { Logo } from '../../src/ui/Logo';
import { WorkspaceName } from '../../src/ui/OverviewCards';
import { NeedCard } from '../../src/ui/home/Needs';
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
      <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
        <Logo size={64} />
        <HeaderButton
          testID="open-settings"
          icon="settings-outline"
          label={t('Settings')}
          onPress={() => router.push('/settings')}
        />
      </View>
      <Text
        accessibilityRole="header"
        style={{ color: colors.text, fontSize: 32, fontWeight: '800' }}
      >
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
      {button(t('Paste a link or enter details'), () => router.push('/pair'))}
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
      accessibilityRole="header"
      style={{
        color: colors.text,
        fontSize: 18,
        fontWeight: '700',
        paddingHorizontal: 18,
        marginTop: 12,
        marginBottom: 8,
      }}
    >
      {children}
    </Text>
  );
}

export default function Home() {
  const { colors, t } = usePrefs();
  const { machines, switchTo } = useSession();
  const { home, refreshAll } = useAttention();
  const open = useOpenThread();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [refreshing, setRefreshing] = useState(false);
  const [seen, setSeen] = useState<string[]>([]);
  const [all, setAll] = useState(false);
  const stateLabel = useStateLabel();

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
  const renderNeed = (item: NeedItem) => (
    <NeedCard
      key={item.id}
      item={item}
      many={many}
      onAnswer={answer}
      onOpen={(entry) => open(entry.machineKey, entry.thread)}
      onReply={(entry) => open(entry.machineKey, entry.thread, replyTo(entry))}
      onDismiss={dismiss}
    />
  );

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
        <View
          style={{
            flexDirection: 'row',
            alignItems: 'center',
            gap: 10,
            paddingHorizontal: 18,
            paddingVertical: 8,
          }}
        >
          <Logo size={34} />
          <Text
            accessibilityRole="header"
            style={{
              color: colors.text,
              fontWeight: '800',
              fontSize: 28,
              letterSpacing: -0.6,
              flex: 1,
            }}
          >
            {t('Home')}
          </Text>
          <HeaderButton
            testID="open-settings"
            icon="settings-outline"
            label={t('Settings')}
            onPress={() => router.push('/settings')}
          />
        </View>

        {needs.length ? (
          <View>
            <Label>{t('Needs you')}</Label>
            <View style={{ paddingHorizontal: 18, gap: 10 }}>
              {renderNeed(needs[0]!)}
              {needs.length > 1 ? (
                <Disclosure
                  title={t('{n} more waiting').replace('{n}', String(needs.length - 1))}
                  expanded={all}
                  onPress={() => setAll((value) => !value)}
                  testID="needs-more"
                >
                  <View style={{ gap: 10 }}>{needs.slice(1).map(renderNeed)}</View>
                </Disclosure>
              ) : null}
            </View>
          </View>
        ) : null}

        <Label>{t('Workspaces')}</Label>
        <View
          style={{
            marginHorizontal: 18,
            borderRadius: radius.card,
            overflow: 'hidden',
            backgroundColor: colors.card,
            borderWidth: 1,
            borderColor: colors.line,
          }}
        >
          {home.desktops.map((desktop) => (
            <View
              key={desktop.key}
              style={{ borderBottomWidth: 1, borderBottomColor: colors.line }}
            >
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`${desktop.name}, ${stateLabel(desktop.state)}`}
                testID={`workspace-row-${desktop.key}`}
                onPress={async () => {
                  await switchTo(desktop.key);
                  router.navigate('/conversations');
                }}
                style={({ pressed }) => ({
                  minHeight: 56,
                  padding: 12,
                  flexDirection: 'row',
                  alignItems: 'center',
                  gap: 8,
                  backgroundColor: pressed ? colors.raised : 'transparent',
                })}
              >
                <Icon name="desktop-outline" size={18} color={colors.sub} />
                <View style={{ flex: 1, gap: 3 }}>
                  <WorkspaceName workspace={desktop} />
                  {desktop.active > 0 && desktop.state === 'online' ? (
                    <Text style={{ color: colors.sub, fontSize: 12 }}>
                      {t(
                        desktop.active === 1 ? '1 active conversation' : '{n} active conversations',
                      ).replace('{n}', String(desktop.active))}
                    </Text>
                  ) : null}
                </View>
                <ConnectionBadge state={desktop.state} />
                <Icon name="chevron-forward" size={14} color={colors.sub} />
              </Pressable>
            </View>
          ))}
          <DisclosureRow
            title={t('Add a desktop')}
            icon="add"
            testID="nest-add"
            onPress={() => router.push({ pathname: '/pair', params: { add: '1' } })}
          />
        </View>
      </ScrollView>
    </View>
  );
}
