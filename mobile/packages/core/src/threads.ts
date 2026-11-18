/**
 * Threads as Slack draws them, over the data model the desktop has: a thread is a member's
 * conversation hanging from the lead's assignment post (`Said` rows with `member`), a reply sent
 * "also to the channel" is a row with `broadcast`. Nothing here invents a thread on a message
 * that is not one; the person can only reply in an existing member thread. Pure.
 */
import type { Member } from '@orochi/protocol';
import { memberState, type MemberState } from './status';
import { ago, dayNumber, toMs } from './time';
import {
  buildMemberThread,
  type Entry,
  type ThreadLine,
  type TimelineInput,
  type Voice,
} from './timeline';
import { plural, t, type Lang } from './i18n';

const lowered = (v: unknown) => String(v ?? '').toLowerCase();

export interface ThreadSummary {
  name: string;
  /** The assignment post the thread hangs from. */
  parent: Voice | null;
  /** Replies in the thread (the parent is not one). */
  replies: number;
  /** Up to three distinct people who replied, the member first; empty when nobody has. */
  participants: Voice[];
  lastAt: number | null;
  lastText: string;
  lastWho: string;
  /** Times of the replies somebody else wrote (what can be unread). */
  replyAts: number[];
  state: MemberState;
  live: boolean;
  /** Finished and unremarkable ("done"): the footer says nothing about its work. */
  settled: boolean;
  needs: string | null;
  /** Asking you, or mentioned you. */
  needsYou: boolean;
}

const isReply = (l: ThreadLine): l is Extract<ThreadLine, { type: 'post' }> => l.type === 'post';

export const needsYouOf = (needs: string | null | undefined) =>
  needs === 'asking' || needs === 'mention';

/** The names that have a thread: every member, and everyone an assignment names. */
export function threadNames(input: TimelineInput): string[] {
  const seen = new Map<string, string>();
  for (const m of input.members) seen.set(lowered(m.name), m.name);
  for (const l of input.said)
    if (l.kind === 'message' && l.via === 'handoff' && l.member)
      seen.set(lowered(l.member), l.member);
  return [...seen.values()];
}

export function summarizeLines(
  name: string,
  lines: ThreadLine[],
  member: Member | undefined,
  lang: Lang = 'en',
): ThreadSummary {
  const parentLine = lines.find((l) => l.type === 'parent');
  const posts = lines.filter(isReply);
  const people = new Map<string, Voice>();
  for (const p of posts) {
    const key = p.voice.mine ? 'you' : lowered(p.voice.role || p.voice.who);
    if (!people.has(key)) people.set(key, p.voice);
  }
  const last = posts.at(-1);
  const state = memberState(member, lang);
  return {
    name,
    parent: parentLine && 'voice' in parentLine ? parentLine.voice : null,
    replies: posts.length,
    participants: [...people.values()].slice(0, 3),
    lastAt: last ? toMs(last.voice.at) : null,
    lastText: last?.voice.text ?? '',
    lastWho: last?.voice.who ?? '',
    replyAts: posts.filter((p) => !p.voice.mine).map((p) => toMs(p.voice.at) ?? 0),
    state,
    live: Boolean(member?.live),
    settled: state.label === t('done', lang),
    needs: member?.needs ?? null,
    needsYou: needsYouOf(member?.needs) || member?.state === 'asking',
  };
}

/** Every thread of a conversation that has an assignment or a reply. */
export function summarizeThreads(input: TimelineInput, lang: Lang = 'en'): ThreadSummary[] {
  const out: ThreadSummary[] = [];
  for (const name of threadNames(input)) {
    const member = input.members.find((m) => lowered(m.name) === lowered(name));
    const { lines } = buildMemberThread({ ...input, name });
    const s = summarizeLines(name, lines, member, lang);
    if (s.parent || s.replies > 0) out.push(s);
  }
  return out;
}

export interface Footer {
  unread: boolean;
  unreadCount: number;
  needsYou: boolean;
  /** The member's work, only when it says something ("working…", "failed"): not "done". */
  caption: string | null;
}

/** What a thread footer shows, given when the person last looked at the thread. */
export function footerOf(s: ThreadSummary, seenAt: number | null | undefined): Footer {
  const after = seenAt ?? 0;
  const unreadCount = s.replyAts.filter((at) => at > after).length;
  const caption = s.live || !s.settled ? s.state.label : null;
  return { unread: unreadCount > 0, unreadCount, needsYou: s.needsYou, caption };
}

/** Unread (or needing you) first, then by the latest reply; ties by name. */
export function orderThreads(
  rows: ThreadSummary[],
  seen: Record<string, number | undefined>,
): ThreadSummary[] {
  const hot = (s: ThreadSummary) => footerOf(s, seen[lowered(s.name)]).unread || s.needsYou;
  return [...rows].sort(
    (a, b) =>
      Number(hot(b)) - Number(hot(a)) ||
      (b.lastAt ?? -1) - (a.lastAt ?? -1) ||
      a.name.localeCompare(b.name),
  );
}

export const unreadThreads = (
  rows: ThreadSummary[],
  seen: Record<string, number | undefined>,
): number => rows.filter((s) => footerOf(s, seen[lowered(s.name)]).unread || s.needsYou).length;

/** The time to remember as "seen": the latest reply, so clock differences cannot matter. */
export const latestReplyAt = (lines: ThreadLine[]): number | null => {
  let best: number | null = null;
  for (const l of lines)
    if (l.type === 'post') {
      const at = toMs(l.voice.at);
      if (at !== null && (best === null || at > best)) best = at;
    }
  return best;
};

const fresh = (l: ThreadLine, after: number) =>
  l.type === 'post' && !l.voice.mine && (toMs(l.voice.at) ?? 0) > after;

/** Where to land when a thread opens: the first reply you have not seen, or null for the bottom. */
export function firstUnreadIndex(
  lines: ThreadLine[],
  seenAt: number | null | undefined,
): number | null {
  const i = lines.findIndex((l) => fresh(l, seenAt ?? 0));
  return i < 0 ? null : i;
}

/** How many replies arrived after the point the reader was last at the bottom. */
export const newReplyCount = (lines: ThreadLine[], anchorAt: number | null | undefined): number =>
  lines.filter((l) => fresh(l, anchorAt ?? 0)).length;

/** The thread screen's two parts: the parent post, and the replies with day dividers. */
export function threadView(lines: ThreadLine[]): {
  parent: Voice | null;
  replies: ThreadLine[];
  count: number;
} {
  const parentLine = lines.find((l) => l.type === 'parent');
  const rest = lines.filter((l) => l.type !== 'parent');
  return {
    parent: parentLine && 'voice' in parentLine ? parentLine.voice : null,
    replies: withThreadDays(rest),
    count: rest.filter((l) => l.type === 'post').length,
  };
}

// ---- day dividers -------------------------------------------------------------------------

const atOfEntry = (e: Entry): number | null =>
  e.type === 'post'
    ? toMs(e.voice.at)
    : e.type === 'media'
      ? toMs(e.at)
      : e.type === 'foot'
        ? toMs(e.endedAt)
        : null;

/** A divider before the first dated row of each local day. The row after a divider starts a new run. */
export function withDays(entries: Entry[]): Entry[] {
  const out: Entry[] = [];
  let day: number | null = null;
  for (const e of entries) {
    const at = atOfEntry(e);
    const d = at === null ? null : dayNumber(at);
    if (at !== null && d !== day) {
      out.push({ type: 'day', key: `day-${e.key}`, at });
      day = d;
      out.push(e.type === 'post' && e.run ? { ...e, run: false } : e);
      continue;
    }
    out.push(e);
  }
  return out;
}

export function withThreadDays(lines: ThreadLine[]): ThreadLine[] {
  const out: ThreadLine[] = [];
  let day: number | null = null;
  for (const l of lines) {
    const at = l.type === 'post' ? toMs(l.voice.at) : null;
    const d = at === null ? null : dayNumber(at);
    if (at !== null && d !== day) {
      out.push({ type: 'day', key: `day-${l.key}`, at });
      day = d;
      out.push(l.type === 'post' && l.run ? { ...l, run: false } : l);
      continue;
    }
    out.push(l);
  }
  return out;
}

/** The caption above a post that was also sent to the channel from a thread, or null. */
export const repliedToThread = (v: Pick<Voice, 'broadcast' | 'thread'>): string | null =>
  v.broadcast && v.thread ? v.thread : null;

/** Slack's reading of a footer for a screen reader: "Thread with 3 replies, last reply 4 minutes ago, unread". */
export function footerLabel(
  s: ThreadSummary,
  f: Footer,
  now: number,
  lang: Lang,
  tr: (key: string) => string,
): string {
  const parts = [`${tr('Thread')} ${s.name}: ${plural(s.replies, 'reply', 'replies', lang)}`];
  if (s.lastAt) parts.push(`${tr('last reply')} ${ago(s.lastAt, now, lang, true)}`);
  if (f.needsYou) parts.push(tr('Needs you'));
  if (f.unread) parts.push(tr('unread'));
  return parts.join(lang === 'ja' ? '、' : ', ');
}
