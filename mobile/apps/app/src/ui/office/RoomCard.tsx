import React, { memo, useEffect, useMemo } from 'react';
import { Pressable, Text, View } from 'react-native';
import Animated, {
  FadeInDown,
  FadeOut,
  cancelAnimation,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withSequence,
  withTiming,
} from 'react-native-reanimated';
import {
  selectPeople,
  since,
  STATE_LABELS,
  STATUS_TONE,
  type Person,
  type Room,
  type RoomLine,
} from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Icon, type IconName } from '../Icon';
import { MoreTile } from './More';
import { useOfficeMotion } from './motion';
import { Seat } from './Seat';

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
  done: 'checkmark-circle',
};

/** A message hops along once when it is news, and rests when it is not. */
function Hop({ fresh }: { fresh: boolean }) {
  const { colors } = usePrefs();
  const calm = !useOfficeMotion();
  const x = useSharedValue(0);
  useEffect(() => {
    if (!fresh || calm) {
      x.value = 0;
      return;
    }
    x.value = withRepeat(
      withSequence(withTiming(7, { duration: 450 }), withTiming(0, { duration: 450 })),
      4,
    );
    return () => cancelAnimation(x);
  }, [fresh, calm, x]);
  const style = useAnimatedStyle(() => ({ transform: [{ translateX: x.value }] }));
  return (
    <Animated.View style={style}>
      <Icon
        name={fresh ? 'mail' : 'mail-outline'}
        size={16}
        color={fresh ? colors.text : colors.muted}
      />
    </Animated.View>
  );
}

function Line({ line }: { line: RoomLine }) {
  const { colors, t } = usePrefs();
  const to = line.to ?? t('everyone');
  const where =
    line.direction === 'out'
      ? `↗ ${line.across}`
      : line.direction === 'in'
        ? `↘ ${line.across}`
        : '';
  return (
    <View
      accessible
      accessibilityLabel={`${line.from} → ${to}${where ? ` (${where})` : ''}: ${line.text}`}
      style={{ flexDirection: 'row', gap: 8, alignItems: 'flex-start' }}
    >
      <View style={{ paddingTop: 1 }}>
        <Hop fresh={line.fresh} />
      </View>
      <View style={{ flex: 1 }}>
        <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 11 }}>
          <Text style={{ fontWeight: '700', color: colors.text }}>{line.from}</Text> → {to}
          {where ? `  ${where}` : ''}
        </Text>
        <Text numberOfLines={2} style={{ color: colors.text, fontSize: 12, lineHeight: 16 }}>
          {line.text}
        </Text>
      </View>
    </View>
  );
}

/**
 * One conversation as a room: its title and state, up to six of its team in a flowing grid (the
 * rest are one "+N" tile that opens the whole list), its recent messages.
 */
function RoomCardView({
  room,
  icons,
  onOpen,
  onPerson,
  onMore,
  onCollapse,
}: {
  room: Room;
  icons: Record<string, string>;
  onOpen: (room: Room) => void;
  onPerson: (p: Person) => void;
  onMore: (room: Room) => void;
  /** Given for a room that was opened from a one-line row: a button to close it again. */
  onCollapse?: (room: Room) => void;
}) {
  const { colors, t } = usePrefs();
  const reduced = useReducedMotion();
  const picked = useMemo(() => selectPeople(room.people), [room.people]);
  const tone = room.done ? 'ok' : (STATUS_TONE[room.thread.status] ?? 'muted');
  const asks = room.thread.status === 'needs_you' && !room.done;
  const edge = asks ? colors.attention : tone === 'live' && !room.done ? colors.live : colors.line;
  const status = room.done ? t('done') : t(STATE_LABELS[room.thread.status] ?? 'Idle');
  const when = since(room.since);
  return (
    <Animated.View
      entering={reduced || !room.arriving ? undefined : FadeInDown.duration(400)}
      exiting={reduced || !room.closing ? undefined : FadeOut.duration(300)}
      style={{
        borderRadius: 12,
        borderWidth: asks ? 3 : edge === colors.line ? 1 : 1.5,
        borderColor: edge,
        backgroundColor: colors.card,
        overflow: 'hidden',
        opacity: room.closing ? 0.5 : 1,
      }}
    >
      <View style={{ flexDirection: 'row', alignItems: 'center' }}>
        <Pressable
          testID={`room-${room.id}`}
          accessibilityRole="button"
          accessibilityLabel={`${room.title || t('New conversation')}, ${status}${when ? `, ${when}` : ''}`}
          onPress={() => onOpen(room)}
          style={{
            flex: 1,
            flexDirection: 'row',
            alignItems: 'center',
            gap: 10,
            padding: 14,
            minHeight: 56,
          }}
        >
          <Icon
            name={MARK[room.done ? 'done' : room.thread.status] ?? 'ellipse-outline'}
            size={20}
            color={
              tone === 'ask'
                ? colors.text
                : tone === 'live' || tone === 'ok'
                  ? colors.live
                  : colors.muted
            }
          />
          <View style={{ flex: 1 }}>
            <Text numberOfLines={1} style={{ color: colors.text, fontSize: 16, fontWeight: '700' }}>
              {room.title || t('New conversation')}
            </Text>
            <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
              {status}
              {when ? ` · ${when}` : ''}
            </Text>
          </View>
          <Icon name="chevron-forward" size={18} color={colors.muted} />
        </Pressable>
        {onCollapse ? (
          <Pressable
            testID={`collapse-${room.id}`}
            accessibilityRole="button"
            accessibilityLabel={t('Collapse this room')}
            onPress={() => onCollapse(room)}
            style={{
              width: 48,
              alignSelf: 'stretch',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <Icon name="chevron-up" size={20} color={colors.sub} />
          </Pressable>
        ) : null}
      </View>
      <View style={{ paddingHorizontal: 8, paddingBottom: 8 }}>
        {room.people.length ? (
          <View style={{ flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'center' }}>
            {picked.shown.map((p, i) => (
              <Seat key={`${p.key}:${i}`} person={p} icon={icons[p.provider]} onPress={onPerson} />
            ))}
            {picked.more ? <MoreTile room={room} picked={picked} onPress={onMore} /> : null}
          </View>
        ) : (
          <Text style={{ color: colors.sub, padding: 12, textAlign: 'center' }}>
            {t('Nobody is here yet.')}
          </Text>
        )}
      </View>
      {room.lines.length ? (
        <View
          style={{
            gap: 8,
            padding: 12,
            paddingTop: 10,
            borderTopWidth: 1,
            borderTopColor: colors.line,
          }}
        >
          {room.lines.map((l) => (
            <Line key={l.seq} line={l} />
          ))}
        </View>
      ) : null}
    </Animated.View>
  );
}

export const RoomCard = memo(RoomCardView);
