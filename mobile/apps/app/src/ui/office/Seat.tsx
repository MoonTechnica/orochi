import React, { memo, useEffect, useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import Animated, { FadeIn, FadeOut, useReducedMotion } from 'react-native-reanimated';
import { span, type Person } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Avatar, MiniAvatar } from './Avatar';

export const nameOf = (p: Person, t: (k: string) => string) =>
  p.role || (p.lead ? t('lead') : p.agent);

export function stateLabel(p: Person, t: (k: string) => string) {
  return t(p.state);
}

/** Seconds of work, ticking on its own so the room is not redrawn each second. */
export function Clock({ at, style }: { at: number; style?: object }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  return <Text style={style}>{span(now - at)}</Text>;
}

/** What a person says: a small bubble below the name, pointing up at the avatar. */
function Bubble({ person }: { person: Person }) {
  const { colors, t } = usePrefs();
  const b = person.bubble;
  if (!b) return null;
  const ask = b.kind === 'ask';
  const body = ask ? t('Needs you') : b.text || t('working…');
  const to = b.kind === 'talk' ? `→ ${b.to ?? t('everyone')}${b.away ? ` · ${b.away}` : ''}` : '';
  const fill = ask ? colors.primary : colors.raised;
  const edge = ask ? colors.primary : colors.sub;
  return (
    <View style={{ alignItems: 'center', marginTop: 6, maxWidth: '100%' }}>
      <View
        style={{
          width: 8,
          height: 8,
          marginBottom: -5,
          backgroundColor: fill,
          borderColor: edge,
          borderLeftWidth: 1.5,
          borderTopWidth: 1.5,
          transform: [{ rotate: '45deg' }],
          zIndex: 1,
        }}
      />
      <View
        style={{
          paddingHorizontal: 8,
          paddingVertical: 4,
          borderRadius: 12,
          borderWidth: 1.5,
          borderColor: edge,
          backgroundColor: fill,
          maxWidth: '100%',
        }}
      >
        {to ? (
          <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 10 }}>
            {to}
          </Text>
        ) : null}
        <Text
          numberOfLines={2}
          style={{
            color: ask ? colors.onPrimary : colors.text,
            fontSize: 11,
            lineHeight: 14,
            fontWeight: ask ? '700' : '400',
            textAlign: 'center',
          }}
        >
          {body}
        </Text>
      </View>
    </View>
  );
}

/** One person at a desk: avatar, name, what they say, and the sub-agents beside them. */
function SeatView({
  person,
  icon,
  onPress,
}: {
  person: Person;
  icon?: string;
  onPress: (p: Person) => void;
}) {
  const { colors, t } = usePrefs();
  const reduced = useReducedMotion();
  const name = nameOf(person, t);
  const what = person.bubble?.text || person.doing;
  const label = [
    name,
    [person.agent, person.model].filter(Boolean).join(' '),
    stateLabel(person, t),
    what,
    person.tasks.length ? `${person.tasks.length} ${t('sub-agent')}` : '',
  ]
    .filter(Boolean)
    .join(', ');
  return (
    <Animated.View
      entering={reduced || !person.arriving ? undefined : FadeIn.duration(500)}
      exiting={reduced || person.state !== 'leaving' ? undefined : FadeOut.duration(400)}
      style={{ width: '33.33%', paddingHorizontal: 4, paddingVertical: 8 }}
    >
      <Pressable
        testID={`seat-${person.key}`}
        accessibilityRole="button"
        accessibilityLabel={label}
        onPress={() => onPress(person)}
        style={{ alignItems: 'center', opacity: person.state === 'leaving' ? 0.5 : 1 }}
      >
        <View style={{ paddingBottom: 6 }}>
          <Avatar person={person} icon={icon} mail={Boolean(person.mail)} />
        </View>
        <Text
          numberOfLines={1}
          style={{
            color: colors.text,
            fontSize: 12,
            fontWeight: '700',
            textDecorationLine: person.lead ? 'underline' : 'none',
            maxWidth: '100%',
          }}
        >
          {name}
        </Text>
        <View style={{ flexDirection: 'row', gap: 5, maxWidth: '100%' }}>
          {person.startedAt ? (
            <Clock at={person.startedAt} style={{ color: colors.text, fontSize: 10 }} />
          ) : null}
          {person.model ? (
            <Text numberOfLines={1} style={{ color: colors.muted, fontSize: 10, flexShrink: 1 }}>
              {person.model}
            </Text>
          ) : null}
        </View>
        <Bubble person={person} />
        {person.tasks.length ? (
          <View style={{ marginTop: 5, gap: 2, alignSelf: 'stretch' }}>
            {person.tasks.slice(0, 2).map((k) => (
              <View key={k.id} style={{ flexDirection: 'row', alignItems: 'center', gap: 4 }}>
                <MiniAvatar title={k.title || String(k.id)} agent={person.agent} />
                <Text numberOfLines={1} style={{ color: colors.muted, fontSize: 9, flex: 1 }}>
                  {k.title || t('sub-agent')}
                </Text>
              </View>
            ))}
            {person.tasks.length > 2 ? (
              <Text style={{ color: colors.muted, fontSize: 9 }}>+{person.tasks.length - 2}</Text>
            ) : null}
          </View>
        ) : null}
      </Pressable>
    </Animated.View>
  );
}

export const Seat = memo(SeatView);
