import { describe, expect, it } from 'vitest';
import {
  ago,
  buildMemberThread,
  buildTimeline,
  dayLabel,
  firstUnreadIndex,
  footerLabel,
  footerOf,
  latestReplyAt,
  newReplyCount,
  orderThreads,
  repliedToThread,
  summarizeThreads,
  threadView,
  unreadThreads,
  withDays,
  type Entry,
} from '../src';
import { fixture } from './fixtures';

const channel = fixture('channel');
const input = () => ({
  thread: channel.thread,
  said: channel.room,
  members: channel.members,
  prompts: [],
});

describe('thread summaries from the channel fixture', () => {
  const all = summarizeThreads(input());
  const impl = all.find((s) => s.name === 'implementer')!;
  const rev = all.find((s) => s.name === 'reviewer')!;

  it('has one thread per member, with its assignment as the parent', () => {
    expect(all.map((s) => s.name).sort()).toEqual(['implementer', 'reviewer']);
    expect(impl.parent?.text).toContain('@implementer tests');
  });

  it('counts replies, names up to three distinct participants, and knows the last reply', () => {
    const lines = buildMemberThread({ ...input(), name: 'implementer' }).lines;
    expect(impl.replies).toBe(lines.filter((l) => l.type === 'post').length);
    expect(impl.replies).toBeGreaterThanOrEqual(3);
    expect(impl.participants.length).toBeLessThanOrEqual(3);
    const keys = impl.participants.map((v) => v.role || v.who);
    expect(new Set(keys).size).toBe(keys.length);
    expect(impl.lastAt).not.toBeNull();
    expect(impl.lastText.length).toBeGreaterThan(0);
  });

  it('says what needs you, and keeps "done" off the footer', () => {
    expect(impl.needs).toBe('trial');
    expect(impl.needsYou).toBe(false);
    expect(footerOf(impl, null).caption).toBe('trial clean');
    expect(rev.live).toBe(true);
    expect(footerOf(rev, null).caption).toContain('Read test_greet.py');
    const settled = {
      ...impl,
      state: { tone: 'muted' as const, label: 'done' },
      settled: true,
      live: false,
    };
    expect(footerOf(settled, null).caption).toBeNull();
  });
});

describe('unread given what was last seen', () => {
  const s = summarizeThreads(input()).find((x) => x.name === 'implementer')!;
  it('is unread when nothing was seen, or something newer than that arrived', () => {
    expect(footerOf(s, undefined).unread).toBe(true);
    expect(footerOf(s, s.lastAt! - 1).unreadCount).toBeGreaterThan(0);
  });
  it('is read once the latest reply is seen', () => {
    const at = latestReplyAt(buildMemberThread({ ...input(), name: 'implementer' }).lines)!;
    expect(footerOf(s, at).unread).toBe(false);
    expect(footerOf(s, at).unreadCount).toBe(0);
  });
  it('never counts your own replies as unread', () => {
    expect(footerOf({ ...s, replyAts: [] }, 0).unread).toBe(false);
  });
});

describe('the threads list order', () => {
  const rows = summarizeThreads(input());
  const impl = rows.find((x) => x.name === 'implementer')!;
  const rev = rows.find((x) => x.name === 'reviewer')!;
  it('puts unread first, then the latest reply', () => {
    const seenAll = { implementer: impl.lastAt!, reviewer: rev.lastAt! };
    expect(orderThreads(rows, seenAll).map((s) => s.name)).toEqual(
      [impl, rev].sort((a, b) => b.lastAt! - a.lastAt!).map((s) => s.name),
    );
    const older = impl.lastAt! < rev.lastAt! ? impl : rev;
    const newer = older === impl ? rev : impl;
    const seen = { [older.name]: 0, [newer.name]: newer.lastAt! };
    expect(orderThreads(rows, seen)[0]!.name).toBe(older.name);
    expect(unreadThreads(rows, seen)).toBe(1);
  });
  it('counts a thread that needs you as hot even when seen', () => {
    const asking = { ...impl, needsYou: true };
    const seen = { implementer: impl.lastAt!, reviewer: rev.lastAt! };
    expect(unreadThreads([asking, rev], seen)).toBe(1);
  });
});

describe('the broadcast caption', () => {
  it('marks the reply that was also sent to the channel, and only that one', () => {
    const entries = buildTimeline(input());
    const posts = entries.filter((e): e is Extract<Entry, { type: 'post' }> => e.type === 'post');
    const sent = posts.filter((p) => repliedToThread(p.voice));
    expect(sent).toHaveLength(1);
    expect(sent[0]!.voice.text).toContain('Done: two tests');
    expect(sent[0]!.voice.thread).toBe('implementer');
    expect(posts.filter((p) => !repliedToThread(p.voice)).length).toBeGreaterThan(0);
  });
  it('is also marked in the thread, as "also sent"', () => {
    const lines = buildMemberThread({ ...input(), name: 'implementer' }).lines;
    expect(lines.filter((l) => l.type === 'post' && l.voice.broadcast)).toHaveLength(1);
  });
});

describe('day dividers', () => {
  const day = (d: number, h = 10) => new Date(2026, 10, d, h, 0, 0).getTime();
  const post = (key: string, at: number, run = false): Entry => ({
    type: 'post',
    key,
    run,
    turn: null,
    voice: {
      who: 'lead',
      to: null,
      role: 'lead',
      agent: null,
      provider: null,
      model: null,
      at,
      text: key,
      mine: false,
      mentions: [],
      mentionsYou: false,
    },
  });
  it('labels Today, Yesterday and a date', () => {
    const now = day(18, 15);
    expect(dayLabel(day(18, 1), now)).toEqual({ kind: 'today' });
    expect(dayLabel(day(17, 23), now)).toEqual({ kind: 'yesterday' });
    expect(dayLabel(day(10), now)).toEqual({ kind: 'date', at: day(10) });
  });
  it('puts a divider before the first post of each day and starts a new run after it', () => {
    const out = withDays([
      post('a', day(16)),
      post('b', day(16, 11), true),
      post('c', day(17), true),
    ]);
    expect(out.map((e) => e.type)).toEqual(['day', 'post', 'post', 'day', 'post']);
    const b = out[2]!;
    expect(b.type === 'post' && b.run).toBe(true);
    const c = out[4]!;
    expect(c.type === 'post' && c.run).toBe(false);
  });
  it('gives rows with no time no divider of their own', () => {
    const out = withDays([
      { type: 'route', key: 'r', text: '', resumed: false },
      post('a', day(16)),
    ]);
    expect(out.map((e) => e.type)).toEqual(['route', 'day', 'post']);
  });
});

describe('the thread screen', () => {
  const lines = buildMemberThread({ ...input(), name: 'implementer' }).lines;
  it('splits the parent from the replies and counts the replies', () => {
    const v = threadView(lines);
    expect(v.parent?.text).toContain('@implementer tests');
    expect(v.count).toBe(lines.filter((l) => l.type === 'post').length);
    expect(v.replies.some((l) => l.type === 'parent')).toBe(false);
    expect(v.replies.some((l) => l.type === 'day')).toBe(true);
  });
  it('finds the first unread reply, or nothing when all are seen', () => {
    const first = firstUnreadIndex(lines, 0)!;
    expect(lines[first]!.type).toBe('post');
    expect(firstUnreadIndex(lines, latestReplyAt(lines))).toBeNull();
  });
  it('detects replies that arrived after the reader was last at the bottom', () => {
    const seen = latestReplyAt(lines)!;
    expect(newReplyCount(lines, seen)).toBe(0);
    const base = (lines.find((l) => l.type === 'post') as any).voice;
    const arrived = [
      ...lines,
      {
        type: 'post' as const,
        key: 'n',
        run: false,
        report: false,
        voice: { ...base, at: seen + 5000, mine: false },
      },
      {
        type: 'post' as const,
        key: 'm',
        run: false,
        report: false,
        voice: { ...base, at: seen + 6000, mine: true },
      },
    ];
    expect(newReplyCount(arrived, seen)).toBe(1);
  });
});

describe('relative time and the accessibility label', () => {
  const now = 1_800_000_000_000;
  it('reads "just now", minutes, hours and days, in English and Japanese', () => {
    expect(ago(now - 10_000, now)).toBe('just now');
    expect(ago(now - 4 * 60_000, now)).toBe('4m ago');
    expect(ago(now - 4 * 60_000, now, 'en', true)).toBe('4 minutes ago');
    expect(ago(now - 61 * 60_000, now, 'en', true)).toBe('1 hour ago');
    expect(ago(now - 3 * 86_400_000, now)).toBe('3d ago');
    expect(ago(now - 4 * 60_000, now, 'ja')).toBe('4分前');
    expect(ago(now / 1000 - 120, now)).toBe('2m ago');
  });
  it('reads a footer for a screen reader', () => {
    const s = summarizeThreads(input()).find((x) => x.name === 'implementer')!;
    const f = footerOf(s, null);
    const text = footerLabel(s, f, s.lastAt! + 4 * 60_000, 'en', (k) => k);
    expect(text).toContain(`${s.replies} repl`);
    expect(text).toContain('last reply 4 minutes ago');
    expect(text).toContain('unread');
  });
});
