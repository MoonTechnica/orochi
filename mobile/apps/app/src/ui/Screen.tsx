import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';
import { Icon } from './Icon';

export const HEADER = 52;

/** A header of our own: safe-area aware, with a back button sized for a thumb. */
export function Header({
  title,
  subtitle,
  right,
  back = true,
}: {
  title: string;
  subtitle?: string;
  right?: React.ReactNode;
  back?: boolean;
}) {
  const { colors, t } = usePrefs();
  const router = useRouter();
  const insets = useSafeAreaInsets();
  return (
    <View
      style={{
        paddingTop: insets.top,
        backgroundColor: colors.card,
        borderBottomWidth: 1,
        borderBottomColor: colors.line,
      }}
    >
      <View
        style={{ height: HEADER, flexDirection: 'row', alignItems: 'center', paddingHorizontal: 4 }}
      >
        {back ? (
          <Pressable
            onPress={() => (router.canGoBack() ? router.back() : router.replace('/'))}
            accessibilityRole="button"
            accessibilityLabel={t('Back')}
            style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}
          >
            <Icon name="chevron-back" size={26} color={colors.text} />
          </Pressable>
        ) : (
          <View style={{ width: 12 }} />
        )}
        <View style={{ flex: 1 }}>
          <Text numberOfLines={1} style={{ color: colors.text, fontSize: 18, fontWeight: '700' }}>
            {title}
          </Text>
          {subtitle ? (
            <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
              {subtitle}
            </Text>
          ) : null}
        </View>
        {right}
      </View>
    </View>
  );
}

export function HeaderButton({
  icon,
  label,
  onPress,
  badge,
}: {
  icon: React.ComponentProps<typeof Icon>['name'];
  label: string;
  onPress: () => void;
  badge?: number;
}) {
  const { colors } = usePrefs();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={{ width: 44, height: 44, alignItems: 'center', justifyContent: 'center' }}
    >
      <Icon name={icon} size={24} color={colors.text} />
      {badge ? (
        <View
          style={{
            position: 'absolute',
            top: 4,
            right: 2,
            minWidth: 16,
            height: 16,
            borderRadius: 8,
            backgroundColor: colors.attention,
            alignItems: 'center',
            justifyContent: 'center',
            paddingHorizontal: 3,
          }}
        >
          <Text style={{ color: colors.onPrimary, fontSize: 10, fontWeight: '700' }}>{badge}</Text>
        </View>
      ) : null}
    </Pressable>
  );
}

export function Empty({ text }: { text: string }) {
  const { colors } = usePrefs();
  return (
    <View style={{ padding: 32, alignItems: 'center' }}>
      <Text style={{ color: colors.sub }}>{text}</Text>
    </View>
  );
}
