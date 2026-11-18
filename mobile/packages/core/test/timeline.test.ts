import { describe, expect, it } from 'vitest';
import {
  buildMemberThread,
  buildTimeline,
  activeMembers,
  memberState,
  bucketize,
  listSections,
} from '../src';
import { fixture } from './fixtures';

const channel = fixture('channel');
const input = () => ({
  thread: channel.thread,
  said: channel.room,
  members: channel.members,
  prompts: [],
});

describe('timeline from the channel fixture', () => {
  const entries = buildTimeline(input());
  const posts = entries.filter((e) => e.type === 'post');

  it('keeps the person, the lead and channel posts, and folds member work away', () => {
    const whos = posts.map((p) => (p.type === 'post' ? p.voice.who : ''));
    expect(whos[0]).toBe('you');
    expect(whos).toContain('lead');
    // A member's own tool calls (lane > 0, member set) never reach the body.
    expect(entries.filter((e) => e.type === 'work')).toHaveLength(0);
  });

  it('draws one assignment post per member with a bar under it', () => {
    const bars = entries.filter((e) => e.type === 'bar');
    expect(bars.map((b) => (b.type === 'bar' ? b.bar.name : ''))).toEqual([
      'implementer',
      'reviewer',
    ]);
    const impl = bars[0]!;
    if (impl.type !== 'bar') throw new Error();
    expect(impl.bar.replies).toBe(3);
    expect(impl.bar.needs).toBe('trial');
    expect(impl.bar.state.label).toBe('trial clean');
    const rev = bars[1]!;
    if (rev.type !== 'bar') throw new Error();
    expect(rev.bar.state.tone).toBe('live');
    expect(rev.bar.state.label).toContain('Read test_greet.py');
  });

  it('puts a bar straight after its handoff post, not after thread replies', () => {
    const idx = entries.findIndex((e) => e.type === 'bar' && e.bar.name === 'implementer');
    const before = entries[idx - 1]!;
    expect(before.type).toBe('post');
    if (before.type === 'post') expect(before.voice.text).toContain('@implementer tests');
  });

  it('shows a reply sent also to the channel, and not the thread-only replies', () => {
    const texts = posts.map((p) => (p.type === 'post' ? p.voice.text : ''));
    expect(texts.some((t) => t.startsWith('Done: two tests'))).toBe(true);
    expect(texts.some((t) => t.includes('Tests only, greet.py'))).toBe(false);
  });

  it('flags @you posts and the user note, which continues the member thread', () => {
    const you = posts.find((p) => p.type === 'post' && p.voice.mentionsYou);
    expect(you).toBeTruthy();
    const note = posts.find((p) => p.type === 'post' && p.voice.mine);
    expect(note).toBeTruthy();
    // The second handoff to the same member continues its thread: still two bars.
    expect(entries.filter((e) => e.type === 'bar')).toHaveLength(2);
  });

  it('draws a completion turn as one reports line naming the members', () => {
    const line = entries.find((e) => e.type === 'reports');
    expect(line && line.type === 'reports' && line.names).toEqual(['implementer']);
  });

  it('shows the live line while working and open prompts for this thread', () => {
    const live = entries.find((e) => e.type === 'live');
    expect(live).toBeTruthy();
    const prompt = {
      id: 'p1',
      thread_id: channel.thread.thread.id,
      thread_title: '',
      project: '',
      kind: 'permission',
      agent: null,
      model: null,
      role: 'lead',
      title: 'Run?',
      detail: 'cargo test',
      options: [['allow', 'Allow']],
      created_at: 1,
      read_only: false,
      lead: true,
      workspace: null,
    };
    const withAsk = buildTimeline({
      ...input(),
      prompts: [prompt as any, { ...prompt, id: 'p2', thread_id: 'other' } as any],
    });
    expect(withAsk.filter((e) => e.type === 'ask')).toHaveLength(1);
  });
});

describe('member thread', () => {
  it('lays out the assignment, replies and the seats work with a continuation divider', () => {
    const { head, lines } = buildMemberThread({ ...input(), name: 'implementer' });
    expect(head.name).toBe('implementer');
    expect(lines[0]!.type).toBe('parent');
    expect(lines.some((l) => l.type === 'work')).toBe(true);
    expect(lines.some((l) => l.type === 'divider')).toBe(true);
    const reports = lines.filter((l) => l.type === 'post' && l.report);
    expect(reports.length).toBeGreaterThan(0);
  });
});

describe('status and lists', () => {
  it('reads members needing the person as active', () => {
    expect(activeMembers(channel.members).map((m: any) => m.name)).toEqual([
      'reviewer',
      'implementer',
    ]);
    expect(memberState(undefined).label).toBe('done');
  });

  it('buckets sidebar threads by what they need and lists pinned first', () => {
    const projects = fixture('sidebar').projects ?? fixture('sidebar');
    const threads = projects.flatMap((p: any) => p.threads);
    const buckets = bucketize(threads);
    expect(buckets.map((b) => b.name)).toEqual([
      'Needs you',
      'Working',
      'In the background',
      'Done',
    ]);
    const pinned = structuredClone(projects);
    pinned[0].threads[1].pinned = true;
    const sections = listSections(pinned);
    expect(sections[0]!.kind).toBe('pinned');
    expect(listSections(projects, 'scheduler')[0]!.threads).toHaveLength(1);
    expect(listSections(projects, 'zzz')).toHaveLength(0);
  });
});

describe('turn footers with overlapping turns', () => {
  const item = (seq: number, turn: string, kind: string, at: number, text = '') =>
    ({
      seq,
      turn,
      kind,
      at,
      text,
      data: null,
      lane: 0,
      member: null,
      failed: false,
      role: null,
      agent: 'claude',
      provider: 'anthropic',
      model: 'opus',
    }) as any;
  it('puts each footer under its own turn even when the next turn started first', () => {
    const base = channel.thread;
    const items = [
      item(1, 'A', 'user_message', 1000, 'first'),
      item(2, 'B', 'user_message', 2000, 'second'),
      item(3, 'A', 'agent_message', 3000, 'answer to first'),
      item(4, 'B', 'agent_message', 5000, 'answer to second'),
    ];
    const entries = buildTimeline({
      thread: {
        ...base,
        items,
        files: [],
        seats: [],
        thread: { ...base.thread, status: 'idle' },
      },
      said: [],
      members: [],
    });
    const shape = entries.map((e) => (e.type === 'post' ? e.voice.text : e.type));
    expect(shape).toEqual([
      'first',
      'answer to first',
      'foot',
      'second',
      'answer to second',
      'foot',
    ]);
    const foots = entries.filter((e) => e.type === 'foot');
    expect(foots.map((f) => (f.type === 'foot' ? f.ms : 0))).toEqual([2000, 3000]);
  });
});
