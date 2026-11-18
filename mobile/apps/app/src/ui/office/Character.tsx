import React, { useEffect } from 'react';
import Svg, { Circle, Ellipse, G, Path, Rect } from 'react-native-svg';
import Animated, {
  cancelAnimation,
  Easing,
  useAnimatedProps,
  useReducedMotion,
  useSharedValue,
  withDelay,
  withRepeat,
  withSequence,
  withTiming,
} from 'react-native-reanimated';
import type { Face, Look } from '@orochi/core';
import { GROUNDS, HAIR_COLORS, INK, OUTFITS, SKINS } from './avatarPalette';
import { Accessory } from './parts/Accessory';
import { HairBack, HairFront } from './parts/Hair';

const AnimatedEllipse = Animated.createAnimatedComponent(Ellipse);

const EYE_X = [26.5, 37.5] as const;

/** Small dark eyes, no whites and no highlights; short lines when asleep. Blinks when focused. */
function Eyes({ face, animate }: { face: Face; animate: boolean }) {
  const reduced = useReducedMotion();
  const open = useSharedValue(1);
  const blink = animate && face.blink && !reduced;
  useEffect(() => {
    if (!blink) {
      cancelAnimation(open);
      open.value = 1;
      return;
    }
    open.value = withRepeat(
      withSequence(
        withDelay(2800, withTiming(0.1, { duration: 80 })),
        withTiming(1, { duration: 120 }),
      ),
      -1,
    );
    return () => cancelAnimation(open);
  }, [blink, open]);
  // Looking a little down while working.
  const y = face.expression === 'focus' ? 31.8 : 30.5;
  const props = useAnimatedProps(() => ({ ry: 2.2 * open.value }));

  return (
    <EyeShapes
      face={face}
      y={y}
      eye={(x) => (
        <AnimatedEllipse key={x} cx={x} cy={y} rx={1.7} fill={INK} animatedProps={props} />
      )}
    />
  );
}

/** The eyes, brows included, with the ellipse for one open eye supplied by the caller. */
function EyeShapes({
  face,
  y,
  eye,
}: {
  face: Face;
  y: number;
  eye: (x: number) => React.ReactNode;
}) {
  if (face.expression === 'sleepy') {
    return (
      <G stroke={INK} strokeWidth={1.7} strokeLinecap="round">
        <Path d="M24.4 31 L28.6 31" />
        <Path d="M35.4 31 L39.6 31" />
      </G>
    );
  }
  return (
    <G>
      {EYE_X.map(eye)}
      {face.expression === 'asking' ? (
        <G fill="none" stroke={INK} strokeWidth={1.4} strokeLinecap="round">
          <Path d="M23.3 25.6 Q26.5 23.2 29.7 25.2" />
          <Path d="M34.3 25.2 Q37.5 23.2 40.7 25.6" />
        </G>
      ) : null}
    </G>
  );
}

/** The same eyes with no animation hooks at all: for a long list of faces that never move. */
function StillEyes({ face }: { face: Face }) {
  const y = face.expression === 'focus' ? 31.8 : 30.5;
  return (
    <EyeShapes
      face={face}
      y={y}
      eye={(x) => <Ellipse key={x} cx={x} cy={y} rx={1.7} ry={2.2} fill={INK} />}
    />
  );
}

/** One thin line; a small ring when asking; it moves a little after the person spoke. */
function Mouth({ face, animate }: { face: Face; animate: boolean }) {
  const reduced = useReducedMotion();
  const ry = useSharedValue(1);
  const move = animate && face.talking && !reduced;
  useEffect(() => {
    if (!move) {
      cancelAnimation(ry);
      ry.value = 1.4;
      return;
    }
    ry.value = withRepeat(
      withTiming(2.3, { duration: 280, easing: Easing.inOut(Easing.quad) }),
      -1,
      true,
    );
    return () => cancelAnimation(ry);
  }, [move, ry]);
  const props = useAnimatedProps(() => ({ ry: ry.value }));
  if (face.talking)
    return <AnimatedEllipse cx={32} cy={38.6} rx={2.1} fill={INK} animatedProps={props} />;
  return <MouthShape face={face} />;
}

/** The resting mouth of an expression. */
function MouthShape({ face }: { face: Face }) {
  switch (face.expression) {
    case 'asking':
      return <Circle cx={32} cy={38.6} r={1.7} fill="none" stroke={INK} strokeWidth={1.5} />;
    case 'smile':
      return (
        <Path
          d="M26.8 36.8 Q32 43.2 37.2 36.8"
          fill="none"
          stroke={INK}
          strokeWidth={1.7}
          strokeLinecap="round"
        />
      );
    case 'sleepy':
      return (
        <Path
          d="M29 38.6 L35 38.6"
          fill="none"
          stroke={INK}
          strokeWidth={1.6}
          strokeLinecap="round"
        />
      );
    default:
      return (
        <Path
          d="M28.4 37.6 Q32 40 35.6 37.6"
          fill="none"
          stroke={INK}
          strokeWidth={1.7}
          strokeLinecap="round"
        />
      );
  }
}

/**
 * A person as in a product team page: a round head, a plain flat mass of hair, a shirt, two dots
 * for eyes and one thin line for a mouth. Flat colour only, no outlines, no gradients, no shadows.
 * The drawing is `look` and `face` (pure data from `@orochi/core`) and nothing else.
 */
export function Character({
  look,
  face,
  animate = true,
  still = false,
}: {
  look: Look;
  face: Face;
  animate?: boolean;
  /** Drawn with no animation hooks (not even idle ones); the face never moves. */
  still?: boolean;
}) {
  const skin = SKINS[look.skin]!;
  const hair = HAIR_COLORS[look.hairColor]!;
  const outfit = OUTFITS[look.outfit]!;
  return (
    <Svg viewBox="0 0 64 64" width="100%" height="100%">
      <Circle cx={32} cy={32} r={32} fill={GROUNDS[look.ground]} />
      <HairBack style={look.hair} fill={hair} />
      <Rect x={27} y={40} width={10} height={12} rx={3} fill={skin} />
      <Path d="M9 66 Q9 48 32 48 Q55 48 55 66 Z" fill={outfit} />
      <Circle cx={16.8} cy={31.5} r={2.8} fill={skin} />
      <Circle cx={47.2} cy={31.5} r={2.8} fill={skin} />
      <Ellipse cx={32} cy={29.5} rx={15.2} ry={16} fill={skin} />
      <HairFront style={look.hair} fill={hair} />
      {still ? <StillEyes face={face} /> : <Eyes face={face} animate={animate} />}
      {still ? <MouthShape face={face} /> : <Mouth face={face} animate={animate} />}
      <Accessory kind={look.accessory} />
    </Svg>
  );
}
