import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  Animated,
  Dimensions,
  Easing,
  Modal,
  PanResponder,
  Platform,
  Pressable,
  ScrollView,
  Text,
  View,
  useWindowDimensions,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { usePrefs } from '../state/prefs';

const native = Platform.OS !== 'web';

/**
 * A panel that slides in from the right edge (80% of the width, the rest dimmed). A tap on the
 * dimmed part, a swipe to the right on the panel and the system back gesture close it.
 * Content is up to the caller; `DrawerSection` and `DrawerItem` give it the usual shapes.
 */
export function Drawer({
  visible,
  onClose,
  children,
  label,
}: {
  visible: boolean;
  onClose: () => void;
  children: React.ReactNode;
  label?: string;
}) {
  const { colors, t } = usePrefs();
  const insets = useSafeAreaInsets();
  const { width: screen } = useWindowDimensions();
  const width = Math.round(screen * 0.8);
  const slide = useRef(new Animated.Value(width)).current;
  const [mounted, setMounted] = useState(visible);

  useEffect(() => {
    if (visible) {
      setMounted(true);
      slide.setValue(width);
      Animated.timing(slide, {
        toValue: 0,
        duration: 220,
        easing: Easing.out(Easing.cubic),
        useNativeDriver: native,
      }).start();
    } else if (mounted) {
      Animated.timing(slide, {
        toValue: width,
        duration: 180,
        easing: Easing.in(Easing.cubic),
        useNativeDriver: native,
      }).start(() => setMounted(false));
    }
  }, [visible]); // eslint-disable-line react-hooks/exhaustive-deps

  const pan = useMemo(
    () =>
      PanResponder.create({
        onStartShouldSetPanResponder: () => true,
        onMoveShouldSetPanResponder: (_, g) => g.dx > 12 && Math.abs(g.dx) > Math.abs(g.dy) * 1.5,
        onPanResponderMove: (_, g) => slide.setValue(Math.max(0, g.dx)),
        onPanResponderRelease: (_, g) => {
          if (g.dx > width * 0.25 || g.vx > 0.6) onClose();
          else
            Animated.timing(slide, {
              toValue: 0,
              duration: 120,
              useNativeDriver: native,
            }).start();
        },
      }),
    [width, onClose, slide],
  );

  const dim = slide.interpolate({
    inputRange: [0, width],
    outputRange: [0.45, 0],
    extrapolate: 'clamp',
  });
  return (
    <Modal visible={mounted} transparent animationType="none" onRequestClose={onClose}>
      <View style={{ flex: 1 }} accessibilityViewIsModal>
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
          style={{ position: 'absolute', top: 0, bottom: 0, left: 0, right: 0 }}
          onPress={onClose}
          accessibilityRole="button"
          accessibilityLabel={t('Close panel')}
        />
        <Animated.View
          {...pan.panHandlers}
          accessibilityLabel={label}
          style={{
            position: 'absolute',
            top: 0,
            bottom: 0,
            right: 0,
            width,
            backgroundColor: colors.card,
            borderLeftWidth: 1,
            borderLeftColor: colors.line,
            paddingTop: insets.top,
            paddingBottom: insets.bottom,
            transform: [{ translateX: slide }],
          }}
        >
          <ScrollView
            contentContainerStyle={{ paddingBottom: 24 }}
            keyboardShouldPersistTaps="handled"
          >
            {children}
          </ScrollView>
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
