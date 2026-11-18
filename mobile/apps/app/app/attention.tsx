import React from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { useAttention } from '../src/state/attention';
import { useOpenThread } from '../src/state/open';
import { usePrefs } from '../src/state/prefs';
import { useSession } from '../src/state/session';
import { Dot } from '../src/ui/Machines';
import { Empty, Header } from '../src/ui/Screen';

/** Questions and @you mentions from every paired desktop, on one screen. */
export default function Attention() {
  const { colors, t } = usePrefs();
  const { machines } = useSession();
  const { waiting, states } = useAttention();
  const open = useOpenThread();
  const insets = useSafeAreaInsets();
  return (
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <Header title={t('Needs you on all desktops')} />
      <ScrollView contentContainerStyle={{ paddingBottom: insets.bottom + 24 }}>
        {waiting.length === 0 ? <Empty text={t('Nothing is waiting.')} /> : null}
        {machines.map((m) => {
          const rows = waiting.filter((w) => w.machineKey === m.key);
          if (rows.length === 0) return null;
          return (
            <View key={m.key}>
              <View
                style={{
                  flexDirection: 'row',
                  alignItems: 'center',
                  gap: 8,
                  padding: 16,
                  paddingBottom: 6,
                }}
              >
                <Dot state={states[m.key] ?? 'connecting'} />
                <Text style={{ color: colors.sub, fontSize: 13, fontWeight: '700' }}>
                  {rows[0].machineName}
                </Text>
              </View>
              {rows.map((w) => (
                <Pressable
                  key={w.id}
                  accessibilityRole="button"
                  onPress={() => open(w.machineKey, w.thread)}
                  style={{
                    paddingHorizontal: 16,
                    paddingVertical: 12,
                    minHeight: 56,
                    backgroundColor: colors.card,
                    borderTopWidth: 1,
                    borderTopColor: colors.line,
                  }}
                >
                  <Text
                    style={{ color: colors.text, fontSize: 16, fontWeight: '600' }}
                    numberOfLines={1}
                  >
                    {w.kind === 'prompt' ? '? ' : '@ '}
                    {w.title}
                  </Text>
                  {w.body ? (
                    <Text style={{ color: colors.sub, fontSize: 13 }} numberOfLines={2}>
                      {w.body}
                    </Text>
                  ) : null}
                </Pressable>
              ))}
            </View>
          );
        })}
      </ScrollView>
    </View>
  );
}
