import React, { useState } from 'react';
import { Modal, Pressable, ScrollView, Text, View } from 'react-native';
import { RemoteClient, api, baseUrl } from '@orochi/protocol';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';
import { useAttention } from '../state/attention';
import { useOpenThread } from '../state/open';
import { useSession } from '../state/session';

/** Pick a desktop and a project, then `new_thread` on that desktop and open it. */
export function NewThreadSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const { colors, t } = usePrefs();
  const { machines } = useSession();
  const { snapshots } = useAttention();
  const open = useOpenThread();
  const insets = useSafeAreaInsets();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const online = snapshots.filter((s) => s.state === 'online');

  const start = async (machineKey: string, root: string) => {
    const m = machines.find((x) => x.key === machineKey);
    if (!m) return;
    setBusy(`${machineKey}:${root}`);
    setError(null);
    try {
      const id = await api(
        new RemoteClient({ base: baseUrl(m.host, m.port), token: m.token }),
      ).newThread(root);
      onClose();
      await open(machineKey, id);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={onClose}>
      <View
        accessibilityViewIsModal
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
      >
        <Pressable
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={t('Close menu')}
        />
        <View
          style={{
            backgroundColor: colors.card,
            borderTopLeftRadius: 12,
            borderTopRightRadius: 12,
            paddingBottom: insets.bottom + 8,
            maxHeight: '75%',
          }}
        >
          <Text
            style={{
              color: colors.text,
              fontSize: 18,
              fontWeight: '800',
              padding: 16,
              paddingBottom: 6,
            }}
          >
            {t('New conversation')}
          </Text>
          <ScrollView>
            {online.length === 0 ? (
              <Text style={{ color: colors.sub, padding: 16 }}>
                {t('No desktop is connected.')}
              </Text>
            ) : null}
            {online.map((s) => (
              <View key={s.key}>
                {snapshots.length > 1 ? (
                  <Text
                    style={{
                      color: colors.sub,
                      fontSize: 12,
                      fontWeight: '700',
                      paddingHorizontal: 16,
                      paddingTop: 12,
                    }}
                  >
                    {s.name}
                  </Text>
                ) : null}
                {s.projects
                  .filter((p) => !p.missing)
                  .map((p) => (
                    <Pressable
                      key={p.id}
                      accessibilityRole="button"
                      accessibilityLabel={`${p.name}${snapshots.length > 1 ? ` · ${s.name}` : ''}`}
                      disabled={!!busy}
                      onPress={() => start(s.key, p.root)}
                      style={{
                        minHeight: 52,
                        justifyContent: 'center',
                        paddingHorizontal: 16,
                        borderTopWidth: 1,
                        borderTopColor: colors.line,
                        opacity: busy ? 0.6 : 1,
                      }}
                    >
                      <Text style={{ color: colors.text, fontSize: 16 }}>{p.name}</Text>
                      <Text style={{ color: colors.sub, fontSize: 12 }} numberOfLines={1}>
                        {p.root}
                      </Text>
                    </Pressable>
                  ))}
              </View>
            ))}
            {error ? <Text style={{ color: colors.dangerInk, padding: 16 }}>{error}</Text> : null}
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}
