import React, { useCallback, useEffect, useState } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { useLocalSearchParams } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import type { ResidentRow } from '@orochi/protocol';
import { usePrefs } from '../../src/state/prefs';
import { useSession } from '../../src/state/session';
import { ConversationView } from '../../src/ui/Conversation';
import { Empty } from '../../src/ui/Screen';
import { radius } from '../../src/ui/theme';

/** Which resident each desktop last showed, while the app runs. */
const chosen = new Map<string, string>();

/**
 * The assistant: a resident's conversation, which is the phone's own conversation screen — its
 * channel, its members and their threads (`ui/Conversation.tsx`). What it proposes waits there
 * as a card, and a push about it opens here. This file only finds the resident, and makes the
 * first one.
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
    <ConversationView
      key={id}
      id={id}
      switchable={{
        residents,
        onSwitch: (thread) => {
          chosen.set(machine, thread);
          load();
        },
      }}
    />
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
