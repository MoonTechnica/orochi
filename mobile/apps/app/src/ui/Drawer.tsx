import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  Animated,
  Dimensions,
  Easing,
  KeyboardAvoidingView,
  Modal,
  PanResponder,
  Platform,
  Pressable,
  ScrollView,
  Text,
  View,
  useWindowDimensions,
} from 'react-native';
import { useReducedMotion } from 'react-native-reanimated';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';
import { HeaderButton } from './Screen';

const native = Platform.OS !== 'web';

/**
 * A panel that slides in from a screen edge. Tap outside, swipe back towards the edge,
 * or use the system back action to close it.
 * Content is up to the caller; `DrawerSection` and `DrawerItem` give it the usual shapes.
 */
export function Drawer({
  visible,
  onClose,
  children,
  label,
  side = 'right',
  title,
}: {
  visible: boolean;
  onClose: () => void;
  children: React.ReactNode;
  label?: string;
  side?: 'left' | 'right';
  title?: string;
}) {
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();
  const { width: screen } = useWindowDimensions();
  const fromLeft = side === 'left';
  // Update the panel's measured frame so iOS exposes its controls on each opening.
  const layoutSlide = fromLeft && Platform.OS === 'ios';
  const direction = fromLeft ? -1 : 1;
  const width = Math.round(Math.min(screen * 0.8, fromLeft ? 360 : screen));
  const reduced = useReducedMotion();
  const slide = useRef(new Animated.Value(1)).current;
  const [mounted, setMounted] = useState(visible);

  useEffect(() => {
    if (visible) setMounted(true);
    if (reduced) {
      slide.setValue(visible ? 0 : 1);
      if (!visible) setMounted(false);
      return;
    }
    const animation = Animated.timing(slide, {
      toValue: visible ? 0 : 1,
      duration: visible ? 220 : 180,
      easing: visible ? Easing.out(Easing.cubic) : Easing.in(Easing.cubic),
      useNativeDriver: native && !layoutSlide,
    });
    animation.start(({ finished }) => {
      if (finished && !visible) setMounted(false);
    });
    return () => animation.stop();
  }, [visible, slide, reduced, layoutSlide]);

  const pan = useMemo(() => {
    const restore = () =>
      Animated.timing(slide, {
        toValue: 0,
        duration: reduced ? 0 : 120,
        useNativeDriver: native && !layoutSlide,
      }).start();
    return PanResponder.create({
      onStartShouldSetPanResponder: () => true,
      onMoveShouldSetPanResponderCapture: (_, g) =>
        g.dx * direction > 12 && Math.abs(g.dx) > Math.abs(g.dy) * 1.5,
      onPanResponderMove: (_, g) => {
        slide.setValue(Math.max(0, (g.dx * direction) / width));
      },
      onPanResponderRelease: (_, g) => {
        if (g.dx * direction > width * 0.25 || g.vx * direction > 0.6) onClose();
        else restore();
      },
      onPanResponderTerminate: restore,
    });
  }, [direction, width, onClose, reduced, slide, layoutSlide]);

  const dim = slide.interpolate({
    inputRange: [0, 1],
    outputRange: [0.45, 0],
    extrapolate: 'clamp',
  });
  const translateX = slide.interpolate({
    inputRange: [0, 1],
    outputRange: [0, width * direction],
    extrapolate: 'clamp',
  });
  return (
    <Modal visible={mounted} transparent animationType="none" onRequestClose={onClose}>
      <View style={{ flex: 1 }} accessibilityViewIsModal={!fromLeft}>
        <Animated.View
          pointerEvents="none"
          style={{
            position: 'absolute',
            top: 0,
            bottom: 0,
            left: 0,
            right: 0,
            backgroundColor: '#000',
            opacity: dim,
          }}
        />
        <Pressable
          testID="drawer-backdrop"
          style={{
            position: 'absolute',
            top: 0,
            bottom: 0,
            left: fromLeft ? width : 0,
            right: fromLeft ? 0 : width,
          }}
          onPress={onClose}
          accessibilityRole={title ? undefined : 'button'}
          accessibilityLabel={title ? undefined : t('Close panel')}
          accessible={!title}
          tabIndex={title ? -1 : 0}
        />
        <Animated.View
          {...pan.panHandlers}
          accessibilityLabel={label}
          accessibilityViewIsModal={fromLeft}
          style={{
            position: 'absolute',
            top: 0,
            bottom: 0,
            ...(fromLeft
              ? {
                  left: layoutSlide ? translateX : 0,
                  borderRightWidth: 1,
                  paddingLeft: insets.left,
                }
              : { right: 0, borderLeftWidth: 1, paddingRight: insets.right }),
            width,
            backgroundColor: colors.card,
            borderColor: colors.line,
            paddingTop: insets.top,
            paddingBottom: insets.bottom,
            transform: layoutSlide ? undefined : [{ translateX }],
          }}
        >
          {title ? (
            <View
              style={{
                flexDirection: 'row',
                alignItems: 'center',
                gap: 8,
                paddingLeft: 16,
                paddingRight: 8,
                minHeight: 52,
              }}
            >
              <Text
                accessibilityRole="header"
                style={{ flex: 1, color: colors.text, fontSize: 18, fontWeight: '800' }}
              >
                {title}
              </Text>
              <HeaderButton icon="close" label={t('Close panel')} onPress={onClose} />
            </View>
          ) : null}
          <KeyboardAvoidingView
            style={{ flex: 1 }}
            behavior={fromLeft && Platform.OS === 'ios' ? 'padding' : undefined}
          >
            <ScrollView
              contentContainerStyle={{ paddingBottom: 24 }}
              keyboardShouldPersistTaps="handled"
            >
              {children}
            </ScrollView>
          </KeyboardAvoidingView>
        </Animated.View>
      </View>
    </Modal>
  );
}

export function DrawerSection({ title, children }: { title?: string; children: React.ReactNode }) {
  const { colors } = usePrefs();
  return (
    <View style={{ borderTopWidth: 1, borderTopColor: colors.line, paddingVertical: 6 }}>
      {title ? (
        <Text
          style={{
            color: colors.sub,
            fontSize: 12,
            fontWeight: '700',
            textTransform: 'uppercase',
            paddingHorizontal: 18,
            paddingTop: 8,
            paddingBottom: 4,
          }}
        >
          {title}
        </Text>
      ) : null}
      {children}
    </View>
  );
}

export function DrawerItem({
  label,
  detail,
  onPress,
  danger,
  selected,
  badge,
  leading,
}: {
  label: string;
  detail?: string;
  /** A face or mark before the label. */
  leading?: React.ReactNode;
  onPress: () => void;
  danger?: boolean;
  selected?: boolean;
  badge?: string;
}) {
  const { colors } = usePrefs();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={{
        minHeight: 48,
        paddingHorizontal: 18,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
      }}
    >
      {leading}
      <View style={{ flex: 1 }}>
        <Text
          numberOfLines={1}
          style={{
            color: danger ? colors.dangerInk : colors.text,
            fontSize: 16,
            fontWeight: selected ? '700' : '400',
          }}
        >
          {label}
        </Text>
        {detail ? (
          <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
            {detail}
          </Text>
        ) : null}
      </View>
      {badge ? <Text style={{ color: colors.text, fontWeight: '700' }}>{badge}</Text> : null}
    </Pressable>
  );
}

/** A strip on the right edge that opens a drawer when dragged to the left. */
export function EdgeOpen({ onOpen }: { onOpen: () => void }) {
  const { t } = usePrefs();
  const pan = useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onPanResponderRelease: (_, g) => {
          if (g.dx < -36) onOpen();
        },
      }),
    [onOpen],
  );
  void Dimensions;
  return (
    <View
      {...pan.panHandlers}
      accessibilityLabel={t('Open panel by dragging')}
      style={{ position: 'absolute', top: 120, bottom: 120, right: 0, width: 18 }}
    />
  );
}
