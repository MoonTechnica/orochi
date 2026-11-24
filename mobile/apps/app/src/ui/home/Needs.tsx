import React, { useRef, useState } from 'react';
import { Animated, PanResponder, Pressable, Text, View } from 'react-native';
import { since, whoOf, type NeedItem } from '@orochi/core';
import { Icon } from '../Icon';
import { usePrefs } from '../../state/prefs';
import { WhoFace } from '../people/WhoFace';
import { mono, radius } from '../theme';

function Button({
  label,
  onPress,
  primary,
  disabled,
  testID,
}: {
  label: string;
  onPress: () => void;
  primary?: boolean;
  disabled?: boolean;
  testID?: string;
}) {
  const { colors } = usePrefs();
  return (
    <Pressable
      testID={testID}
      accessibilityRole="button"
      accessibilityLabel={label}
      disabled={disabled}
      onPress={onPress}
      accessibilityState={{ disabled }}
      style={({ pressed }) => ({
        minHeight: 44,
        paddingHorizontal: 11,
        borderRadius: radius.pill,
        justifyContent: 'center',
        backgroundColor: primary ? colors.attention : 'transparent',
        borderWidth: primary ? 0 : 1,
        borderColor: colors.line,
        opacity: disabled ? 0.5 : pressed ? 0.75 : 1,
      })}
    >
      <Text
        style={{ color: primary ? colors.onPrimary : colors.text, fontWeight: '700', fontSize: 14 }}
      >
        {label}
      </Text>
    </Pressable>
  );
}

/** A thing that waits for you, with its answer on the card: Allow once / Deny, or Reply. */
export function NeedCard({
  item,
  many,
  onAnswer,
  onOpen,
  onReply,
  onDismiss,
}: {
  item: NeedItem;
  many: boolean;
  onAnswer: (item: NeedItem, option: string | null) => Promise<void>;
  onOpen: (item: NeedItem) => void;
  onReply: (item: NeedItem) => void;
  onDismiss: (item: NeedItem) => void;
}) {
  const { colors, t } = usePrefs();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const x = useRef(new Animated.Value(0)).current;
  const swipe = item.kind === 'mention';
  const pan = useRef(
    PanResponder.create({
      onMoveShouldSetPanResponder: (_, g) =>
        swipe && Math.abs(g.dx) > 14 && Math.abs(g.dx) > Math.abs(g.dy) * 1.5,
      onPanResponderMove: Animated.event([null, { dx: x }], { useNativeDriver: false }),
      onPanResponderRelease: (_, g) => {
        if (Math.abs(g.dx) > 96) {
          Animated.timing(x, {
            toValue: g.dx > 0 ? 400 : -400,
            duration: 140,
            useNativeDriver: false,
          }).start(() => onDismiss(item));
        } else Animated.spring(x, { toValue: 0, useNativeDriver: false }).start();
      },
    }),
  ).current;

  const answer = async (option: string | null) => {
    setBusy(true);
    setFailed(false);
    try {
      await onAnswer(item, option);
    } catch {
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };
  const p = item.prompt;
  const where = [item.project, many ? item.machineName : null].filter(Boolean).join(' · ');
  const options = p ? p.options.slice(0, 3) : [];
  const hasNo = options.some(([, name]) => /reject|deny|decline|cancel/i.test(name));

  return (
    <Animated.View
      {...(swipe ? pan.panHandlers : {})}
      testID={`need-${item.kind}`}
      style={{
        transform: [{ translateX: x }],
        backgroundColor: colors.card,
        borderRadius: radius.tile,
        borderWidth: 1,
        borderColor: colors.line,
        padding: 14,
        gap: 10,
      }}
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <Icon name={p ? 'shield-checkmark-outline' : 'at'} size={17} color={colors.text} />
        <Text
          numberOfLines={1}
          style={{
            color: colors.text,
            fontWeight: '800',
            fontSize: 12,
            flex: 1,
            letterSpacing: 0.4,
          }}
        >
          {(item.kind === 'approval'
            ? t('Waiting for approval')
            : t('Mentioned you')
          ).toUpperCase()}
        </Text>
        {item.at ? <Text style={{ color: colors.sub, fontSize: 12 }}>{since(item.at)}</Text> : null}
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${item.title || t('New conversation')}, ${where}`}
        onPress={() => onOpen(item)}
      >
        <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 12 }}>
          {[item.mention?.who, item.title || t('New conversation'), where]
            .filter(Boolean)
            .join(' · ')}
        </Text>
        <View style={{ flexDirection: 'row', gap: 10, alignItems: 'flex-start', marginTop: 4 }}>
          {item.mention ? (
            <WhoFace
              who={whoOf(
                {
                  name: item.mention.who,
                  role: item.from?.role ?? item.mention.who,
                  agent: item.from?.agent,
                  provider: item.from?.provider,
                },
                item.from?.lead ? item.from.role || 'lead' : null,
              )}
            />
          ) : null}
          <Text
            numberOfLines={3}
            style={{ color: colors.text, fontWeight: '700', fontSize: 18, flex: 1 }}
          >
            {p ? p.title : item.mention?.text}
          </Text>
        </View>
        {p?.detail ? (
          <Text
            numberOfLines={2}
            style={{
              fontFamily: mono,
              fontSize: 12,
              color: colors.text,
              backgroundColor: colors.code,
              padding: 8,
              borderRadius: 8,
              marginTop: 8,
              overflow: 'hidden',
            }}
          >
            {p.detail}
          </Text>
        ) : null}
      </Pressable>
      {failed ? (
        <Text style={{ color: colors.dangerInk, fontSize: 12 }}>
          {t('Could not send the answer.')}
        </Text>
      ) : null}
      <View style={{ flexDirection: 'row', gap: 8, flexWrap: 'wrap' }}>
        {p ? (
          <>
            {options.map(([id, name], i) => (
              <Button
                key={id}
                testID={`need-option-${i}`}
                label={name}
                primary={i === 0}
                disabled={busy}
                onPress={() => answer(id)}
              />
            ))}
            {hasNo ? null : (
              <Button label={t('Deny')} disabled={busy} onPress={() => answer(null)} />
            )}
          </>
        ) : (
          <>
            <Button testID="need-reply" label={t('Reply')} primary onPress={() => onReply(item)} />
            <Button testID="need-dismiss" label={t('Dismiss')} onPress={() => onDismiss(item)} />
          </>
        )}
      </View>
    </Animated.View>
  );
}
