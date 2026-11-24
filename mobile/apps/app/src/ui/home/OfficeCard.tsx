import React, { useMemo } from 'react';
import { Pressable, Text, View, useWindowDimensions } from 'react-native';
import { officeSummary, type MachineSnapshot, type Person } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Icon } from '../Icon';
import { Avatar } from '../office/Avatar';

/** Home's way into the Office: who is at work, who waits for you, and the first few faces. */
export function OfficeCard({
  snapshots,
  onPress,
}: {
  snapshots: MachineSnapshot[];
  onPress: () => void;
}) {
  const { colors, t } = usePrefs();
  const { width, fontScale } = useWindowDimensions();
  const faceCount = fontScale > 1.3 ? 0 : width < 380 ? 2 : 3;
  const { stats, faces } = useMemo(() => {
    const sum = { working: 0, assistants: 0, asking: 0, messages: 0, rooms: 0 };
    const faces: { id: string; person: Person }[] = [];
    for (const m of snapshots) {
      if (m.state !== 'online') continue;
      const one = officeSummary(
        { projects: m.projects, seats: m.seats, tasks: m.tasks, talk: [] },
        Date.now(),
      );
      for (const k of Object.keys(sum) as (keyof typeof sum)[]) sum[k] += one.stats[k];
      faces.push(...one.faces.map((person) => ({ id: `${m.key}:${person.key}`, person })));
    }
    return { stats: sum, faces: faces.slice(0, 5) };
  }, [snapshots]);
  const line = stats.rooms
    ? t('{n} at work · {m} need you')
        .replace('{n}', String(stats.working))
        .replace('{m}', String(stats.asking))
    : t('The office is quiet. Start a thread to put someone to work.');
  return (
    <Pressable
      testID="office-card"
      accessibilityRole="button"
      accessibilityLabel={`${t('Office')}. ${line}`}
      onPress={onPress}
      style={{
        marginHorizontal: 18,
        marginTop: 0,
        marginBottom: 12,
        borderRadius: 12,
        padding: 16,
        backgroundColor: colors.card,
        borderWidth: stats.asking ? 2 : 1,
        borderColor: stats.asking ? colors.attention : colors.line,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 14,
      }}
    >
      <View style={{ flex: 1, gap: 4 }}>
        <Text style={{ color: colors.text, fontSize: 20, fontWeight: '800' }}>{t('Office')}</Text>
        <Text style={{ color: colors.sub, fontSize: 13 }} numberOfLines={2}>
          {line}
        </Text>
        {stats.rooms ? (
          <Text style={{ color: colors.sub, fontSize: 12 }}>
            {t(stats.rooms === 1 ? '{n} room' : '{n} rooms').replace('{n}', String(stats.rooms))} ·{' '}
            {stats.assistants} {t('assistants')}
          </Text>
        ) : null}
      </View>
      {faces.length > 0 && faceCount > 0 ? (
        <View style={{ flexDirection: 'row', paddingRight: 8 }}>
          {faces.slice(0, faceCount).map(({ id, person: p }, i) => (
            <View key={`${id}:${i}`} style={{ marginRight: -10, zIndex: 10 - i }}>
              <Avatar person={p} size={34} still />
            </View>
          ))}
        </View>
      ) : null}
      <Icon name="chevron-forward" size={20} color={colors.muted} />
    </Pressable>
  );
}
