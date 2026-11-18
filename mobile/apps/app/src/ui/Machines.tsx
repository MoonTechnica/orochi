import React, { useState } from 'react';
import {
  Alert,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  Text,
  View,
} from 'react-native';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { displayName, titleIn, type Machine } from '@orochi/core';
import { useAttention, type MachineState } from '../state/attention';
import { usePrefs } from '../state/prefs';
import { useSession } from '../state/session';
import { Icon } from './Icon';
import { Sheet, TextPrompt } from './Sheet';

export function useStateColor() {
  const { colors } = usePrefs();
  return (s: MachineState) =>
    s === 'online'
      ? colors.live
      : s === 'connecting'
        ? colors.muted
        : s === 'repair'
          ? colors.attention // not an error: the desktop is fine, this phone must pair again
          : colors.danger; // cannot be reached
}

/** Text-safe colour of a state (the plain one is for dots and rings). */
export function useStateInk() {
  const { colors } = usePrefs();
  return (s: MachineState) =>
    s === 'online'
      ? colors.text
      : s === 'connecting'
        ? colors.sub
        : s === 'repair'
          ? colors.text
          : colors.dangerInk;
}

export function useStateLabel() {
  const { t } = usePrefs();
  return (s: MachineState, short = false) =>
    s === 'online'
      ? t('Online')
      : s === 'connecting'
        ? t('Connecting')
        : s === 'repair'
          ? short
            ? t('Re-pair')
            : t('Needs pairing again')
          : t('Offline');
}

export function Dot({ state }: { state: MachineState }) {
  const color = useStateColor();
  const label = useStateLabel();
  return (
    <View
      accessibilityLabel={label(state)}
      style={{ width: 10, height: 10, borderRadius: 5, backgroundColor: color(state) }}
    />
  );
}

/**
 * Every paired desktop with its state; tapping one makes it the active machine. The same list
 * is the switcher's sheet and Settings → Desktops.
 */
export function MachineList({ onPicked }: { onPicked?: () => void }) {
  const { colors, t } = usePrefs();
  const { machines, active, switchTo, rename, remove, titleOf } = useSession();
  const { states, waiting } = useAttention();
  const label = useStateLabel();
  const router = useRouter();
  const [menu, setMenu] = useState<Machine | null>(null);
  const [renaming, setRenaming] = useState<Machine | null>(null);
  const store = { machines, activeKey: active?.key ?? null };

  const confirmRemove = (m: Machine) => {
    const go = () => remove(m.key).then(() => machines.length === 1 && router.replace('/pair'));
    if (Platform.OS === 'web') {
      if (globalThis.confirm?.(`${t('Remove this desktop?')} ${titleOf(m)}`)) go();
      return;
    }
    Alert.alert(t('Remove this desktop?'), titleOf(m), [
      { text: t('Cancel'), style: 'cancel' },
      { text: t('Remove'), style: 'destructive', onPress: go },
    ]);
  };

  return (
    <View>
      <TextPrompt
        inline
        visible={!!renaming}
        title={t('Rename')}
        initial={renaming ? displayName(renaming) : ''}
        placeholder={renaming?.name}
        confirm={t('Save')}
        onClose={() => setRenaming(null)}
        onSubmit={(text) => renaming && rename(renaming.key, text === renaming.name ? '' : text)}
      />
      {machines.map((m) => {
        const count = waiting.filter((w) => w.machineKey === m.key).length;
        const selected = m.key === active?.key;
        return (
          <View
            key={m.key}
            style={{
              flexDirection: 'row',
              alignItems: 'center',
              minHeight: 60,
              borderTopWidth: 1,
              borderTopColor: colors.line,
            }}
          >
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${titleIn(store, m)}, ${label(states[m.key] ?? 'connecting')}`}
              accessibilityState={{ selected }}
              onPress={async () => {
                await switchTo(m.key);
                if (!selected) router.replace('/conversations');
                onPicked?.();
              }}
              style={{
                flex: 1,
                flexDirection: 'row',
                alignItems: 'center',
                gap: 12,
                paddingVertical: 10,
              }}
            >
              <Dot state={states[m.key] ?? 'connecting'} />
              <View style={{ flex: 1 }}>
                <Text
                  numberOfLines={1}
                  style={{
                    color: selected ? colors.text : colors.text,
                    fontSize: 16,
                    fontWeight: selected ? '700' : '500',
                  }}
                >
                  {titleIn(store, m)}
                  {selected ? '  ✓' : ''}
                </Text>
                <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
                  {label(states[m.key] ?? 'connecting')} · {m.host}:{m.port}
                </Text>
              </View>
              {count > 0 ? (
                <Text style={{ color: colors.text, fontWeight: '700' }}>
                  {count} {t('waiting')}
                </Text>
              ) : null}
            </Pressable>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`${t('More')}: ${titleOf(m)}`}
              onPress={() => setMenu(m)}
              style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}
            >
              <Icon name="ellipsis-horizontal" size={20} color={colors.sub} />
            </Pressable>
          </View>
        );
      })}
      <Pressable
        accessibilityRole="button"
        onPress={() => {
          onPicked?.();
          router.push({ pathname: '/pair', params: { add: '1' } });
        }}
        style={{
          minHeight: 52,
          justifyContent: 'center',
          borderTopWidth: 1,
          borderTopColor: colors.line,
        }}
      >
        <Text style={{ color: colors.text, fontSize: 16, fontWeight: '600' }}>
          + {t('Add a desktop')}
        </Text>
      </Pressable>
      <Sheet
        visible={!!menu}
        title={menu ? titleOf(menu) : ''}
        onClose={() => setMenu(null)}
        actions={
          menu
            ? [
                { label: t('Rename'), onPress: () => setRenaming(menu) },
                ...(menu.needsRepair
                  ? [
                      {
                        label: t('Pair again'),
                        onPress: () => {
                          onPicked?.();
                          router.push({ pathname: '/pair', params: { repair: menu.key } });
                        },
                      },
                    ]
                  : []),
                { label: t('Remove'), danger: true, onPress: () => confirmRemove(menu) },
              ]
            : []
        }
      />
    </View>
  );
}

/** The conversation list's header: the active machine's name with its state; opens the list. */
export function MachineSwitcher() {
  const { colors, t } = usePrefs();
  const { machines, active, titleOf } = useSession();
  const { states, waiting } = useAttention();
  const insets = useSafeAreaInsets();
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const store = { machines, activeKey: active?.key ?? null };
  const elsewhere = waiting.filter((w) => w.machineKey !== active?.key).length;
  return (
    <>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${t('Switch desktop')}: ${active ? titleOf(active) : ''}`}
        onPress={() => setOpen(true)}
        style={{ flex: 1, flexDirection: 'row', alignItems: 'center', gap: 8, minHeight: 44 }}
      >
        <Text
          numberOfLines={1}
          style={{
            color: colors.text,
            fontSize: 28,
            fontWeight: '800',
            letterSpacing: -0.6,
            flexShrink: 1,
          }}
        >
          {active ? titleIn(store, active) : 'Orochi'}
        </Text>
        <Dot state={active ? (states[active.key] ?? 'connecting') : 'offline'} />
        <Icon name="chevron-down" size={18} color={colors.sub} />
        {elsewhere > 0 ? (
          <View
            style={{
              backgroundColor: colors.attention,
              borderRadius: 10,
              paddingHorizontal: 7,
              minWidth: 20,
              alignItems: 'center',
            }}
          >
            <Text style={{ color: colors.onPrimary, fontSize: 12, fontWeight: '700' }}>
              {elsewhere}
            </Text>
          </View>
        ) : null}
      </Pressable>
      <Modal visible={open} transparent animationType="fade" onRequestClose={() => setOpen(false)}>
        <KeyboardAvoidingView
          behavior={Platform.OS === 'ios' ? 'padding' : undefined}
          accessibilityViewIsModal
          style={{ flex: 1, backgroundColor: '#0006', justifyContent: 'flex-end' }}
        >
          <Pressable
            style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
            onPress={() => setOpen(false)}
            accessibilityRole="button"
            accessibilityLabel={t('Close menu')}
          />
          <View
            style={{
              backgroundColor: colors.card,
              borderTopLeftRadius: 12,
              borderTopRightRadius: 12,
              paddingHorizontal: 20,
              paddingTop: 14,
              paddingBottom: insets.bottom + 12,
              maxHeight: '80%',
            }}
          >
            <Text style={{ color: colors.sub, fontSize: 13, fontWeight: '700', paddingBottom: 6 }}>
              {t('Desktops')}
            </Text>
            <ScrollView keyboardShouldPersistTaps="handled">
              <MachineList onPicked={() => setOpen(false)} />
              {machines.length > 1 ? (
                <Pressable
                  accessibilityRole="button"
                  onPress={() => {
                    setOpen(false);
                    router.push('/attention');
                  }}
                  style={{
                    minHeight: 52,
                    justifyContent: 'center',
                    borderTopWidth: 1,
                    borderTopColor: colors.line,
                  }}
                >
                  <Text style={{ color: colors.text, fontSize: 16, fontWeight: '600' }}>
                    {t('Needs you on all desktops')} ({waiting.length})
                  </Text>
                </Pressable>
              ) : null}
            </ScrollView>
          </View>
        </KeyboardAvoidingView>
      </Modal>
    </>
  );
}

/** What the tabs show instead of data while the active machine refuses this phone's token. */
export function RepairPanel() {
  const { colors, t } = usePrefs();
  const { active, machines, remove, titleOf } = useSession();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const confirmRemove = () => {
    if (!active) return;
    const go = () =>
      remove(active.key).then(() => machines.length === 1 && router.replace('/pair'));
    if (Platform.OS === 'web') {
      if (globalThis.confirm?.(`${t('Remove this desktop?')} ${titleOf(active)}`)) go();
      return;
    }
    Alert.alert(t('Remove this desktop?'), titleOf(active), [
      { text: t('Cancel'), style: 'cancel' },
      { text: t('Remove'), style: 'destructive', onPress: go },
    ]);
  };
  return (
    <ScrollView
      testID="repair-panel"
      style={{ flex: 1, backgroundColor: colors.bg }}
      contentContainerStyle={{ padding: 20, paddingTop: insets.top + 16, gap: 14 }}
    >
      <View style={{ flexDirection: 'row' }}>
        <MachineSwitcher />
      </View>
      <Text style={{ color: colors.text, fontSize: 15 }}>
        {t('This phone is no longer paired with this desktop (revoked there). Pair again.')}
      </Text>
      <Pressable
        accessibilityRole="button"
        onPress={() => router.push({ pathname: '/pair', params: { repair: active?.key } })}
        style={{
          minHeight: 50,
          borderRadius: 12,
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: colors.primary,
        }}
      >
        <Text style={{ color: colors.onPrimary, fontSize: 16, fontWeight: '700' }}>
          {t('Pair again')}
        </Text>
      </Pressable>
      <Pressable
        testID="repair-remove"
        accessibilityRole="button"
        onPress={confirmRemove}
        style={{
          minHeight: 50,
          borderRadius: 12,
          alignItems: 'center',
          justifyContent: 'center',
          borderWidth: 1,
          borderColor: colors.line,
        }}
      >
        <Text style={{ color: colors.dangerInk, fontSize: 16, fontWeight: '600' }}>
          {t('Remove this desktop')}
        </Text>
      </Pressable>
      <Text style={{ color: colors.sub, fontSize: 13 }}>
        {t('Your other desktops are not affected. Switch from the name above, or use the tabs.')}
      </Text>
    </ScrollView>
  );
}
