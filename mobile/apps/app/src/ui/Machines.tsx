import React, { useCallback, useState } from 'react';
import { Alert, Platform, Pressable, ScrollView, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { displayName, titleIn, type Machine } from '@orochi/core';
import { useAttention, type MachineState } from '../state/attention';
import { usePrefs } from '../state/prefs';
import { useSession } from '../state/session';
import { Drawer } from './Drawer';
import { Icon, type IconName } from './Icon';
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

const CONNECTION_ICONS: Record<MachineState, IconName> = {
  online: 'checkmark-circle-outline',
  connecting: 'time-outline',
  offline: 'cloud-offline-outline',
  repair: 'key-outline',
};

export function ConnectionBadge({ state }: { state: MachineState }) {
  const { colors } = usePrefs();
  const label = useStateLabel();
  const ink = useStateInk();
  const color =
    state === 'online'
      ? colors.connectionOnline
      : state === 'connecting'
        ? colors.connectionPending
        : ink(state);
  return (
    <View
      testID={`connection-${state}`}
      style={{
        alignSelf: 'flex-start',
        flexDirection: 'row',
        alignItems: 'center',
        gap: 5,
        borderRadius: 8,
        paddingHorizontal: 7,
        paddingVertical: 4,
        backgroundColor: colors.raised,
        flexShrink: 0,
      }}
    >
      <View aria-hidden>
        <Icon name={CONNECTION_ICONS[state]} size={14} color={color} />
      </View>
      <Text style={{ color, fontSize: 12, fontWeight: '700' }}>{label(state, true)}</Text>
    </View>
  );
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
 * is in the workspace drawer and Settings → Desktops.
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
              <View style={{ flex: 1 }}>
                <Text
                  numberOfLines={1}
                  style={{
                    color: colors.text,
                    fontSize: 16,
                    fontWeight: selected ? '700' : '500',
                  }}
                >
                  {displayName(m)}
                  {selected ? '  ✓' : ''}
                </Text>
                <View
                  style={{
                    flexDirection: 'row',
                    flexWrap: 'wrap',
                    gap: 6,
                    alignItems: 'center',
                    marginTop: 4,
                  }}
                >
                  <ConnectionBadge state={states[m.key] ?? 'connecting'} />
                  <Text
                    numberOfLines={1}
                    style={{ color: colors.sub, fontSize: 12, flexShrink: 1 }}
                  >
                    {m.host}:{m.port}
                  </Text>
                </View>
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

/** Shows the active workspace and opens the workspace picker. */
export function MachineSwitcher() {
  const { colors, t } = usePrefs();
  const { active, titleOf } = useSession();
  const { states } = useAttention();
  const [open, setOpen] = useState(false);
  const close = useCallback(() => setOpen(false), []);
  const name = active ? displayName(active) : 'Orochi';
  const address = active && titleOf(active) !== name ? `${active.host}:${active.port}` : null;
  return (
    <>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${t('Switch workspace')}: ${active ? titleOf(active) : ''}`}
        accessibilityState={{ expanded: open }}
        onPress={() => setOpen(true)}
        style={{
          flex: 1,
          flexShrink: 1,
          flexDirection: 'row',
          alignItems: 'center',
          gap: 8,
          minHeight: 44,
        }}
      >
        <Dot state={active ? (states[active.key] ?? 'connecting') : 'offline'} />
        <View style={{ flexShrink: 1 }}>
          <Text numberOfLines={1} style={{ color: colors.text, fontSize: 17, fontWeight: '800' }}>
            {name}
          </Text>
          {address ? (
            <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
              {address}
            </Text>
          ) : null}
        </View>
        <View aria-hidden>
          <Icon name="chevron-forward" size={16} color={colors.sub} />
        </View>
      </Pressable>
      <Drawer visible={open} onClose={close} side="left" title={t('Workspaces')}>
        <View style={{ paddingHorizontal: 16 }}>
          <MachineList onPicked={close} />
        </View>
      </Drawer>
    </>
  );
}

/** What the tabs show instead of data while the active machine refuses this phone's token. */
export function RepairPanel() {
  const { colors, t } = usePrefs();
  const { active, machines, remove, titleOf } = useSession();
  const router = useRouter();

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
    <View style={{ flex: 1, backgroundColor: colors.bg }}>
      <ScrollView
        testID="repair-panel"
        style={{ flex: 1, backgroundColor: colors.bg }}
        contentContainerStyle={{ padding: 20, paddingTop: 16, gap: 14 }}
      >
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
          {t('Switch desktops from the name above.')}
        </Text>
      </ScrollView>
    </View>
  );
}
