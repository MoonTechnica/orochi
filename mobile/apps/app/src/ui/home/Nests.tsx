import React, { useState } from 'react';
import { Pressable, ScrollView, Text, View } from 'react-native';
import type { DesktopCard } from '@orochi/core';
import { usePrefs } from '../../state/prefs';
import { useStateColor, useStateInk, useStateLabel } from '../Machines';
import { radius } from '../theme';

const TILE = 150;

/**
 * Six ten-minute bars of conversations *touched* (their `updated_at`): the desktop's RPCs have
 * no history of agent work, only what is live now, so this is all the wire can say honestly.
 */
function Bars({ values, color }: { values: number[]; color: string }) {
  const { colors, t } = usePrefs();
  const top = Math.max(1, ...values);
  const total = values.reduce((a, b) => a + b, 0);
  return (
    <View
      accessible
      accessibilityLabel={t('{n} conversations touched in the last hour').replace(
        '{n}',
        String(total),
      )}
      style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 3, height: 22 }}
    >
      {values.map((v, i) => (
        <View
          key={i}
          style={{
            width: 6,
            height: 3 + (19 * v) / top,
            borderRadius: 2,
            backgroundColor: v ? color : colors.line,
          }}
        />
      ))}
    </View>
  );
}

/** Desktops as a strip of nests; a nest that cannot be used says why when tapped. */
export function Nests({
  desktops,
  onOpen,
  onRetry,
  onRepair,
  onAdd,
}: {
  desktops: DesktopCard[];
  onOpen: (d: DesktopCard) => void;
  onRetry: () => void;
  onRepair: (d: DesktopCard) => void;
  onAdd: () => void;
}) {
  const { colors, t } = usePrefs();
  const color = useStateColor();
  const ink = useStateInk();
  const label = useStateLabel();
  const [picked, setPicked] = useState<string | null>(null);
  const pickedCard = desktops.find((d) => d.key === picked && d.state !== 'online');

  return (
    <View>
      <ScrollView
        horizontal
        showsHorizontalScrollIndicator={false}
        contentContainerStyle={{ gap: 10, paddingHorizontal: 18 }}
      >
        {desktops.map((d) => {
          const bad = d.state === 'offline' || d.state === 'repair';
          const c = color(d.state);
          return (
            <Pressable
              key={d.key}
              testID={`nest-${d.key}`}
              accessibilityRole="button"
              accessibilityLabel={`${d.name}, ${label(d.state)}`}
              onPress={() => (bad ? setPicked(picked === d.key ? null : d.key) : onOpen(d))}
              style={{
                width: TILE,
                height: 124,
                padding: 12,
                justifyContent: 'space-between',
                borderRadius: radius.tile,
                backgroundColor: colors.card,
                borderWidth: bad ? 1.5 : 1,
                borderColor: bad ? c : colors.line,
              }}
            >
              <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
                <View
                  style={{
                    width: 9,
                    height: 9,
                    borderRadius: 5,
                    backgroundColor: d.state === 'connecting' ? 'transparent' : c,
                    borderWidth: d.state === 'connecting' ? 2 : 0,
                    borderColor: c,
                  }}
                />
                <Text
                  numberOfLines={1}
                  style={{ color: ink(d.state), fontSize: 11, fontWeight: '700', flex: 1 }}
                >
                  {label(d.state, true)}
                </Text>
              </View>
              <Text
                numberOfLines={2}
                style={{
                  color: colors.text,
                  fontWeight: '700',
                  fontSize: 15,
                  lineHeight: 18,
                }}
              >
                {d.name}
              </Text>
              <View
                style={{
                  flexDirection: 'row',
                  alignItems: 'flex-end',
                  justifyContent: 'space-between',
                }}
              >
                <Bars values={d.bars} color={c} />
                <Text style={{ color: colors.sub, fontSize: 12 }}>
                  {d.state === 'online' ? `${d.active} ${t('running')}` : '—'}
                </Text>
              </View>
              <Text numberOfLines={1} style={{ color: colors.sub, fontSize: 10 }}>
                {t('touched in the last hour')}
              </Text>
            </Pressable>
          );
        })}
        <Pressable
          testID="nest-add"
          accessibilityRole="button"
          accessibilityLabel={t('Pair a desktop')}
          onPress={onAdd}
          style={{
            width: 90,
            height: 124,
            borderRadius: radius.tile,
            borderWidth: 1.5,
            borderStyle: 'dashed',
            borderColor: colors.muted,
            alignItems: 'center',
            justifyContent: 'center',
            gap: 4,
          }}
        >
          <Text style={{ color: colors.text, fontSize: 28, lineHeight: 30 }}>+</Text>
          <Text style={{ color: colors.sub, fontSize: 12 }}>{t('Add')}</Text>
        </Pressable>
      </ScrollView>
      {pickedCard ? (
        <View
          testID="nest-reason"
          style={{
            marginHorizontal: 18,
            marginTop: 10,
            padding: 12,
            gap: 8,
            borderLeftWidth: 3,
            borderLeftColor: color(pickedCard.state),
            backgroundColor: colors.card,
            borderRadius: 8,
          }}
        >
          <Text style={{ color: colors.text, fontSize: 14 }}>
            {pickedCard.state === 'repair'
              ? t('Pairing was revoked on the desktop. Pair again.')
              : t('Cannot reach this desktop. Check that Tailscale is connected.')}
          </Text>
          {pickedCard.state === 'offline' && pickedCard.error ? (
            <Text numberOfLines={2} style={{ color: colors.sub, fontSize: 12 }}>
              {pickedCard.error}
            </Text>
          ) : null}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={pickedCard.state === 'repair' ? t('Pair again') : t('Retry')}
            onPress={() => (pickedCard.state === 'repair' ? onRepair(pickedCard) : onRetry())}
            style={{
              alignSelf: 'flex-start',
              minHeight: 40,
              paddingHorizontal: 16,
              justifyContent: 'center',
              borderRadius: radius.pill,
              backgroundColor: pickedCard.state === 'repair' ? colors.primary : 'transparent',
              borderWidth: pickedCard.state === 'repair' ? 0 : 1,
              borderColor: colors.line,
            }}
          >
            <Text
              style={{
                fontWeight: '700',
                color: pickedCard.state === 'repair' ? colors.onPrimary : colors.text,
              }}
            >
              {pickedCard.state === 'repair' ? t('Pair again') : t('Retry')}
            </Text>
          </Pressable>
        </View>
      ) : null}
    </View>
  );
}
