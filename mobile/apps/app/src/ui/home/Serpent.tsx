import React, { useEffect, useMemo } from 'react';
import { Pressable, View, useWindowDimensions } from 'react-native';
import Svg, { Circle, Path } from 'react-native-svg';
import Animated, {
  Easing,
  cancelAnimation,
  useAnimatedProps,
  useReducedMotion,
  useSharedValue,
  withRepeat,
  withTiming,
} from 'react-native-reanimated';
import type { AgentKind, AgentRow } from '@orochi/core';
import { usePrefs } from '../../state/prefs';

const AnimatedPath = Animated.createAnimatedComponent(Path);
const AnimatedCircle = Animated.createAnimatedComponent(Circle);

export const HEADS = 8;
const HEIGHT = 92;
const PAD = 22;

/** One sinuous line across the width; eight slots sit on it, the live agents fill them in order. */
export function Serpent({
  agents,
  onOpen,
}: {
  agents: AgentRow[];
  onOpen: (row: AgentRow) => void;
}) {
  const { colors, t } = usePrefs();
  const { width: screen } = useWindowDimensions();
  const width = Math.min(screen, 560) - 36;
  const reduced = useReducedMotion();
  const flow = useSharedValue(0);
  const pulse = useSharedValue(0);
  const working = agents.some((a) => a.kind === 'working');

  const y = (x: number) => HEIGHT / 2 + 20 * Math.sin((x / width) * Math.PI * 3.2 + 0.6);
  const d = useMemo(() => {
    let out = '';
    for (let x = 0; x <= width; x += 6) out += `${x === 0 ? 'M' : 'L'}${x} ${y(x).toFixed(1)}`;
    return out;
  }, [width]); // eslint-disable-line react-hooks/exhaustive-deps
  const slots = Array.from({ length: HEADS }, (_, i) => {
    const x = PAD + (i * (width - 2 * PAD)) / (HEADS - 1);
    return { x, y: y(x) };
  });

  useEffect(() => {
    if (reduced) {
      cancelAnimation(flow);
      cancelAnimation(pulse);
      flow.value = 0;
      pulse.value = 0.4;
      return;
    }
    flow.value = 0;
    flow.value = withRepeat(withTiming(1, { duration: 1600, easing: Easing.linear }), -1, false);
    pulse.value = 0;
    pulse.value = withRepeat(
      withTiming(1, { duration: 1500, easing: Easing.out(Easing.quad) }),
      -1,
    );
    return () => {
      cancelAnimation(flow);
      cancelAnimation(pulse);
    };
  }, [reduced, flow, pulse]);

  const flowProps = useAnimatedProps(() => ({ strokeDashoffset: -flow.value * 22 }));
  const haloProps = useAnimatedProps(() => ({
    r: 9 + pulse.value * 9,
    opacity: 0.7 * (1 - pulse.value),
  }));

  const tone = (k: AgentKind) =>
    k === 'approval' || k === 'mention'
      ? colors.attention
      : k === 'working'
        ? colors.live
        : colors.muted;

  return (
    <View style={{ width, height: HEIGHT, alignSelf: 'center' }} testID="serpent">
      <Svg width={width} height={HEIGHT} style={{ position: 'absolute' }}>
        <Path d={d} stroke={colors.sub} strokeWidth={1.25} fill="none" strokeLinecap="round" />
        {working ? (
          <AnimatedPath
            d={d}
            stroke={colors.text}
            strokeWidth={2}
            fill="none"
            strokeLinecap="round"
            strokeDasharray="5 17"
            animatedProps={flowProps}
            opacity={0.9}
          />
        ) : null}
        {slots.map((s, i) => {
          const a = agents[i];
          if (!a) return <Circle key={i} cx={s.x} cy={s.y} r={2.5} fill={colors.muted} />;
          const need = a.kind === 'approval' || a.kind === 'mention';
          return (
            <React.Fragment key={i}>
              {need ? (
                <AnimatedCircle
                  cx={s.x}
                  cy={s.y}
                  fill={colors.attention}
                  animatedProps={haloProps}
                />
              ) : null}
              <Circle
                cx={s.x}
                cy={s.y}
                r={need ? 8 : a.kind === 'working' ? 6 : 4.5}
                fill={tone(a.kind)}
                opacity={a.kind === 'background' ? 0.7 : 1}
              />
            </React.Fragment>
          );
        })}
      </Svg>
      {agents.slice(0, HEADS).map((a, i) => (
        <Pressable
          key={a.id}
          testID={`head-${i}`}
          accessibilityRole="button"
          accessibilityLabel={`${a.title || t('New conversation')}, ${
            a.kind === 'approval'
              ? t('Waiting for approval')
              : a.kind === 'mention'
                ? t('Mentioned you')
                : a.kind === 'working'
                  ? t('Working')
                  : t('Watching in the background')
          }`}
          onPress={() => onOpen(a)}
          style={{
            position: 'absolute',
            left: slots[i].x - 22,
            top: slots[i].y - 22,
            width: 44,
            height: 44,
          }}
        />
      ))}
    </View>
  );
}
