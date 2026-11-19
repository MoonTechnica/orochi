import React, { memo, useEffect } from 'react';
import { Text, View } from 'react-native';
import { faceOf, isAnonymous, lookOfWho, whoKey, type PersonState, type Who } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { useYouSeed } from '../../state/you';
import { Character } from '../office/Character';
import { ProviderBadge } from '../office/Avatar';

/** The conversation's face size, and where the text beside it starts (12 margin + face + 10 gap). */
export const FACE = 34;
export const INDENT = 12 + FACE + 10;

interface Props {
  who: Who;
  size?: number;
  /** A live state picks the expression; without one it is the neutral smile. */
  state?: PersonState;
  /** The provider's mark at the corner (agents only; off below 28 px unless asked). */
  badge?: boolean;
  /** The name is written beside it: hidden from the accessibility tree. Off = an image named for the person. */
  decorative?: boolean;
}

/** Dev builds count the faces mounted and drawn (`globalThis.__faces`), to measure a long list. */
function useCounted() {
  const g = globalThis as { __faces?: { mounted: number; renders: number } };
  if (__DEV__) {
    g.__faces ??= { mounted: 0, renders: 0 };
    g.__faces.renders++;
  }
  useEffect(() => {
    if (!__DEV__) return;
    g.__faces!.mounted++;
    return () => {
      g.__faces!.mounted--;
    };
  }, [g]);
}

/**
 * A speaker as the Office draws them (same character for the same role and agent), static: no blink,
 * no moving mouth, no animation hooks, so a long conversation can hold hundreds. It is memoized on
 * what it shows (identity, size, state), never on the objects that carry it.
 *
 * The human is told from an agent at a glance twice: a ring set off from the face by a gap in the
 * background colour, and no provider badge.
 */
function WhoFaceView({ who, size = FACE, state, badge, decorative = true }: Props) {
  const { colors } = usePrefs();
  const seed = useYouSeed();
  useCounted();
  const hide = decorative
    ? ({
        accessibilityElementsHidden: true,
        importantForAccessibility: 'no-hide-descendants',
      } as const)
    : ({ accessible: true, accessibilityRole: 'image', accessibilityLabel: who.name } as const);
  if (isAnonymous(who)) {
    return (
      <View
        {...hide}
        style={{
          width: size,
          height: size,
          borderRadius: size / 2,
          borderWidth: 1,
          borderColor: colors.muted,
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <Text style={{ color: colors.text, fontWeight: '700', fontSize: size * 0.42 }}>
          {(who.name || '?').slice(0, 1).toUpperCase()}
        </Text>
      </View>
    );
  }
  const look = lookOfWho(who, seed);
  const face = faceOf(state ?? 'reported', false);
  if (who.you) {
    return (
      <View
        {...hide}
        style={{
          width: size,
          height: size,
          borderRadius: size / 2,
          borderWidth: 1.5,
          borderColor: colors.text,
          padding: Math.max(2, Math.round(size * 0.07)),
          backgroundColor: colors.bg,
        }}
      >
        <View style={{ flex: 1, borderRadius: size / 2, overflow: 'hidden' }}>
          <Character look={look} face={face} still />
        </View>
      </View>
    );
  }
  const marked = badge ?? size >= 28;
  return (
    <View {...hide} style={{ width: size, height: size }}>
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
        <Character look={look} face={face} still />
      </View>
      {marked && who.agent ? (
        <ProviderBadge provider={who.provider} model={who.model} agent={who.agent} size={size} />
      ) : null}
    </View>
  );
}

const same = (a: Props, b: Props) =>
  whoKey(a.who) === whoKey(b.who) &&
  a.who.provider === b.who.provider &&
  a.who.name === b.who.name &&
  a.size === b.size &&
  a.state === b.state &&
  a.badge === b.badge &&
  a.decorative === b.decorative;

export const WhoFace = memo(WhoFaceView, same);
