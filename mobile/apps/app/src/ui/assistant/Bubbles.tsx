import React, { useState } from 'react';
import { Linking, Pressable, Text, View } from 'react-native';
import { folded, proposalWords, scheduleWords, type Source } from '@orochi/core';
import type { RoutineRow } from '@orochi/protocol';
import { usePrefs } from '../../state/prefs';
import { Icon } from '../Icon';
import { Markdown, type MdContext } from '../Markdown';
import { mono, radius } from '../theme';

/**
 * The pieces of a resident's conversation as a messenger draws them. The person's bubbles are
 * the app's main-action inversion, the assistant's the raised grey, so nothing here carries a
 * colour of its own.
 */
const BUBBLE = {
  maxWidth: '84%' as const,
  borderRadius: 20,
  paddingHorizontal: 16,
  paddingVertical: 12,
};

export function MeBubble({ text }: { text: string }) {
  const { colors } = usePrefs();
  return (
    <View style={{ paddingHorizontal: 12, paddingVertical: 3, alignItems: 'flex-end' }}>
      <View style={{ ...BUBBLE, backgroundColor: colors.primary }}>
        <Text style={{ color: colors.onPrimary, fontSize: 16, lineHeight: 23 }} selectable>
          {text}
        </Text>
      </View>
    </View>
  );
}

export function AgentBubble({
  text,
  sources,
  ctx,
  onSource,
  onLongPress,
}: {
  text: string;
  sources: Source[];
  ctx: MdContext;
  onSource: (s: Source) => void;
  onLongPress?: () => void;
}) {
  const { colors, t } = usePrefs();
  const [open, setOpen] = useState(false);
  const { head, more } = folded(text);
  return (
    <View style={{ paddingHorizontal: 12, paddingVertical: 3, alignItems: 'flex-start' }}>
      <Pressable
        onLongPress={onLongPress}
        style={{ ...BUBBLE, backgroundColor: colors.raised, gap: 8 }}
      >
        <Markdown text={open || !more ? text : head} ctx={ctx} size={16} />
        {more ? (
          <Pressable
            accessibilityRole="button"
            onPress={() => setOpen(!open)}
            hitSlop={8}
            style={{ minHeight: 32, justifyContent: 'center' }}
          >
            <Text style={{ color: colors.sub, fontSize: 14, fontWeight: '600' }}>
              {open ? t('Show less') : t('Show more')}
            </Text>
          </Pressable>
        ) : null}
        {sources.length ? (
          <View style={{ gap: 4, borderTopWidth: 1, borderTopColor: colors.line, paddingTop: 8 }}>
            {sources.map((s) => (
              <Pressable
                key={s.target}
                accessibilityRole="link"
                accessibilityLabel={`${t('Source')}: ${s.label}`}
                onPress={() => onSource(s)}
                style={{ flexDirection: 'row', alignItems: 'center', gap: 6, minHeight: 32 }}
              >
                <Icon
                  name={s.kind === 'web' ? 'globe-outline' : 'document-text-outline'}
                  size={16}
                  color={colors.sub}
                />
                <Text
                  numberOfLines={1}
                  style={{
                    color: colors.text,
                    fontSize: 14,
                    textDecorationLine: 'underline',
                    flex: 1,
                  }}
                >
                  {s.label}
                </Text>
              </Pressable>
            ))}
          </View>
        ) : null}
      </Pressable>
    </View>
  );
}

export function Typing() {
  const { colors, t } = usePrefs();
  return (
    <View style={{ paddingHorizontal: 12, paddingVertical: 3, alignItems: 'flex-start' }}>
      <View
        accessibilityLabel={t('Working…')}
        style={{ ...BUBBLE, backgroundColor: colors.raised, paddingVertical: 10 }}
      >
        <Text style={{ color: colors.sub, fontSize: 18, letterSpacing: 2 }}>•••</Text>
      </View>
    </View>
  );
}

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

/** A page it read opens in the browser; a file opens in the app's own viewer. */
export function openSource(s: Source, openFile: (path: string) => void) {
  if (s.kind === 'web') Linking.openURL(s.target).catch(() => {});
  else openFile(s.target);
}
