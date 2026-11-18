import React, { useMemo, useRef, useState } from 'react';
import { Animated, PanResponder, Platform, View } from 'react-native';
import { Image } from 'expo-image';
import {
  DOUBLE_TAP_MS,
  MAX_SCALE,
  MIN_SCALE,
  clampView,
  distance,
  doubleTap,
  isTap,
  pinch,
  tapKind,
  type Tap,
  type View2D,
} from '@orochi/core';
import { usePrefs } from '../state/prefs';

/**
 * Pinch to zoom, drag while zoomed, double tap to zoom in on the tapped point (and out again).
 * The maths is `@orochi/core`'s zoom.ts. Pinch and pan run on the core PanResponder, which only
 * claims a touch that is two fingers or a drag of a zoomed image, so a swipe at scale 1 still
 * belongs to the pager. A tap never reaches a responder that declines it, so taps are read from
 * the raw touch events (native) or clicks (web) instead.
 */
export function ZoomableImage({
  url,
  width,
  height,
  onZoomChange,
  onTap,
  onError,
}: {
  url: string;
  width: number;
  height: number;
  onZoomChange?: (zoomed: boolean) => void;
  onTap?: () => void;
  onError?: () => void;
}) {
  const { t } = usePrefs();
  const [zoomed, setZoomed] = useState(false);
  const scale = useRef(new Animated.Value(1)).current;
  const tx = useRef(new Animated.Value(0)).current;
  const ty = useRef(new Animated.Value(0)).current;
  const state = useRef({
    view: { s: 1, x: 0, y: 0 } as View2D,
    base: { s: 1, x: 0, y: 0 } as View2D,
    dist: 0,
    mid: { x: 0, y: 0 },
    last: null as Tap | null,
    pending: null as ReturnType<typeof setTimeout> | null,
    touch: { at: 0, x: 0, y: 0, travelled: 0, multi: false },
  }).current;

  const show = (v: View2D, animate: boolean) => {
    const c = clampView(v, width, height);
    state.view = c;
    if (animate) {
      Animated.parallel([
        Animated.timing(scale, { toValue: c.s, duration: 220, useNativeDriver: false }),
        Animated.timing(tx, { toValue: c.x, duration: 220, useNativeDriver: false }),
        Animated.timing(ty, { toValue: c.y, duration: 220, useNativeDriver: false }),
      ]).start();
    } else {
      scale.setValue(c.s);
      tx.setValue(c.x);
      ty.setValue(c.y);
    }
    setZoomed(c.s > 1.02);
    onZoomChange?.(c.s > 1.02);
  };

  // Relative to the frame's centre, which is where the image scales from.
  const centred = (x: number, y: number) => ({ x: x - width / 2, y: y - height / 2 });

  /** One finished tap at (x, y) in the frame: a double tap zooms, a lone one waits to be sure. */
  const tapped = (x: number, y: number) => {
    const tap: Tap = { at: Date.now(), x, y };
    if (tapKind(state.last, tap) === 'double') {
      state.last = null;
      if (state.pending) clearTimeout(state.pending);
      state.pending = null;
      show(doubleTap(state.view, centred(x, y), width, height), true);
      return;
    }
    state.last = tap;
    state.pending = setTimeout(() => {
      state.pending = null;
      state.last = null;
      onTap?.();
    }, DOUBLE_TAP_MS);
  };

  const mid = (touches: readonly { pageX: number; pageY: number }[]) => ({
    x: (touches[0]!.pageX + touches[1]!.pageX) / 2,
    y: (touches[0]!.pageY + touches[1]!.pageY) / 2,
  });
  const gap = (touches: readonly { pageX: number; pageY: number }[]) =>
    touches.length < 2
      ? 0
      : distance(
          { x: touches[0]!.pageX, y: touches[0]!.pageY },
          { x: touches[1]!.pageX, y: touches[1]!.pageY },
        );

  const responder = useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => false,
        onMoveShouldSetPanResponder: (e, g) =>
          e.nativeEvent.touches.length >= 2 ||
          (state.view.s > 1.02 && (Math.abs(g.dx) > 2 || Math.abs(g.dy) > 2)),
        onMoveShouldSetPanResponderCapture: (e) => e.nativeEvent.touches.length >= 2,
        onPanResponderTerminationRequest: () => false,
        onPanResponderGrant: (e) => {
          const touches = e.nativeEvent.touches as any;
          state.base = { ...state.view };
          state.dist = gap(touches);
          state.mid = touches.length >= 2 ? mid(touches) : { x: 0, y: 0 };
        },
        onPanResponderMove: (e, g) => {
          const touches = e.nativeEvent.touches as any;
          if (touches.length >= 2) {
            const d = gap(touches);
            const m = mid(touches);
            if (!state.dist) {
              // the second finger came down after the grab: the pinch starts here
              state.dist = d;
              state.mid = m;
              state.base = { ...state.view };
              return;
            }
            show(
              pinch(
                state.base,
                state.dist,
                d,
                centred(state.mid.x, state.mid.y),
                centred(m.x, m.y),
                width,
                height,
              ),
              false,
            );
          } else if (state.view.s > 1.02) {
            show({ s: state.view.s, x: state.base.x + g.dx, y: state.base.y + g.dy }, false);
          }
        },
        onPanResponderRelease: () => {
          state.dist = 0;
          if (state.view.s < 1.05) show({ s: MIN_SCALE, x: 0, y: 0 }, true);
          else if (state.view.s > MAX_SCALE) show({ ...state.view, s: MAX_SCALE }, true);
        },
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [width, height],
  );

  const touches =
    Platform.OS === 'web'
      ? {}
      : {
          onTouchStart: (e: any) => {
            const n = e.nativeEvent;
            if (n.touches.length > 1) state.touch.multi = true;
            else
              state.touch = { at: Date.now(), x: n.pageX, y: n.pageY, travelled: 0, multi: false };
          },
          onTouchMove: (e: any) => {
            const n = e.nativeEvent;
            state.touch.travelled = Math.max(
              state.touch.travelled,
              distance({ x: n.pageX, y: n.pageY }, state.touch),
            );
          },
          onTouchEnd: (e: any) => {
            const n = e.nativeEvent;
            const t = state.touch;
            const last = n.touches.length === 0;
            if (last && !t.multi && isTap(Date.now() - t.at, t.travelled)) {
              // locationX is relative to the frame; pageX is not when the pager is scrolled
              tapped(n.locationX ?? t.x, n.locationY ?? t.y);
            }
            if (last) state.touch = { at: 0, x: 0, y: 0, travelled: 0, multi: false };
          },
        };

  const web =
    Platform.OS === 'web'
      ? {
          onWheel: (e: any) =>
            show(
              {
                s: Math.max(
                  MIN_SCALE,
                  Math.min(MAX_SCALE, state.view.s * (e.deltaY < 0 ? 1.15 : 0.87)),
                ),
                x: state.view.x,
                y: state.view.y,
              },
              false,
            ),
          onClick: (e: any) => {
            const r = e.currentTarget?.getBoundingClientRect?.();
            tapped(r ? e.clientX - r.left : width / 2, r ? e.clientY - r.top : height / 2);
          },
        }
      : {};

  return (
    <View
      testID="zoomable-image"
      accessible
      accessibilityLabel={zoomed ? t('Image, zoomed') : t('Image, fit to screen')}
      style={{ width, height, overflow: 'hidden' }}
      {...responder.panHandlers}
      {...touches}
      {...(web as object)}
    >
      <Animated.View
        style={{ width, height, transform: [{ translateX: tx }, { translateY: ty }, { scale }] }}
      >
        <Image
          source={{ uri: url }}
          style={{ width, height }}
          contentFit="contain"
          onError={onError}
        />
      </Animated.View>
    </View>
  );
}
