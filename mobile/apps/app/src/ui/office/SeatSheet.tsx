import React, { useState } from 'react';
import { Modal, Pressable, ScrollView, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { since, type Person } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Avatar } from './Avatar';
import { Clock, nameOf, stateLabel } from './Seat';

/** What one person is doing, in full, with the two things that can be done about it. */
export function SeatSheet({
  person,
  icon,
  boss,
  canStop,
  onClose,
  onOpen,
  onStop,
}: {
  person: Person | null;
  icon?: string;
  boss: string | null;
  canStop: boolean;
  onClose: () => void;
  onOpen: (p: Person) => void;
  onStop: (p: Person) => Promise<void>;
}) {
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const p = person;
  const close = () => {
    setFailed(false);
    onClose();
  };
  const button = (label: string, onPress: () => void, kind: 'primary' | 'danger' | 'plain') => (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      onPress={onPress}
      style={{
        minHeight: 50,
        borderRadius: 10,
        alignItems: 'center',
        justifyContent: 'center',
        backgroundColor: kind === 'primary' ? colors.primary : 'transparent',
        borderWidth: kind === 'primary' ? 0 : 1,
        borderColor: kind === 'danger' ? colors.danger : colors.line,
      }}
    >
      <Text
        style={{
          fontSize: 16,
          fontWeight: '700',
          color:
            kind === 'primary'
              ? colors.onPrimary
              : kind === 'danger'
                ? colors.dangerInk
                : colors.text,
        }}
      >
        {label}
      </Text>
    </Pressable>
  );
  return (
    <Modal visible={!!p} transparent animationType="fade" onRequestClose={close}>
      <View
        accessibilityViewIsModal
        style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
      >
        <Pressable
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={close}
          accessibilityRole="button"
          accessibilityLabel={t('Close')}
        />
        {p ? (
          <View
            testID="seat-sheet"
            style={{
              backgroundColor: colors.card,
              borderTopLeftRadius: 12,
              borderTopRightRadius: 12,
              padding: 18,
              paddingBottom: insets.bottom + 14,
              gap: 14,
              maxHeight: '85%',
            }}
          >
            <ScrollView contentContainerStyle={{ gap: 14 }}>
              <View style={{ flexDirection: 'row', gap: 16, alignItems: 'center' }}>
                <Avatar person={p} size={72} icon={icon} still />
                <View style={{ flex: 1, gap: 2 }}>
                  <Text
                    style={{ color: colors.text, fontSize: 20, fontWeight: '800' }}
                    numberOfLines={1}
                  >
                    {nameOf(p, t)}
                  </Text>
                  <Text style={{ color: colors.sub, fontSize: 13 }} numberOfLines={1}>
                    {[p.agent, p.model].filter(Boolean).join(' · ')}
                  </Text>
                  <View style={{ flexDirection: 'row', gap: 8 }}>
                    <Text style={{ color: colors.text, fontSize: 13, fontWeight: '600' }}>
                      {stateLabel(p, t)}
                    </Text>
                    {p.startedAt ? (
                      <Clock at={p.startedAt} style={{ color: colors.sub, fontSize: 13 }} />
                    ) : null}
                  </View>
                  {boss ? (
                    <Text style={{ color: colors.muted, fontSize: 12 }}>
                      {t('Reports to {name}').replace('{name}', boss)}
                    </Text>
                  ) : null}
                </View>
              </View>
              {p.state === 'asking' ? (
                <Text style={{ color: colors.text, fontSize: 15, fontWeight: '700' }}>
                  {t('Waiting for you')}
                  {p.doing ? `: ${p.doing}` : ''}
                </Text>
              ) : p.doing ? (
                <Text style={{ color: colors.text, fontSize: 15, lineHeight: 21 }}>{p.doing}</Text>
              ) : null}
              {p.mail ? (
                <View
                  style={{ backgroundColor: colors.raised, borderRadius: 10, padding: 12, gap: 2 }}
                >
                  <Text style={{ color: colors.sub, fontSize: 12 }}>
                    {t('Said to this seat')} · {p.mail.from_role || p.mail.from} ·{' '}
                    {since(p.mail.at)}
                  </Text>
                  <Text style={{ color: colors.text, fontSize: 14 }}>{p.mail.text}</Text>
                </View>
              ) : null}
              {p.tasks.length ? (
                <View style={{ gap: 6 }}>
                  <Text style={{ color: colors.sub, fontSize: 12, fontWeight: '700' }}>
                    {t('Sub-agents').toUpperCase()}
                  </Text>
                  {p.tasks.map((k) => (
                    <View key={k.id} style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
                      <View
                        style={{
                          width: 9,
                          height: 9,
                          borderRadius: 5,
                          borderWidth: 1.5,
                          borderColor: colors.text,
                        }}
                      />
                      <Text style={{ color: colors.text, fontSize: 13, flex: 1 }} numberOfLines={2}>
                        {k.title || t('sub-agent')}
                        {k.doing
                          ? ` · ${k.doing}`
                          : k.calls
                            ? ` · ${k.calls} ${t(k.calls === 1 ? 'call' : 'calls')}`
                            : ''}
                      </Text>
                      <Clock at={k.startedAt} style={{ color: colors.muted, fontSize: 12 }} />
                    </View>
                  ))}
                </View>
              ) : null}
              {failed ? (
                <Text style={{ color: colors.dangerInk, fontSize: 13 }}>
                  {t('Could not stop it.')}
                </Text>
              ) : null}
              {button(
                p.state === 'asking' ? t('Open the question') : t('Open thread'),
                () => {
                  close();
                  onOpen(p);
                },
                'primary',
              )}
              {canStop && p.seatId && (p.state === 'working' || p.state === 'asking')
                ? button(
                    busy ? t('Stopping…') : t('Stop this seat'),
                    async () => {
                      if (busy) return;
                      setBusy(true);
                      try {
                        await onStop(p);
                        close();
                      } catch {
                        setFailed(true);
                      } finally {
                        setBusy(false);
                      }
                    },
                    'danger',
                  )
                : null}
              {button(t('Close'), close, 'plain')}
            </ScrollView>
          </View>
        ) : null}
      </View>
    </Modal>
  );
}
