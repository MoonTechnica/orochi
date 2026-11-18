import React from 'react';
import { Pressable, Text, View } from 'react-native';
import { PERSON_STATES, type PersonState, type Room, type Selection } from '@orochi/core';
import { usePrefs } from '../../state/prefs';

/** What the people behind a "+N" are doing, in words, for the label of the tile. */
export function summaryText(summary: Selection['summary'], t: (k: string) => string) {
  return PERSON_STATES.filter((s) => summary[s] > 0)
    .map((s) => `${summary[s]} ${t(s)}`)
    .join(', ');
}

/** A small mark per state: filled for the ones at work or asking, hollow for the rest. */
function Dot({ state }: { state: PersonState }) {
  const { colors } = usePrefs();
  const live = state === 'working' || state === 'asking';
  return (
    <View
      style={{
        width: 9,
        height: 9,
        borderRadius: 5,
        borderWidth: state === 'asking' ? 3 : 1.5,
        borderColor: state === 'asking' ? colors.attention : live ? colors.text : colors.muted,
        backgroundColor: live ? colors.text : 'transparent',
        borderStyle: live ? 'solid' : 'dashed',
      }}
    />
  );
}

/** The seat after the last drawn one: how many more are in the room, and what they are doing. */
export function MoreTile({
  room,
  picked,
  onPress,
}: {
  room: Room;
  picked: Selection;
  onPress: (room: Room) => void;
}) {
  const { colors, t } = usePrefs();
  const parts = PERSON_STATES.filter((s) => picked.summary[s] > 0);
  const label = `${t(
    picked.more === 1 ? '+{n} more person in {room}' : '+{n} more people in {room}',
  )
    .replace('{n}', String(picked.more))
    .replace('{room}', room.title || t('New conversation'))}, ${summaryText(picked.summary, t)}`;
  return (
    <View style={{ width: '33.33%', paddingHorizontal: 4, paddingVertical: 8 }}>
      <Pressable
        testID={`more-${room.id}`}
        accessibilityRole="button"
        accessibilityLabel={label}
        onPress={() => onPress(room)}
        style={{ alignItems: 'center' }}
      >
        <View
          style={{
            width: 56,
            height: 56,
            borderRadius: 28,
            borderWidth: 2,
            borderStyle: 'dashed',
            borderColor: colors.sub,
            backgroundColor: colors.raised,
            alignItems: 'center',
            justifyContent: 'center',
            marginBottom: 6,
          }}
        >
          <Text style={{ color: colors.text, fontSize: 18, fontWeight: '800' }}>
            +{picked.more}
          </Text>
        </View>
        <Text style={{ color: colors.text, fontSize: 12, fontWeight: '700' }}>{t('more')}</Text>
        <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 6, justifyContent: 'center' }}>
          {parts.slice(0, 3).map((s) => (
            <View key={s} style={{ flexDirection: 'row', alignItems: 'center', gap: 3 }}>
              <Dot state={s} />
              <Text style={{ color: colors.sub, fontSize: 10 }}>{picked.summary[s]}</Text>
            </View>
          ))}
        </View>
      </Pressable>
    </View>
  );
}
