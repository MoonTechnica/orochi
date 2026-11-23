import React, { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { proposalWords, scheduleWords } from '@orochi/core';
import type { RoutineRow } from '@orochi/protocol';
import { usePrefs } from '../../state/prefs';
import { Icon } from '../Icon';
import { mono, radius } from '../theme';

/**
 * What the assistant proposed: when it would run (or the command it would run unattended,
 * shown whole), what it would do, and the person's answer. Nothing runs before Allow.
 */
export function ProposalCard({
  routine,
  onAnswer,
}: {
  routine: RoutineRow;
  onAnswer: (allow: boolean) => Promise<void>;
}) {
  const { colors, t, lang } = usePrefs();
  const [busy, setBusy] = useState(false);
  const { when, command } = proposalWords(routine);
  const answer = async (allow: boolean) => {
    setBusy(true);
    try {
      await onAnswer(allow);
    } finally {
      setBusy(false);
    }
  };
  const button = (label: string, allow: boolean) => (
    <Pressable
      accessibilityRole="button"
      disabled={busy}
      onPress={() => answer(allow)}
      style={{
        flex: 1,
        minHeight: 44,
        borderRadius: radius.pill,
        alignItems: 'center',
        justifyContent: 'center',
        backgroundColor: allow ? colors.primary : 'transparent',
        borderWidth: allow ? 0 : 1,
        borderColor: colors.line,
        opacity: busy ? 0.5 : 1,
      }}
    >
      <Text style={{ color: allow ? colors.onPrimary : colors.text, fontWeight: '700' }}>
        {label}
      </Text>
    </Pressable>
  );
  return (
    <View
      testID="proposal-card"
      style={{
        marginHorizontal: 12,
        marginVertical: 6,
        padding: 16,
        gap: 8,
        borderRadius: 16,
        backgroundColor: colors.card,
        borderWidth: 2,
        borderColor: colors.attention,
      }}
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 6 }}>
        <Icon name="alarm-outline" size={16} color={colors.text} />
        <Text style={{ color: colors.text, fontWeight: '700', fontSize: 15 }}>
          {command ? t('Proposed watch') : t('Proposed routine')}
        </Text>
      </View>
      <Text style={{ color: colors.text, fontSize: 15 }}>{scheduleWords(when, lang)}</Text>
      {routine.member ? (
        <Text style={{ color: colors.text, fontSize: 12, fontWeight: '700' }}>
          {t('Run by {name}').replace('{name}', routine.member)}
        </Text>
      ) : null}
      {command ? (
        <View style={{ backgroundColor: colors.code, borderRadius: 8, padding: 10 }}>
          <Text selectable style={{ color: colors.text, fontFamily: mono, fontSize: 13 }}>
            {command}
          </Text>
        </View>
      ) : null}
      <Text style={{ color: colors.sub, fontSize: 14 }}>{routine.instruction}</Text>
      <Text style={{ color: colors.sub, fontSize: 12 }}>
        {command
          ? t('It runs this command here with nobody present, reading only.')
          : t('It runs reading only, with nobody present.')}
      </Text>
      <View style={{ flexDirection: 'row', gap: 8, marginTop: 4 }}>
        {button(t('Decline'), false)}
        {button(t('Allow'), true)}
      </View>
    </View>
  );
}
