import React, { memo, useMemo } from 'react';
import { Pressable, Text, View } from 'react-native';
import { faceRow, roomAsks, STATE_LABELS, STATUS_TONE, type Room } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Icon, type IconName } from '../Icon';
import { Avatar } from './Avatar';

const MARK: Record<string, IconName> = {
  needs_you: 'chatbubble-ellipses',
  working: 'sync',
  queued: 'time-outline',
  held: 'time-outline',
  interrupted: 'pause-circle-outline',
  background: 'pulse',
  failed: 'alert-circle',
  unread: 'ellipse',
  idle: 'ellipse-outline',
};

/** A room as one line: its state, title, a few faces and how many more, and a marker if it asks. */
function CompactRowView({
  room,
  icons,
  onOpen,
}: {
  room: Room;
  icons: Record<string, string>;
  onOpen: (room: Room) => void;
}) {
  const { colors, t } = usePrefs();
  const { faces, more } = useMemo(() => faceRow(room.people), [room.people]);
  const asks = roomAsks(room);
  const status = room.done ? t('done') : t(STATE_LABELS[room.thread.status] ?? 'Idle');
  const tone = STATUS_TONE[room.thread.status] ?? 'muted';
  const count = room.people.length;
  const label = [
    room.title || t('New conversation'),
    status,
    `${count} ${t(count === 1 ? 'person' : 'people')}`,
    asks ? t('Needs you') : '',
  ]
    .filter(Boolean)
    .join(', ');
  return (
    <Pressable
      testID={`room-${room.id}`}
      accessibilityRole="button"
      accessibilityState={{ expanded: false }}
      accessibilityLabel={`${label}. ${t('Show this room')}`}
      onPress={() => onOpen(room)}
      style={{
        minHeight: 52,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
        paddingHorizontal: 12,
        paddingVertical: 8,
        borderRadius: 12,
        borderWidth: 1,
        borderColor: colors.line,
        backgroundColor: colors.card,
        opacity: room.closing ? 0.5 : 1,
      }}
    >
      <Icon
        name={MARK[room.thread.status] ?? 'ellipse-outline'}
        size={18}
        color={tone === 'live' ? colors.live : tone === 'ask' ? colors.text : colors.muted}
      />
      <Text
        numberOfLines={1}
        style={{ flex: 1, color: colors.text, fontSize: 14, fontWeight: '700' }}
      >
        {room.title || t('New conversation')}
      </Text>
      {asks ? (
        <View
          style={{
            backgroundColor: colors.primary,
            borderRadius: 10,
            paddingHorizontal: 7,
            paddingVertical: 2,
          }}
        >
          <Text style={{ color: colors.onPrimary, fontSize: 10, fontWeight: '800' }}>
            {t('Needs you')}
          </Text>
        </View>
      ) : null}
      <View style={{ flexDirection: 'row', alignItems: 'center' }}>
        {faces.map((p, i) => (
          <View key={`${p.key}:${i}`} style={{ marginLeft: i ? -9 : 0, zIndex: 10 - i }}>
            <Avatar person={p} size={26} icon={icons[p.provider]} still />
          </View>
        ))}
        {more ? (
          <Text style={{ color: colors.sub, fontSize: 12, fontWeight: '700', marginLeft: 5 }}>
            +{more}
          </Text>
        ) : null}
      </View>
      <Icon name="chevron-down" size={18} color={colors.muted} />
    </Pressable>
  );
}

export const CompactRow = memo(CompactRowView);
