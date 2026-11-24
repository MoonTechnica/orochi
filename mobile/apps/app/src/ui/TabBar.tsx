import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import type { BottomTabBarProps } from 'expo-router/tabs';
import { usePrefs } from '../state/prefs';
import { Icon, type IconName } from './Icon';

const icons: Record<string, IconName> = {
  index: 'home-outline',
  'all-conversations': 'chatbubbles-outline',
  'all-activity': 'pulse-outline',
  'all-assistants': 'sparkles-outline',
  conversations: 'chatbubbles-outline',
  assistant: 'sparkles-outline',
  activity: 'pulse-outline',
};

/** Selection changes ink and the marker, while touch targets stay in place. */
export function TabBar({ state, descriptors, emitter, navigateToTab }: BottomTabBarProps) {
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();

  return (
    <View
      testID="tab-bar"
      style={{
        backgroundColor: colors.card,
        borderTopWidth: 1,
        borderTopColor: colors.line,
        paddingBottom: Math.max(insets.bottom, 4),
        paddingLeft: insets.left,
        paddingRight: insets.right,
      }}
    >
      <View
        accessibilityRole="tablist"
        style={{ flexDirection: 'row', alignSelf: 'center', width: '100%', maxWidth: 480 }}
      >
        {state.routes.map((route) => {
          const options = descriptors[route.key].options;
          const selected = state.routes[state.index]?.key === route.key;
          const badge = options.tabBarBadge;
          const title = options.title ?? route.name;
          const ink = selected ? colors.text : colors.sub;

          return (
            <Pressable
              key={route.key}
              testID={`tab-${route.name}`}
              accessibilityRole="tab"
              aria-selected={selected}
              accessibilityLabel={`${title}, ${t('tab')}${badge ? `, ${t('{n} awaiting your response').replace('{n}', String(badge))}` : ''}`}
              onPress={() => {
                const event = emitter.emit({
                  type: 'tabPress',
                  target: route.key,
                  canPreventDefault: true,
                });
                if (!selected && !event.defaultPrevented) navigateToTab(route.key);
              }}
              style={({ pressed }) => ({
                flex: 1,
                minWidth: 0,
                minHeight: 56,
                paddingHorizontal: 4,
                paddingVertical: 8,
                alignItems: 'center',
                justifyContent: 'center',
                gap: 4,
                backgroundColor: pressed ? colors.raised : 'transparent',
              })}
            >
              <View
                style={{
                  position: 'absolute',
                  top: 0,
                  width: 24,
                  height: 2,
                  backgroundColor: selected ? colors.text : 'transparent',
                }}
              />
              <View>
                <Icon name={icons[route.name] ?? 'ellipse-outline'} size={20} color={ink} />
                {badge ? (
                  <View
                    style={{
                      position: 'absolute',
                      top: 0,
                      right: -5,
                      height: 6,
                      width: 6,
                      borderRadius: 3,
                      backgroundColor: ink,
                    }}
                  />
                ) : null}
              </View>
              <Text style={{ color: ink, fontSize: 12, fontWeight: '600', textAlign: 'center' }}>
                {title}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}
