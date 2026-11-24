import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { usePrefs } from '../state/prefs';
import { Icon, type IconName } from './Icon';

export function DisclosureRow({
  title,
  summary,
  icon,
  expanded,
  onPress,
  testID,
}: {
  title: string;
  summary?: string;
  icon?: IconName;
  expanded?: boolean;
  onPress: () => void;
  testID?: string;
}) {
  const { colors } = usePrefs();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      aria-expanded={expanded}
      accessibilityLabel={[title, summary].filter(Boolean).join(', ')}
      onPress={onPress}
      style={({ pressed }) => ({
        minHeight: 48,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
        paddingHorizontal: 12,
        paddingVertical: 8,
        backgroundColor: pressed ? colors.raised : 'transparent',
      })}
    >
      {icon ? <Icon name={icon} size={19} color={colors.sub} /> : null}
      <View style={{ flex: 1, gap: 2 }}>
        <Text style={{ color: colors.text, fontSize: 15, fontWeight: '600' }}>{title}</Text>
        {summary ? <Text style={{ color: colors.sub, fontSize: 13 }}>{summary}</Text> : null}
      </View>
      <Icon
        name={expanded === undefined ? 'chevron-forward' : expanded ? 'chevron-up' : 'chevron-down'}
        size={16}
        color={colors.sub}
      />
    </Pressable>
  );
}

type RowProps = React.ComponentProps<typeof DisclosureRow>;

export function Disclosure({
  expanded,
  children,
  ...row
}: Omit<RowProps, 'expanded'> & { expanded: boolean; children: React.ReactNode }) {
  const { colors } = usePrefs();
  return (
    <View style={{ borderTopWidth: 1, borderTopColor: colors.line }}>
      <DisclosureRow {...row} expanded={expanded} />
      {expanded ? children : null}
    </View>
  );
}
