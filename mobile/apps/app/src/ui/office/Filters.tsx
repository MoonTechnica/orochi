import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { FILTERS, type OfficeFilter } from '@orochi/core';
import { usePrefs } from '../../state/prefs';

const LABEL: Record<OfficeFilter, string> = {
  all: 'All',
  needs: 'Needs you',
  working: 'Working',
};

/** Chips that narrow the rooms to the ones that need you or are working, each with its count. */
export function Filters({
  value,
  counts,
  onChange,
}: {
  value: OfficeFilter;
  counts: Record<OfficeFilter, number>;
  onChange: (f: OfficeFilter) => void;
}) {
  const { colors, t } = usePrefs();
  return (
    <View style={{ flexDirection: 'row', gap: 8 }}>
      {FILTERS.map((f) => {
        const on = f === value;
        return (
          <Pressable
            key={f}
            testID={`filter-${f}`}
            accessibilityRole="button"
            accessibilityState={{ selected: on }}
            accessibilityLabel={`${t(LABEL[f])}, ${t(counts[f] === 1 ? '{n} room' : '{n} rooms').replace('{n}', String(counts[f]))}`}
            onPress={() => onChange(f)}
            style={{
              minHeight: 40,
              paddingHorizontal: 14,
              borderRadius: 20,
              flexDirection: 'row',
              alignItems: 'center',
              gap: 6,
              backgroundColor: on ? colors.text : colors.card,
              borderWidth: 1,
              borderColor: on ? colors.text : colors.line,
            }}
          >
            <Text style={{ color: on ? colors.bg : colors.text, fontWeight: '700', fontSize: 13 }}>
              {t(LABEL[f])}
            </Text>
            <Text style={{ color: on ? colors.bg : colors.sub, fontWeight: '600', fontSize: 13 }}>
              {counts[f]}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}
