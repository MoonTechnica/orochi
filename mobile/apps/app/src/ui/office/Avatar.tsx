import React, { useEffect, useMemo } from 'react';
import { Image, Text, View } from 'react-native';
import Svg, { Path } from 'react-native-svg';
import Animated, {
  Easing,
  cancelAnimation,
  useAnimatedStyle,
  useSharedValue,
  withRepeat,
  withSequence,
  withTiming,
} from 'react-native-reanimated';
import {
  brandOf,
  faceOf,
  kindOf,
  lookFor,
  seedOf,
  type Person,
  type PersonState,
} from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { Character } from './Character';
import { useOfficeMotion } from './motion';
import { BRAND_PATHS } from './brandPaths';
import { BADGE_IMAGE_GROUND, BADGE_LETTER, vendorColor } from './avatarPalette';

const LETTERS: Record<string, string> = { anthropic: 'C', openai: 'O', google: 'G' };

type Face = Pick<Person, 'role' | 'lead' | 'provider' | 'agent' | 'state' | 'speaking'> & {
  model?: string | null;
};

/**
 * The round avatar: the ring says what it is doing (solid and pulsing while it works, dashed while
 * idle, red while it asks) and the provider's badge sits at its corner.
 */
export function Avatar({
  person,
  size = 56,
  icon,
  mail = false,
  still = false,
}: {
  person: Face;
  size?: number;
  icon?: string;
  mail?: boolean;
  /** No pulse or bob: for small avatars that only mark who is there. */
  still?: boolean;
}) {
  const { colors } = usePrefs();
  const calm = !useOfficeMotion();
  const look = useMemo(
    () => lookFor(seedOf(person), kindOf({ role: person.role, lead: person.lead })),
    [person.role, person.agent, person.lead], // eslint-disable-line react-hooks/exhaustive-deps
  );
  const face = faceOf(person.state, person.speaking);
  const live = person.state === 'working' || person.state === 'asking';
  const moving = live && !still;
  const ring =
    person.state === 'asking'
      ? colors.attention
      : person.state === 'working'
        ? colors.text
        : colors.muted;
  const pulse = useSharedValue(0);
  const bob = useSharedValue(0);
  useEffect(() => {
    if (calm || !moving) {
      cancelAnimation(pulse);
      cancelAnimation(bob);
      pulse.value = 0;
      bob.value = 0;
      return;
    }
    pulse.value = withRepeat(
      withTiming(1, { duration: 1800, easing: Easing.out(Easing.quad) }),
      -1,
    );
    bob.value = withRepeat(
      withSequence(withTiming(-1.5, { duration: 900 }), withTiming(0, { duration: 900 })),
      -1,
    );
    return () => {
      cancelAnimation(pulse);
      cancelAnimation(bob);
    };
  }, [moving, calm, pulse, bob]);
  const ringStyle = useAnimatedStyle(() => ({
    opacity: 0.7 * (1 - pulse.value),
    transform: [{ scale: 1 + 0.45 * pulse.value }],
  }));
  const bobStyle = useAnimatedStyle(() => ({ transform: [{ translateY: bob.value }] }));
  return (
    <View
      testID="avatar"
      style={{ width: size, height: size }}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {moving ? (
        <Animated.View
          pointerEvents="none"
          style={[
            {
              position: 'absolute',
              top: -2,
              left: -2,
              right: -2,
              bottom: -2,
              borderRadius: size,
              borderWidth: person.state === 'asking' ? 3 : 2,
              borderColor: ring,
            },
            ringStyle,
          ]}
        />
      ) : null}
      <View
        style={{
          width: size,
          height: size,
          borderRadius: size / 2,
          borderWidth: person.state === 'asking' ? 4 : 2,
          borderStyle: live ? 'solid' : 'dashed',
          borderColor: live ? ring : colors.line,
          backgroundColor: colors.bg,
          overflow: 'hidden',
          opacity: person.state === 'idle' ? 0.9 : 1,
        }}
      >
        {/* The illustration is always inside its own solid rim (white in dark, black in light),
            so a person whose hue is red-ish is a picture in a frame, never a state. */}
        <View
          style={{
            flex: 1,
            borderRadius: size / 2,
            borderWidth: 1.5,
            borderColor: colors.text,
            backgroundColor: colors.bg,
            overflow: 'hidden',
          }}
        >
          <Animated.View style={[{ flex: 1 }, bobStyle]}>
            <Character look={look} face={face} animate={!still && !calm} />
          </Animated.View>
        </View>
      </View>
      {person.agent ? (
        <ProviderBadge
          provider={person.provider}
          model={person.model}
          agent={person.agent}
          size={size}
          icon={icon}
        />
      ) : null}
      {mail ? <Envelope /> : null}
    </View>
  );
}

/** The provider's mark at the corner of a face (its own colour, its first letter or its image). */
export function ProviderBadge({
  provider,
  model,
  agent,
  size,
  icon,
}: {
  provider: string | null | undefined;
  model?: string | null;
  agent: string;
  size: number;
  icon?: string;
}) {
  const { colors } = usePrefs();
  const badge = Math.max(14, Math.round(size * 0.3));
  const brand = brandOf(provider, model);
  const letter = LETTERS[provider ?? ''] ?? (agent || '?').slice(0, 1).toUpperCase();
  return (
    <View
      style={{
        position: 'absolute',
        right: -size * 0.08,
        bottom: -size * 0.04,
        width: badge,
        height: badge,
        borderRadius: badge / 2,
        borderWidth: 1.5,
        borderColor: colors.bg,
        backgroundColor: icon || brand ? BADGE_IMAGE_GROUND : vendorColor(provider),
        alignItems: 'center',
        justifyContent: 'center',
        overflow: 'hidden',
      }}
    >
      {icon ? (
        <Image source={{ uri: icon }} style={{ width: badge - 5, height: badge - 5 }} />
      ) : brand ? (
        <Svg width={badge - 5} height={badge - 5} viewBox="0 0 24 24">
          <Path d={BRAND_PATHS[brand]} fill={vendorColor(brand)} />
        </Svg>
      ) : (
        <Text style={{ color: BADGE_LETTER, fontWeight: '700', fontSize: badge * 0.55 }}>
          {letter}
        </Text>
      )}
    </View>
  );
}

/** An unopened message waits on the one it was for; it does not blink. */
function Envelope() {
  const { colors } = usePrefs();
  return (
    <View
      style={{
        position: 'absolute',
        top: -5,
        right: -6,
        width: 18,
        height: 13,
        borderRadius: 3,
        borderWidth: 1.5,
        borderColor: colors.text,
        backgroundColor: colors.bg,
      }}
    >
      <View
        style={{
          position: 'absolute',
          top: -4,
          left: 2.5,
          width: 9,
          height: 9,
          borderRightWidth: 1.5,
          borderBottomWidth: 1.5,
          borderColor: colors.text,
          transform: [{ rotate: '45deg' }, { scale: 0.75 }],
        }}
      />
    </View>
  );
}

export const stateKey: Record<PersonState, string> = {
  working: 'working',
  asking: 'asking',
  mentioned: 'mentioned',
  reported: 'reported',
  idle: 'idle',
  leaving: 'leaving',
};

/** A sub-agent beside its seat: the same drawing, small, in a thin rim of the text colour. */
export function MiniAvatar({
  title,
  agent,
  size = 16,
}: {
  title: string;
  agent: string;
  size?: number;
}) {
  const { colors } = usePrefs();
  const look = useMemo(() => lookFor(`${title}|${agent}`, 'sub'), [title, agent]);
  return (
    <View
      accessibilityElementsHidden
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        borderWidth: 1.5,
        borderColor: colors.text,
        overflow: 'hidden',
      }}
    >
      <Character look={look} face={faceOf('working', false)} animate={false} />
    </View>
  );
}
