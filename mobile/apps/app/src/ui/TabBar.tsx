import React, { createContext, useContext, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';
import { useSession } from '../state/session';
import { Icon, type IconName } from './Icon';
import { NewThreadSheet } from './NewThread';
import { radius } from './theme';

interface BarProps {
  state: { index: number; routes: { key: string; name: string }[] };
  descriptors: Record<string, { options: any }>;
  emitter: { emit: (e: any) => any };
  navigateToTab: (routeKey: string) => void;
}

const ICONS: Record<string, IconName> = {
  index: 'git-network-outline',
  assistant: 'sparkles-outline',
  conversations: 'chatbubbles-outline',
  activity: 'pulse-outline',
  settings: 'settings-outline',
};

/** The measured height of the bar, for what has to sit above it (the bar is not drawn over the screens). */
export const TabBarHeight = createContext<{ height: number; set: (h: number) => void }>({
  height: 0,
  set: () => {},
});
export const useTabBarHeight = () => useContext(TabBarHeight).height;

/** A floating pill: four places and, in the middle, the one action of the app. */
export function TabBar({ state, descriptors, emitter, navigateToTab }: BarProps) {
  const { colors, t } = usePrefs();
  const { machines } = useSession();
  const insets = useSafeAreaInsets();
  const { set } = useContext(TabBarHeight);
  const [adding, setAdding] = useState(false);
  const routes = state.routes.filter((r) => descriptors[r.key].options.href !== null);
  const half = Math.ceil(routes.length / 2);
  const left = routes.slice(0, half);
  const right = routes.slice(half);

  const item = (r: (typeof routes)[number], i: number) => {
    const o = descriptors[r.key].options;
    const focused = state.routes[state.index]?.key === r.key;
    const badge = o.tabBarBadge as number | undefined;
    const title = String(o.title ?? r.name);
    return (
      <Pressable
        key={r.key}
        testID={`tab-${r.name}`}
        accessibilityRole="tab"
        accessibilityState={{ selected: focused }}
        accessibilityLabel={`${title}, ${t('tab')}, ${i + 1}/${routes.length}`}
        onPress={() => {
          const e = emitter.emit({ type: 'tabPress', target: r.key, canPreventDefault: true });
          if (!focused && !e?.defaultPrevented) navigateToTab(r.key);
        }}
        style={{
          height: 44,
          minWidth: 44,
          paddingHorizontal: focused ? 14 : 10,
          borderRadius: radius.pill,
          flexDirection: 'row',
          alignItems: 'center',
          gap: 6,
          backgroundColor: focused ? colors.mine : 'transparent',
        }}
      >
        <View>
          <Icon
            name={ICONS[r.name] ?? 'ellipse-outline'}
            size={22}
            color={focused ? colors.text : colors.sub}
          />
          {badge ? (
            <View
              style={{
                position: 'absolute',
                top: -6,
                right: -9,
                minWidth: 16,
                height: 16,
                borderRadius: 8,
                paddingHorizontal: 4,
                backgroundColor: colors.attention,
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              <Text style={{ color: colors.bg, fontSize: 10, fontWeight: '800' }}>{badge}</Text>
            </View>
          ) : null}
        </View>
        {focused ? (
          <Text style={{ color: colors.text, fontSize: 12, fontWeight: '700' }}>{title}</Text>
        ) : null}
      </Pressable>
    );
  };

  return (
    <View
      testID="tab-bar"
      onLayout={(e) => set(Math.round(e.nativeEvent.layout.height))}
      style={{
        backgroundColor: colors.bg,
        paddingBottom: Math.max(insets.bottom, 10),
        paddingTop: 6,
        paddingHorizontal: 14,
      }}
    >
      <View
        style={{
          height: 64,
          borderRadius: 14,
          backgroundColor: colors.bg,
          borderWidth: 1,
          borderColor: colors.line,
          flexDirection: 'row',
          alignItems: 'center',
          justifyContent: 'space-around',
          paddingHorizontal: 6,
        }}
      >
        {left.map((r, i) => item(r, i))}
        {machines.length > 0 ? (
          <Pressable
            testID="new-thread"
            accessibilityRole="button"
            accessibilityLabel={t('New conversation')}
            onPress={() => setAdding(true)}
            style={{
              width: 52,
              height: 52,
              borderRadius: 12,
              backgroundColor: colors.primary,
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <Icon name="add" size={30} color={colors.onPrimary} />
          </Pressable>
        ) : null}
        {right.map((r, i) => item(r, i + half))}
      </View>
      <NewThreadSheet visible={adding} onClose={() => setAdding(false)} />
    </View>
  );
}
