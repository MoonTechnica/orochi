import React from 'react';
import { Pressable, Text, View } from 'react-native';
import type { OfficeFilter, OfficeStats } from '@orochi/core';
import { usePrefs } from '../../state/prefs';

const CHIPS: [keyof OfficeStats, string, OfficeFilter | null][] = [
  ['working', 'at work', 'working'],
  ['assistants', 'assistants', null],
  ['asking', 'need you', 'needs'],
  ['messages', 'messages', null],
  ['rooms', 'rooms open', 'all'],
];

/**
 * The desktop's counters across the top: who works, who waits for you, what was said. Three of them
 * are also the filters: tap "need you" to see only those rooms, tap it again for all of them.
 */
export function Strip({
  stats,
  filter,
  onFilter,
}: {
  stats: OfficeStats;
  filter: OfficeFilter;
  onFilter: (f: OfficeFilter) => void;
}) {
  const { colors, t } = usePrefs();
  return (
    <View
      style={{
        flexDirection: 'row',
        backgroundColor: colors.card,
        borderRadius: 8,
        borderWidth: 1,
        borderColor: colors.line,
        paddingVertical: 4,
      }}
    >
      {CHIPS.map(([key, label, target]) => {
        const on = target !== null && target !== 'all' && filter === target;
        const body = (
          <>
            <Text style={{ fontSize: 22, fontWeight: '800', color: colors.text }}>
              {stats[key]}
            </Text>
            <Text
              numberOfLines={2}
              style={{ color: colors.sub, fontSize: 10, textAlign: 'center' }}
            >
              {t(label)}
            </Text>
          </>
        );
        const base = { flex: 1, alignItems: 'center' as const, gap: 1, paddingVertical: 6 };
        return target ? (
          <Pressable
            key={key}
            testID={`stat-${key}`}
            accessibilityRole="button"
            accessibilityState={{ selected: on }}
            accessibilityLabel={`${stats[key]} ${t(label)}`}
            onPress={() => onFilter(on || target === 'all' ? 'all' : target)}
            style={[
              base,
              {
                borderRadius: 6,
                borderBottomWidth: 2,
                borderBottomColor: on ? colors.text : 'transparent',
              },
            ]}
          >
            {body}
          </Pressable>
        ) : (
          <View
            key={key}
            testID={`stat-${key}`}
            accessible
            accessibilityLabel={`${stats[key]} ${t(label)}`}
            style={base}
          >
            {body}
          </View>
        );
      })}
    </View>
  );
}
