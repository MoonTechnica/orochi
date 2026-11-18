import React, { memo, useMemo } from 'react';
import { Pressable, Text, View } from 'react-native';
import { ago, footerLabel, footerOf, type ThreadSummary, type Voice } from '@orochi/core';
import { usePrefs } from '../state/prefs';
import { Icon } from './Icon';
import { usePeople } from './people/People';
import { INDENT, WhoFace } from './people/WhoFace';

/** Up to three distinct repliers, 20 px, overlapping; the member alone while nobody has replied. */
function Stack({ s, size }: { s: ThreadSummary; size: number }) {
  const people = usePeople();
  const whos = useMemo(
    () =>
      s.participants.length
        ? s.participants.map((v: Voice) => people.voice(v))
        : [people.who(s.name)],
    [s.participants, s.name, people],
  );
  return (
    <View style={{ flexDirection: 'row', alignItems: 'center' }} importantForAccessibility="no">
      {whos.map((w, i) => (
        <View key={`${i}${w.name}`} style={{ marginLeft: i ? -size * 0.3 : 0 }}>
          <WhoFace who={w} size={size} badge={false} />
        </View>
      ))}
    </View>
  );
}

const Dot = ({ size = 8 }: { size?: number }) => {
  const { colors } = usePrefs();
  return (
    <View
      style={{ width: size, height: size, borderRadius: size / 2, backgroundColor: colors.text }}
    />
  );
};

/** "Needs you": the inverted chip, the one thing in the footer that is not grey or plain text. */
export function NeedsChip({ label }: { label: string }) {
  const { colors } = usePrefs();
  return (
    <View
      style={{
        borderRadius: 8,
        paddingHorizontal: 7,
        paddingVertical: 2,
        backgroundColor: colors.attention,
      }}
    >
      <Text style={{ color: colors.onPrimary, fontSize: 11, fontWeight: '700' }}>{label}</Text>
    </View>
  );
}

/**
 * The line under a message that has a thread (Slack's "thread footer"): replier faces, "N replies",
 * "Last reply 4m ago", a chevron; the whole row opens the thread. Unread is a dot and bold text.
 */
export const ThreadFooter = memo(function ThreadFooter({
  s,
  seenAt,
  onPress,
  now,
}: {
  s: ThreadSummary;
  seenAt: number | null | undefined;
  onPress: () => void;
  now: number;
}) {
  const { colors, t, lang } = usePrefs();
  const f = footerOf(s, seenAt);
  const label = footerLabel(s, f, now, lang, t);
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      testID={`thread-footer-${s.name}`}
      style={{
        marginLeft: INDENT - 6,
        marginRight: 12,
        marginVertical: 2,
        paddingVertical: 6,
        paddingHorizontal: 6,
        minHeight: 44,
        borderRadius: 8,
        justifyContent: 'center',
        gap: 2,
      }}
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        {f.unread ? <Dot /> : null}
        <Stack s={s} size={20} />
        <Text
          style={{
            color: colors.text,
            fontSize: 13,
            fontWeight: f.unread ? '800' : '700',
            flexShrink: 0,
          }}
        >
          {s.replies === 0
            ? t('View thread')
            : lang === 'ja'
              ? `${s.replies}${t('replies')}`
              : `${s.replies} ${s.replies === 1 ? 'reply' : 'replies'}`}
        </Text>
        {s.lastAt ? (
          <Text style={{ color: colors.sub, fontSize: 12, flexShrink: 1 }} numberOfLines={1}>
            {t('Last reply')} {ago(s.lastAt, now, lang)}
          </Text>
        ) : null}
        <View style={{ flex: 1 }} />
        <Icon name="chevron-forward" size={16} color={colors.sub} />
      </View>
      {f.needsYou || f.caption ? (
        <View
          style={{
            flexDirection: 'row',
            alignItems: 'center',
            gap: 8,
            marginLeft: f.unread ? 16 : 0,
          }}
        >
          {f.needsYou ? <NeedsChip label={t('Needs you')} /> : null}
          {f.caption ? (
            <Text style={{ color: colors.sub, fontSize: 12, flexShrink: 1 }} numberOfLines={1}>
              {f.caption}
            </Text>
          ) : null}
        </View>
      ) : null}
    </Pressable>
  );
});

/** A single compact pill under the channel header: "Threads 3" with a dot when something is unread. */
export function ThreadsPill({
  count,
  unread,
  onPress,
}: {
  count: number;
  unread: boolean;
  onPress: () => void;
}) {
  const { colors, t } = usePrefs();
  return (
    <View
      style={{
        flexDirection: 'row',
        paddingHorizontal: 12,
        paddingVertical: 6,
        backgroundColor: colors.card,
        borderBottomWidth: 1,
        borderBottomColor: colors.line,
      }}
    >
      <Pressable
        onPress={onPress}
        accessibilityRole="button"
        accessibilityLabel={`${t('Threads')} (${count})${unread ? `, ${t('unread')}` : ''}`}
        testID="threads-pill"
        style={{
          minHeight: 36,
          flexDirection: 'row',
          alignItems: 'center',
          gap: 6,
          paddingHorizontal: 12,
          borderRadius: 18,
          borderWidth: 1,
          borderColor: colors.line,
        }}
      >
        <Icon name="chatbubbles-outline" size={16} color={colors.text} />
        <Text style={{ color: colors.text, fontSize: 13, fontWeight: '700' }}>{t('Threads')}</Text>
        <Text style={{ color: colors.sub, fontSize: 13 }}>{count}</Text>
        {unread ? <Dot /> : null}
      </Pressable>
    </View>
  );
}

const firstLine = (text: string) =>
  (text.split('\n').find((l) => l.trim()) ?? '').replace(/[*_`#>]/g, '').trim();

/** One thread in the Threads list. */
export const ThreadCard = memo(function ThreadCard({
  s,
  seenAt,
  now,
  onPress,
}: {
  s: ThreadSummary;
  seenAt: number | null | undefined;
  now: number;
  onPress: () => void;
}) {
  const { colors, t, lang } = usePrefs();
  const f = footerOf(s, seenAt);
  const hot = f.unread || f.needsYou;
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={footerLabel(s, f, now, lang, t)}
      testID={`thread-card-${s.name}`}
      style={{
        marginHorizontal: 12,
        marginVertical: 5,
        padding: 12,
        gap: 6,
        borderRadius: 12,
        borderWidth: f.needsYou ? 2 : 1,
        borderColor: f.needsYou ? colors.attention : colors.line,
        backgroundColor: colors.card,
      }}
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        {f.unread ? <Dot /> : null}
        <Stack s={s} size={24} />
        <Text
          style={{
            color: colors.text,
            fontSize: 15,
            fontWeight: hot ? '800' : '700',
            flexShrink: 1,
          }}
          numberOfLines={1}
        >
          {s.name}
        </Text>
        <View style={{ flex: 1 }} />
        {f.needsYou ? <NeedsChip label={t('Needs you')} /> : null}
        {s.lastAt ? (
          <Text style={{ color: colors.sub, fontSize: 12 }}>{ago(s.lastAt, now, lang)}</Text>
        ) : null}
      </View>
      {s.parent ? (
        <Text style={{ color: colors.text, fontSize: 14 }} numberOfLines={1}>
          {firstLine(s.parent.text)}
        </Text>
      ) : null}
      {s.replies > 0 ? (
        <Text style={{ color: colors.sub, fontSize: 13 }} numberOfLines={2}>
          <Text style={{ fontWeight: hot ? '700' : '600', color: hot ? colors.text : colors.sub }}>
            {s.lastWho}
          </Text>
          {': '}
          {firstLine(s.lastText)}
        </Text>
      ) : null}
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8 }}>
        <Text style={{ color: colors.text, fontSize: 12, fontWeight: '700' }}>
          {lang === 'ja'
            ? `${s.replies}${t('replies')}`
            : `${s.replies} ${s.replies === 1 ? 'reply' : 'replies'}`}
        </Text>
        {f.caption ? (
          <Text style={{ color: colors.sub, fontSize: 12, flexShrink: 1 }} numberOfLines={1}>
            {f.caption}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );
});
