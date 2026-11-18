import React, { useMemo, useState } from 'react';
import { ActivityIndicator, Pressable, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import {
  dayLabel,
  hhmm,
  plural,
  repliedToThread,
  span,
  type Entry,
  type ThreadLine,
  type Voice,
  AT,
  isAnonymous,
} from '@orochi/core';
import type { ItemRow, Member, Prompt } from '@orochi/protocol';
import { usePrefs } from '../state/prefs';
import { useSession } from '../state/session';
import { LiveFace } from './people/LiveFace';
import { usePeople } from './people/People';
import { FACE, INDENT, WhoFace } from './people/WhoFace';
import { Icon } from './Icon';
import { Markdown, type MdContext } from './Markdown';
import { MediaCard } from './MediaThumb';
import { mono, toneInk } from './theme';

/** Text-safe colour for a tone (the plain tokens are for fills and marks). */
export const toneColor = toneInk;

export function PostView({
  voice,
  run,
  ctx,
  report,
  inThread,
  onLongPress,
  onOpenThread,
}: {
  voice: Voice;
  run: boolean;
  ctx: MdContext;
  report?: boolean;
  /** Drawn inside a member's thread: a broadcast reply says it was also sent to the channel. */
  inThread?: boolean;
  onLongPress?: (voice: Voice) => void;
  /** In the channel: opens the thread a broadcast reply belongs to. */
  onOpenThread?: (member: string) => void;
}) {
  const { colors, t } = usePrefs();
  const who = usePeople().voice(voice);
  // A tap shows the time on every message (a run of one speaker has it only here).
  const [timed, setTimed] = useState(false);
  const replied = !inThread ? repliedToThread(voice) : null;
  const alsoSent = inThread && voice.broadcast;
  return (
    <Pressable
      onPress={() => setTimed(!timed)}
      onLongPress={onLongPress ? () => onLongPress(voice) : undefined}
      delayLongPress={350}
      accessibilityLabel={`${replied ? `${t('Replied to a thread')}, ` : ''}${voice.who}: ${voice.text}`}
      style={{
        flexDirection: 'row',
        gap: 10,
        paddingHorizontal: 12,
        paddingTop: run ? 2 : 10,
        paddingBottom: 2,
        borderLeftWidth: voice.mentionsYou ? 3 : 0,
        borderLeftColor: colors.attention,
        backgroundColor: voice.mentionsYou ? colors.code : voice.mine ? colors.mine : undefined,
      }}
    >
      <View style={{ width: FACE }}>
        {run ? (
          timed && voice.at ? (
            <Text style={{ color: colors.sub, fontSize: 10, paddingTop: 3 }} numberOfLines={1}>
              {hhmm(voice.at)}
            </Text>
          ) : null
        ) : (
          <WhoFace who={who} />
        )}
      </View>
      <View style={{ flex: 1, minWidth: 0 }}>
        {replied ? (
          <Pressable
            onPress={onOpenThread ? () => onOpenThread(replied) : undefined}
            accessibilityRole="link"
            accessibilityLabel={`${t('Replied to a thread')}: ${replied}`}
            hitSlop={6}
            style={{ flexDirection: 'row', alignItems: 'center', gap: 4, marginBottom: 2 }}
          >
            <Icon name="return-down-forward" size={13} color={colors.sub} />
            <Text style={{ color: colors.sub, fontSize: 12, fontWeight: '600' }}>
              {t('Replied to a thread')}
            </Text>
          </Pressable>
        ) : null}
        {run ? null : (
          <View
            style={{
              flexDirection: 'row',
              alignItems: 'baseline',
              flexWrap: 'wrap',
              gap: 6,
              marginBottom: 2,
            }}
          >
            <Text style={{ color: colors.text, fontWeight: '700', fontSize: 15 }}>{voice.who}</Text>
            {voice.to ? (
              <Text style={{ color: colors.sub, fontSize: 12 }}>
                → {voice.to === 'everyone' ? t('everyone') : voice.to}
              </Text>
            ) : null}
            {voice.model ? (
              <Text style={{ color: colors.sub, fontSize: 12 }}>{voice.model}</Text>
            ) : null}
            {voice.at ? (
              <Text style={{ color: colors.sub, fontSize: 12 }}>{hhmm(voice.at)}</Text>
            ) : null}
          </View>
        )}
        <Markdown text={voice.text} ctx={ctx} />
        {alsoSent ? (
          <Text style={{ color: colors.sub, fontSize: 12, marginTop: 2 }}>
            {t('Also sent to the channel')}
          </Text>
        ) : null}
        {report ? (
          <Text style={{ color: colors.text, fontSize: 12, fontWeight: '700', marginTop: 2 }}>
            {t('report')}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );
}

function Fold({
  title,
  children,
  quiet,
}: {
  title: string;
  children: React.ReactNode;
  quiet?: boolean;
}) {
  const { colors } = usePrefs();
  const [open, setOpen] = useState(false);
  return (
    <View
      style={{ marginLeft: INDENT, marginRight: 12, marginVertical: 2, opacity: quiet ? 0.7 : 1 }}
    >
      <Pressable
        onPress={() => setOpen(!open)}
        style={{ minHeight: 32, flexDirection: 'row', alignItems: 'center', gap: 4 }}
        accessibilityRole="button"
      >
        <Icon name={open ? 'chevron-down' : 'chevron-forward'} size={14} color={colors.sub} />
        <Text style={{ color: colors.sub, fontSize: quiet ? 12 : 13 }}>{title}</Text>
      </Pressable>
      {open ? <View style={{ gap: 3, paddingLeft: 8 }}>{children}</View> : null}
    </View>
  );
}

export function WorkView({
  items,
  tools,
  thoughts,
  quiet,
}: {
  items: ItemRow[];
  tools: number;
  thoughts: number;
  quiet?: boolean;
}) {
  const { colors, t, lang } = usePrefs();
  const parts: string[] = [];
  if (thoughts) parts.push(t('Thought'));
  if (tools) parts.push(plural(tools, 'tool call', 'tool calls', lang));
  if (!parts.length) parts.push(plural(items.length, 'tool call', 'tool calls', lang));
  return (
    <Fold title={parts.join(', ')} quiet={quiet}>
      {items.map((item) => {
        const d = item.data || {};
        const label = (
          item.kind === 'thought' ? 'thinking' : `${d.title || item.kind} ${d.detail || ''}`
        ).trim();
        return (
          <View key={item.seq}>
            <Text style={{ color: colors.sub, fontFamily: mono, fontSize: 12 }} numberOfLines={3}>
              {label}
            </Text>
            {item.text ? (
              <Text
                style={{ color: colors.sub, fontFamily: mono, fontSize: 11 }}
                numberOfLines={12}
              >
                {item.text}
              </Text>
            ) : null}
          </View>
        );
      })}
    </Fold>
  );
}

export function AsideView({ notes, label }: { notes: ItemRow[]; label: string | null }) {
  const { colors, t, lang } = usePrefs();
  return (
    <Fold title={label ? t(label) : plural(notes.length, 'routing note', 'routing notes', lang)}>
      {notes.map((n) => (
        <Text key={n.seq} style={{ color: colors.sub, fontSize: 12 }}>
          {n.text}
        </Text>
      ))}
    </Fold>
  );
}

export function ChecksView({ entry }: { entry: Extract<Entry, { type: 'checks' }> }) {
  const { colors, t } = usePrefs();
  const verdict = (
    { success: 'verified', failure: 'failed', cancelled: 'stopped' } as Record<string, string>
  )[entry.outcome ?? ''];
  if (!verdict && !entry.checks.length) return null;
  return (
    <View style={{ marginLeft: INDENT, marginRight: 12, marginVertical: 3, gap: 2 }}>
      {verdict ? (
        <Text
          style={{
            color: entry.outcome === 'success' ? colors.text : colors.dangerInk,
            fontWeight: '700',
            fontSize: 13,
          }}
        >
          {t(verdict)}
        </Text>
      ) : null}
      {entry.checks.map((c, i) => (
        <View key={i}>
          <Text style={{ color: c.passed ? colors.text : colors.dangerInk, fontSize: 13 }}>
            {c.passed ? '✓' : '✗'} {c.name}
          </Text>
          {c.tail ? (
            <Text style={{ color: colors.sub, fontFamily: mono, fontSize: 11 }}>{c.tail}</Text>
          ) : null}
        </View>
      ))}
    </View>
  );
}

export function RouteChip({ text, resumed }: { text: string; resumed: boolean }) {
  const { colors } = usePrefs();
  return (
    <View style={{ marginLeft: INDENT, marginVertical: 3, flexDirection: 'row' }}>
      <View
        style={{
          borderRadius: 10,
          paddingHorizontal: 8,
          paddingVertical: 2,
          backgroundColor: colors.code,
        }}
      >
        <Text style={{ color: colors.sub, fontSize: 11 }}>
          {text}
          {resumed ? ' · ↻' : ''}
        </Text>
      </View>
    </View>
  );
}

export function ReportsLine({
  entry,
  onMember,
}: {
  entry: Extract<Entry, { type: 'reports' }>;
  onMember: (name: string) => void;
}) {
  const { colors, t } = usePrefs();
  const people = usePeople();
  const head =
    entry.origin === 'merge'
      ? t('from the merge')
      : entry.origin === 'mail'
        ? t('from a message')
        : t('reports from');
  return (
    <View
      style={{
        marginLeft: INDENT,
        marginVertical: 4,
        flexDirection: 'row',
        flexWrap: 'wrap',
        alignItems: 'center',
        gap: 4,
      }}
    >
      {entry.names.length > 0 ? (
        <View style={{ flexDirection: 'row', alignItems: 'center', marginRight: 2 }}>
          {entry.names.slice(0, 4).map((n, i) => (
            <View key={n} style={{ marginLeft: i ? -6 : 0 }}>
              <WhoFace who={people.who(n)} size={18} badge={false} />
            </View>
          ))}
        </View>
      ) : null}
      <Text style={{ color: colors.sub, fontSize: 13 }}>{head}</Text>
      {entry.names.length === 0 && entry.origin === 'background' ? (
        <Text style={{ color: colors.sub, fontSize: 13 }}>{t('the helpers')}</Text>
      ) : null}
      {entry.names.map((n) => (
        <Pressable key={n} onPress={() => onMember(n)} hitSlop={8}>
          <Text style={{ color: colors.text, fontSize: 13, fontWeight: '600' }}>{n}</Text>
        </Pressable>
      ))}
    </View>
  );
}

export function FootView({
  entry,
  onFiles,
}: {
  entry: Extract<Entry, { type: 'foot' }>;
  onFiles: () => void;
}) {
  const { colors, t, lang } = usePrefs();
  return (
    <View
      style={{
        marginLeft: INDENT,
        marginVertical: 3,
        flexDirection: 'row',
        gap: 10,
        flexWrap: 'wrap',
      }}
    >
      <Text style={{ color: colors.sub, fontSize: 12 }}>
        {t('Worked for')} {span(entry.ms)} · {hhmm(entry.endedAt)}
      </Text>
      {entry.files ? (
        <Pressable onPress={onFiles} hitSlop={10}>
          <Text style={{ color: colors.text, fontSize: 12 }}>
            {lang === 'ja'
              ? `${entry.files}${t('files changed')}`
              : `${entry.files} ${entry.files === 1 ? 'file changed' : 'files changed'}`}
          </Text>
        </Pressable>
      ) : null}
    </View>
  );
}

export function LiveView({ entry }: { entry: Extract<Entry, { type: 'live' }> }) {
  const { colors, t } = usePrefs();
  const people = usePeople();
  return (
    <View
      style={{
        marginLeft: INDENT,
        marginVertical: 6,
        flexDirection: 'row',
        alignItems: 'center',
        gap: 8,
      }}
    >
      <ActivityIndicator size="small" color={colors.live} />
      {entry.starting ? (
        <Text style={{ color: colors.sub }}>{t('Starting the agent…')}</Text>
      ) : (
        entry.seats.map((s, i) => (
          <View
            key={i}
            style={{ flexDirection: 'row', alignItems: 'center', gap: 6, flexShrink: 1 }}
          >
            <WhoFace who={people.who(s.role)} size={20} badge={false} state="working" />
            <Text style={{ color: colors.sub, flexShrink: 1 }} numberOfLines={1}>
              <Text style={{ fontWeight: '700', color: colors.text }}>{s.role}</Text>{' '}
              {AT[s.what] ? t(AT[s.what]!) : s.what}
            </Text>
          </View>
        ))
      )}
    </View>
  );
}

export function AskCard({
  prompt,
  onAnswer,
}: {
  prompt: Prompt;
  onAnswer: (id: string, option: string | null) => void;
}) {
  const { colors, t } = usePrefs();
  const people = usePeople();
  const asker = people.who(prompt.lead === false && prompt.role ? prompt.role : 'lead');
  const [busy, setBusy] = useState(false);
  const go = async (option: string | null) => {
    setBusy(true);
    try {
      await onAnswer(prompt.id, option);
    } finally {
      setBusy(false);
    }
  };
  return (
    <View
      style={{
        margin: 12,
        padding: 12,
        borderRadius: 12,
        borderWidth: 2,
        borderColor: colors.attention,
        backgroundColor: colors.card,
        gap: 8,
      }}
    >
      <View style={{ flexDirection: 'row', alignItems: 'center', gap: 10 }}>
        {isAnonymous(asker) ? null : <WhoFace who={asker} size={28} state="asking" />}
        <Text style={{ color: colors.text, fontWeight: '700', fontSize: 15, flex: 1 }}>
          {prompt.title}
          {prompt.lead === false && prompt.role ? (
            <Text style={{ color: colors.sub, fontWeight: '400' }}>
              {' '}
              · {t('from')} {prompt.role}
            </Text>
          ) : null}
        </Text>
      </View>
      {prompt.detail ? (
        <Text
          selectable
          style={{
            fontFamily: mono,
            fontSize: 12,
            color: colors.text,
            backgroundColor: colors.code,
            padding: 8,
            borderRadius: 6,
          }}
        >
          {prompt.detail}
        </Text>
      ) : null}
      {prompt.read_only && prompt.role ? (
        <Text style={{ color: colors.sub, fontSize: 12 }}>
          {prompt.role} {t('is read only · this command runs in your working tree')}
        </Text>
      ) : null}
      <View style={{ flexDirection: 'row', flexWrap: 'wrap', gap: 8 }}>
        {prompt.options.map(([id, name]) => (
          <Pressable
            key={id}
            disabled={busy}
            onPress={() => go(id)}
            accessibilityRole="button"
            style={{
              minHeight: 44,
              paddingHorizontal: 16,
              borderRadius: 10,
              backgroundColor: colors.primary,
              justifyContent: 'center',
              opacity: busy ? 0.5 : 1,
            }}
          >
            <Text style={{ color: colors.onPrimary, fontWeight: '700' }}>{name}</Text>
          </Pressable>
        ))}
        <Pressable
          disabled={busy}
          onPress={() => go(null)}
          accessibilityRole="button"
          style={{
            minHeight: 44,
            paddingHorizontal: 16,
            borderRadius: 10,
            borderWidth: 1,
            borderColor: colors.line,
            justifyContent: 'center',
          }}
        >
          <Text style={{ color: colors.text, fontWeight: '600' }}>{t('No')}</Text>
        </Pressable>
      </View>
    </View>
  );
}

/** Slack's day divider: a hairline either side of "Today", "Yesterday" or the date. */
export function DayRow({ at }: { at: number }) {
  const { colors, t, lang } = usePrefs();
  const label = dayLabel(at);
  const text =
    label.kind === 'today'
      ? t('Today')
      : label.kind === 'yesterday'
        ? t('Yesterday')
        : new Date(label.at).toLocaleDateString(lang === 'ja' ? 'ja-JP' : 'en-US', {
            weekday: 'short',
            month: 'short',
            day: 'numeric',
          });
  return (
    <View
      accessibilityRole="header"
      style={{
        flexDirection: 'row',
        alignItems: 'center',
        gap: 10,
        marginVertical: 10,
        paddingHorizontal: 12,
      }}
    >
      <View style={{ flex: 1, height: 1, backgroundColor: colors.line }} />
      <Text style={{ color: colors.sub, fontSize: 12, fontWeight: '700' }}>{text}</Text>
      <View style={{ flex: 1, height: 1, backgroundColor: colors.line }} />
    </View>
  );
}

export function DividerView({ at, where }: { at: number; where: string | null }) {
  const { colors, t } = usePrefs();
  return (
    <Text style={{ textAlign: 'center', color: colors.sub, fontSize: 12, marginVertical: 8 }}>
      {t('continued at')} {hhmm(at)}
      {where ? ` · ${where}` : ''}
    </Text>
  );
}

export function EntryMedia({
  entry,
  thread,
  onOpen,
}: {
  entry: Extract<Entry, { type: 'media' }>;
  thread: string;
  onOpen: (id: string) => void;
}) {
  const { client } = useSession();
  const url = client?.mediaUrl(thread, entry.media.sha);
  if (!url) return null;
  return (
    <View style={{ marginLeft: INDENT, marginRight: 12, marginVertical: 4 }}>
      <MediaCard
        url={url}
        kind={entry.media.kind}
        name={entry.media.name}
        kept={entry.media.kept}
        onOpen={() => onOpen(`m:${entry.media.sha}`)}
      />
    </View>
  );
}

export type { ThreadLine, Member };
export { useMemo };
