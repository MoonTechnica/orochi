import React, { useCallback, useEffect, useMemo, useState } from 'react';
import {
  FlatList,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import * as Clipboard from 'expo-clipboard';
import { buildChat, chatWithDays, whenOf, type ChatLine } from '@orochi/core';
import type { ResidentRow, RoutineRow } from '@orochi/protocol';
import { useData } from '../../src/state/data';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { useThreadData } from '../../src/state/thread';
import { Composer } from '../../src/ui/Composer';
import { AskCard, DayRow } from '../../src/ui/Entries';
import { Icon } from '../../src/ui/Icon';
import { PeopleProvider } from '../../src/ui/people/People';
import { Empty } from '../../src/ui/Screen';
import { Sheet } from '../../src/ui/Sheet';
import { radius } from '../../src/ui/theme';
import {
  AgentBubble,
  MeBubble,
  ProposalCard,
  Typing,
  WokeLine,
  openSource,
} from '../../src/ui/assistant/Bubbles';

/** Which resident each desktop last showed, while the app runs. */
const chosen = new Map<string, string>();

/**
 * The assistant: one conversation with a resident, the way a messenger shows it. What its
 * routines found arrives here as its messages, what it proposes waits here as a card, and a push
 * about it opens here. Routines and notices as lists are one tap away in the menu, not the page.
 */
export default function Assistant() {
  const { thread: asked } = useLocalSearchParams<{ thread?: string }>();
  const { api, active } = useSession();
  const { t, colors } = usePrefs();
  const [residents, setResidents] = useState<ResidentRow[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const machine = active?.key ?? '';

  const load = useCallback(async () => {
    if (!api) return;
    try {
      setResidents((await api.residents()) ?? []);
      setError(null);
    } catch (e) {
      setError(String((e as Error)?.message ?? e));
      setResidents((r) => r ?? []);
    }
  }, [api]);
  useEffect(() => {
    setResidents(null);
    load();
  }, [load]);

  useEffect(() => {
    if (asked) chosen.set(machine, asked);
  }, [asked, machine]);

  const id =
    asked ||
    (residents?.some((r) => r.thread_id === chosen.get(machine)) ? chosen.get(machine) : null) ||
    residents?.[0]?.thread_id ||
    null;

  if (!api) return <Empty text={t('Pair a desktop to talk to its assistant.')} />;
  if (residents === null) return <View style={{ flex: 1, backgroundColor: colors.bg }} />;
  if (!id)
    return (
      <Start
        error={error}
        onMade={async (thread) => {
          chosen.set(machine, thread);
          await load();
        }}
      />
    );
  return (
    <Chat
      key={id}
      id={id}
      residents={residents}
      onSwitch={(thread) => {
        chosen.set(machine, thread);
        load();
      }}
      onReload={load}
    />
  );
}

function Chat({
  id,
  residents,
  onSwitch,
  onReload,
}: {
  id: string;
  residents: ResidentRow[];
  onSwitch: (thread: string) => void;
  onReload: () => Promise<void>;
}) {
  const data = useThreadData(id);
  return (
    <PeopleProvider thread={data.thread?.thread} members={data.members}>
      <ChatBody id={id} data={data} residents={residents} onSwitch={onSwitch} onReload={onReload} />
    </PeopleProvider>
  );
}

function ChatBody({
  id,
  data,
  residents,
  onSwitch,
  onReload,
}: {
  id: string;
  data: ReturnType<typeof useThreadData>;
  residents: ResidentRow[];
  onSwitch: (thread: string) => void;
  onReload: () => Promise<void>;
}) {
  const { api } = useSession();
  const { prompts, refresh } = useData();
  const { colors, t } = usePrefs();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const [routines, setRoutines] = useState<RoutineRow[]>([]);
  const [menu, setMenu] = useState(false);
  const [copy, setCopy] = useState<string | null>(null);
  const { thread, said, members } = data;
  const me = residents.find((r) => r.thread_id === id);
  const name = me?.name ?? thread?.thread.title ?? t('Assistant');

  const loadRoutines = useCallback(async () => {
    const all = await api?.routines().catch(() => null);
    if (all) setRoutines(all.filter((r) => r.thread_id === id));
  }, [api, id]);
  // A proposal arrives with the reply that made it, so the routines are read again whenever
  // the conversation moves, and now and then for a run that ended elsewhere.
  useEffect(() => {
    loadRoutines();
  }, [loadRoutines, thread?.items.length, thread?.thread.status]);
  useEffect(() => {
    const timer = setInterval(loadRoutines, 15000);
    return () => clearInterval(timer);
  }, [loadRoutines]);

  useEffect(() => {
    // Opening it reads it, as any conversation.
    if (api && thread && (thread.thread.unread > 0 || thread.thread.mentions)) {
      api
        .seen(id)
        .then(() => api.seeMentions(id))
        .then(refresh)
        .catch(() => {});
    }
  }, [api, id, thread?.thread.unread, thread?.thread.mentions]); // eslint-disable-line react-hooks/exhaustive-deps

  const bubbles = useMemo(
    () => (thread ? chatWithDays(buildChat({ thread, said, members, prompts, routines })) : []),
    [thread, said, members, prompts, routines],
  );
  const ctx = useMemo(() => ({ thread: id, cwd: thread?.thread.cwd }), [id, thread?.thread.cwd]);
  const working = thread?.thread.status === 'working';
  const next = me?.next_at ? whenOf(me.next_at) : null;

  const answer = useCallback(
    async (prompt: string, option: string | null) => {
      await api?.answer(prompt, option);
      await data.reload();
      await refresh();
    },
    [api, data, refresh],
  );
  const decide = async (routine: RoutineRow, allow: boolean) => {
    await api?.routineAction(routine.id, allow ? 'approve' : 'decline');
    await loadRoutines();
    await onReload();
  };
  const openFile = (path: string) =>
    router.push({ pathname: '/viewer/file', params: { thread: id, path } });

  const render = ({ item }: { item: ChatLine }) => {
    switch (item.type) {
      case 'day':
        return <DayRow at={item.at} />;
      case 'me':
        return <MeBubble text={item.text} />;
      case 'agent':
        return (
          <AgentBubble
            text={item.text}
            sources={item.sources}
            ctx={ctx}
            onSource={(s) => openSource(s, openFile)}
            onLongPress={() => setCopy(item.text)}
          />
        );
      case 'woke':
        return <WokeLine routine={item.routine} at={item.at} />;
      case 'typing':
        return <Typing />;
      case 'ask':
        return <AskCard prompt={item.prompt} onAnswer={answer} />;
      case 'proposal':
        return (
          <ProposalCard routine={item.routine} onAnswer={(allow) => decide(item.routine, allow)} />
        );
    }
  };

  return (
    <KeyboardAvoidingView
      style={{ flex: 1, backgroundColor: colors.bg }}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
    >
      <View
        style={{
          paddingTop: insets.top,
          backgroundColor: colors.bg,
          borderBottomWidth: 1,
          borderBottomColor: colors.line,
        }}
      >
        <View
          style={{ height: 56, flexDirection: 'row', alignItems: 'center', paddingHorizontal: 8 }}
        >
          <Pressable
            testID="assistant-menu"
            accessibilityRole="button"
            accessibilityLabel={t('Menu')}
            onPress={() => setMenu(true)}
            style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}
          >
            <Icon name="menu-outline" size={26} color={colors.text} />
          </Pressable>
          <View style={{ flex: 1, alignItems: 'center' }}>
            <Face name={name} working={working} />
          </View>
          <View style={{ width: 44 }} />
        </View>
        <Text
          numberOfLines={1}
          style={{ color: colors.sub, fontSize: 12, textAlign: 'center', paddingBottom: 8 }}
        >
          {working
            ? t('Working…')
            : me?.paused
              ? t('paused')
              : next
                ? t('Next wakes at {when}').replace('{when}', next)
                : t('Nothing scheduled — ask it to keep you up with something')}
        </Text>
      </View>
      {!thread && !data.loading ? <Empty text={data.error ?? t('Nothing here.')} /> : null}
      <FlatList
        style={{ flex: 1 }}
        inverted
        data={[...bubbles].reverse()}
        keyExtractor={(b) => b.key}
        renderItem={render}
        keyboardShouldPersistTaps="handled"
        initialNumToRender={14}
        windowSize={7}
        contentContainerStyle={{ paddingVertical: 8 }}
        ListFooterComponent={
          thread && bubbles.length === 0 ? (
            <Text style={{ color: colors.sub, textAlign: 'center', padding: 24 }}>
              {t('Ask it anything, or for something every morning: it proposes, you allow.')}
            </Text>
          ) : undefined
        }
      />
      <Composer
        placeholder={t('Message {name}').replace('{name}', name)}
        draftKey={`assistant:${id}`}
        working={working}
        candidates={() => []}
        onInterrupt={() => api?.interrupt(id)}
        onSend={async (text) => {
          await api!.send(id, text);
          await api!.ensureHost(id).catch(() => {});
          await data.reload();
        }}
      />
      <Sheet
        visible={menu}
        title={name}
        onClose={() => setMenu(false)}
        actions={[
          // There is no list of routines: the person asks the assistant to stop one. This is
          // the one switch for when that is not enough.
          ...(me
            ? [
                {
                  id: 'pause',
                  label: me.paused ? t('Resume') : t('Pause everything it has scheduled'),
                  onPress: async () => {
                    await api?.residentAction(me.name, me.paused ? 'resume' : 'pause');
                    await onReload();
                  },
                },
              ]
            : []),
          {
            id: 'details',
            label: t('Conversation details'),
            onPress: () => router.push({ pathname: '/thread/[id]', params: { id } }),
          },
          ...residents
            .filter((r) => r.thread_id !== id)
            .map((r) => ({
              id: `to-${r.thread_id}`,
              label: t('Talk to {name}').replace('{name}', r.name),
              onPress: () => onSwitch(r.thread_id),
            })),
        ]}
      />
      <Sheet
        visible={copy !== null}
        onClose={() => setCopy(null)}
        actions={[
          {
            label: t('Copy text'),
            onPress: () => {
              if (copy) Clipboard.setStringAsync(copy).catch(() => {});
            },
          },
        ]}
      />
    </KeyboardAvoidingView>
  );
}

/** The assistant's face: its initial in a round tile, breathing while it works. */
function Face({ name, working }: { name: string; working: boolean }) {
  const { colors } = usePrefs();
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
      <View
        style={{
          width: 36,
          height: 36,
          borderRadius: 18,
          backgroundColor: colors.primary,
          alignItems: 'center',
          justifyContent: 'center',
          borderWidth: working ? 2 : 0,
          borderColor: colors.live,
        }}
      >
        <Text style={{ color: colors.onPrimary, fontWeight: '800', fontSize: 16 }}>
          {name.slice(0, 1).toUpperCase()}
        </Text>
      </View>
      <Text style={{ color: colors.text, fontSize: 18, fontWeight: '800' }} numberOfLines={1}>
        {name}
      </Text>
    </View>
  );
}

/** No resident yet on this desktop: one is made here, with a name and who it is. */
function Start({
  error,
  onMade,
}: {
  error: string | null;
  onMade: (thread: string) => Promise<void>;
}) {
  const { api } = useSession();
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();
  const [name, setName] = useState('assistant');
  const [about, setAbout] = useState('');
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(error);
  const field = {
    minHeight: 48,
    borderRadius: radius.card,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.card,
    color: colors.text,
    paddingHorizontal: 14,
    fontSize: 16,
  };
  return (
    <View
      style={{
        flex: 1,
        backgroundColor: colors.bg,
        padding: 24,
        paddingTop: insets.top + 48,
        gap: 12,
      }}
    >
      <Text style={{ color: colors.text, fontSize: 28, fontWeight: '800' }}>
        {t('Your assistant')}
      </Text>
      <Text style={{ color: colors.sub, fontSize: 15, lineHeight: 22 }}>
        {t(
          'An assistant lives on your desktop with a folder and notes of its own. Talk to it here; ask it to check something every morning and it proposes a routine for you to allow.',
        )}
      </Text>
      <TextInput
        accessibilityLabel={t('Name')}
        value={name}
        onChangeText={(v) => setName(v.toLowerCase().replace(/[^a-z0-9-]/g, ''))}
        autoCapitalize="none"
        style={field}
      />
      <TextInput
        accessibilityLabel={t('Who it is')}
        placeholder={t('Who is it, and what is it for?')}
        placeholderTextColor={colors.sub}
        value={about}
        onChangeText={setAbout}
        multiline
        style={{ ...field, minHeight: 96, paddingTop: 12, textAlignVertical: 'top' }}
      />
      {message ? (
        <Text accessibilityRole="alert" style={{ color: colors.dangerInk }}>
          {message}
        </Text>
      ) : null}
      <Pressable
        testID="assistant-make"
        accessibilityRole="button"
        disabled={busy || !name}
        onPress={async () => {
          setBusy(true);
          try {
            const made = await api!.residentAdd(name, about);
            await onMade(made.thread);
          } catch (e) {
            setMessage(String((e as Error)?.message ?? e));
          } finally {
            setBusy(false);
          }
        }}
        style={{
          minHeight: 52,
          borderRadius: radius.pill,
          backgroundColor: colors.primary,
          alignItems: 'center',
          justifyContent: 'center',
          opacity: busy || !name ? 0.5 : 1,
        }}
      >
        <Text style={{ color: colors.onPrimary, fontWeight: '700', fontSize: 16 }}>
          {t('Make my assistant')}
        </Text>
      </Pressable>
    </View>
  );
}
