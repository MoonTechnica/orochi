import { describe, expect, it } from 'vitest';
import {
  activityBars,
  buildHome,
  dismissMention,
  groupTrail,
  headCounts,
  type MachineSnapshot,
} from '../src';

const th = (id: string, status: string, over: object = {}) =>
  ({ id, title: `T-${id}`, status, archived: false, pinned: false, updated_at: 1, ...over }) as any;
const snap = (key: string, over: Partial<MachineSnapshot> = {}): MachineSnapshot => ({
  key,
  name: key,
  state: 'online',
  projects: [{ id: 'p', name: 'proj', threads: [] } as any],
  prompts: [],
  mentions: [],
  seats: [],
  tasks: [],
  ...over,
});

describe('home dashboard', () => {
  it('puts approvals and mentions above running work, across desktops', () => {
    const a = snap('home', {
      projects: [
        { id: 'p', name: 'x', threads: [th('1', 'working'), th('2', 'background')] } as any,
      ],
    });
    const b = snap('work', {
      projects: [{ id: 'q', name: 'y', threads: [th('3', 'needs_you'), th('4', 'idle')] } as any],
      prompts: [{ id: 'a1', thread_id: '3', thread_title: 'Ask', project: 'y' } as any],
      mentions: [{ message_id: 7, thread_id: '4', who: 'alice' } as any],
    });
    const home = buildHome([a, b]);
    expect(home.agents.map((r) => r.kind)).toEqual([
      'approval',
      'mention',
      'working',
      'background',
    ]);
    expect(home.agents[0]).toMatchObject({ machineKey: 'work', thread: '3' });
    expect(home.active).toBe(4);
  });
  it('shows one row per conversation, the most urgent state winning', () => {
    const m = snap('a', {
      projects: [{ id: 'p', name: 'x', threads: [th('1', 'working')] } as any],
      prompts: [{ id: 'q', thread_id: '1', thread_title: 'Q', project: 'x' } as any],
    });
    expect(buildHome([m]).agents).toHaveLength(1);
    expect(buildHome([m]).agents[0].kind).toBe('approval');
  });
  it('an offline desktop keeps its card and does not stop the others', () => {
    const on = snap('on', {
      projects: [{ id: 'p', name: 'x', threads: [th('1', 'working')] } as any],
    });
    const off = snap('off', {
      state: 'offline',
      error: 'unreachable',
      projects: [{ id: 'p', name: 'x', threads: [th('9', 'working')] } as any],
    });
    const home = buildHome([off, on]);
    expect(home.agents.map((r) => r.machineKey)).toEqual(['on']);
    expect(home.desktops.map((d) => [d.key, d.state, d.error])).toEqual([
      ['off', 'offline', 'unreachable'],
      ['on', 'online', null],
    ]);
    expect(home.recent.every((r) => r.machineKey === 'on')).toBe(true);
  });
  it('recent conversations: pinned first, then newest, archived left out', () => {
    const m = snap('a', {
      projects: [
        {
          id: 'p',
          name: 'x',
          threads: [
            th('old', 'idle', { updated_at: 1 }),
            th('new', 'idle', { updated_at: 9 }),
            th('pin', 'idle', { pinned: true, updated_at: 0 }),
            th('gone', 'idle', { archived: true, updated_at: 99 }),
          ],
        } as any,
      ],
    });
    expect(buildHome([m]).recent.map((r) => r.thread.id)).toEqual(['pin', 'new', 'old']);
  });
  it('keeps every question and mention as its own item, questions first', () => {
    const m = snap('a', {
      projects: [{ id: 'p', name: 'x', threads: [th('1', 'working')] } as any],
      prompts: [
        { id: 'q1', thread_id: '1', thread_title: 'Q', project: 'x', created_at: 5 } as any,
        { id: 'q2', thread_id: '1', thread_title: 'Q', project: 'x', created_at: 9 } as any,
      ],
      mentions: [{ message_id: 3, thread_id: '1', who: 'bo', at: 9, text: 'hi' } as any],
    });
    const home = buildHome([m]);
    expect(home.needs.map((n) => n.id)).toEqual(['a:pq2', 'a:pq1', 'a:m3']);
    expect(home.agents).toHaveLength(1);
    expect(headCounts(home.agents)).toEqual({ need: 1, working: 0, background: 0 });
  });
  it('counts activity in ten-minute buckets of the last hour', () => {
    const now = 10_000_000_000_000;
    const p = [
      {
        threads: [
          th('a', 'idle', { updated_at: now - 60_000 }),
          th('b', 'idle', { updated_at: now - 11 * 60_000 }),
          th('c', 'idle', { updated_at: now - 70 * 60_000 }),
        ],
      },
    ] as any;
    expect(activityBars(p, now)).toEqual([0, 0, 0, 0, 1, 1]);
  });
  it('groups the trail by project and desktop', () => {
    const m = snap('a', {
      projects: [
        {
          id: 'p',
          name: 'x',
          threads: [th('1', 'idle', { project: 'x', project_id: 'p' })],
        } as any,
        {
          id: 'q',
          name: 'y',
          threads: [th('2', 'idle', { project: 'y', project_id: 'q' })],
        } as any,
      ],
    });
    expect(groupTrail(buildHome([m]).recent).map((g) => [g.project, g.items.length])).toEqual([
      ['x', 1],
      ['y', 1],
    ]);
  });
});

describe('dismissing a mention', () => {
  const two = buildHome([
    snap('m', {
      projects: [{ id: 'p', name: 'x', threads: [th('t1', 'idle'), th('t2', 'idle')] } as any],
      mentions: [
        { message_id: 1, thread_id: 't1', who: 'a', at: 3 } as any,
        { message_id: 2, thread_id: 't1', who: 'b', at: 2 } as any,
        { message_id: 3, thread_id: 't2', who: 'c', at: 1 } as any,
      ],
    }),
  ]).needs;
  const byMessage = (n: number) => two.find((x) => x.mention?.message_id === n)!;

  it('hides one card and tells the desktop only when its thread has no other mention left', () => {
    const first = dismissMention(two, [], byMessage(1));
    expect(first).toEqual({ dismissed: ['m:m1'], seeThread: false });
    const last = dismissMention(two, first.dismissed, byMessage(2));
    expect(last).toEqual({ dismissed: ['m:m1', 'm:m2'], seeThread: true });
  });
  it('another thread is its own: one card there marks that thread at once', () => {
    expect(dismissMention(two, [], byMessage(3)).seeThread).toBe(true);
  });
  it('dismissing twice is the same as once, and an approval never blocks a thread', () => {
    const once = dismissMention(two, ['m:m1'], byMessage(1));
    expect(once.dismissed).toEqual(['m:m1']);
    const withPrompt = [
      ...two,
      { ...byMessage(3), id: 'm:p9', kind: 'approval' as const, mention: undefined },
    ];
    expect(dismissMention(withPrompt, [], byMessage(3)).seeThread).toBe(true);
  });
  it('other desktops with the same thread id are not confused', () => {
    const other = { ...byMessage(2), id: 'n:m2', machineKey: 'n' };
    expect(dismissMention([...two, other], [], byMessage(1)).seeThread).toBe(false);
    expect(dismissMention([byMessage(1), other], [], byMessage(1)).seeThread).toBe(true);
  });
});
