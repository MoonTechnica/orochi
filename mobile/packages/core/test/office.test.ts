import { describe, expect, it } from 'vitest';
import {
  FRESH_MS,
  ROOM_ORDER,
  addressees,
  buildOffice,
  chart,
  hueOf,
  newTracker,
  officeSummary,
  stepOffice,
  type OfficeInput,
} from '../src';
import { fixture } from './fixtures';

const office = fixture('office');
const NOW = 1794636100000;
const talkAt = (i: number, ago: number, over: object = {}) => {
  const s = office.script[i];
  return {
    seq: i + 1,
    project_id: 'p',
    via: 'mailbox',
    from: s.from[1],
    from_thread: s.from[0],
    from_role: s.from[1],
    to: s.to[1],
    to_thread: s.to[0],
    to_role: s.to[1],
    text: s.text,
    mentions: s.mentions ?? [],
    at: NOW - ago,
    read: null,
    ...over,
  } as any;
};
const input = (over: Partial<OfficeInput> = {}): OfficeInput => ({
  projects: structuredClone(office.sidebar),
  seats: office.live_seats,
  tasks: office.live_tasks,
  talk: [],
  ...over,
});
const room = (o: ReturnType<typeof buildOffice>['office'], id: string) =>
  o.floors.flatMap((f) => f.rooms).find((r) => r.id === id)!;

describe('the office, from the recorded desktop data', () => {
  it('makes a floor per project and a room per busy conversation, the one that needs you first', () => {
    const { office: o } = buildOffice(input(), newTracker(), NOW);
    expect(o.floors.map((f) => f.name)).toEqual(['orochi', 'web-app']);
    expect(o.floors[0].rooms.map((r) => r.id)).toEqual(['t2', 't1']);
    expect(o.floors[1].rooms.map((r) => r.id)).toEqual(['t3']);
    // idle conversations with nobody live are not rooms
    expect(o.floors.flatMap((f) => f.rooms.map((r) => r.id))).not.toContain('t5');
    expect(ROOM_ORDER.needs_you).toBeLessThan(ROOM_ORDER.working);
  });

  it('reads the state of each person: asking lead, working seats, an idle member', () => {
    const { office: o } = buildOffice(input(), newTracker(), NOW);
    const t2 = room(o, 't2');
    expect(t2.people.map((p) => [p.role, p.state])).toEqual([
      ['lead', 'asking'],
      ['tester', 'idle'],
    ]);
    expect(t2.people[0].bubble).toMatchObject({ kind: 'ask' });
    const t1 = room(o, 't1');
    expect(t1.people.map((p) => p.state)).toEqual(['working', 'working', 'working']);
    expect(t1.people[1]).toMatchObject({
      doing: 'Editing desktop/src/app.css',
      seatId: 'b',
      model: 'gpt-6.1-sol',
      bubble: { kind: 'work', text: 'Editing desktop/src/app.css' },
    });
    expect(t1.people[2].background).toBe(true);
    expect(t1.people[2].readOnly).toBe(true);
  });

  it('puts a sub-agent beside the seat it runs under, without the Task: prefix', () => {
    const { office: o } = buildOffice(input(), newTracker(), NOW);
    expect(room(o, 't1').people[0].tasks).toEqual([
      {
        id: 1,
        title: 'find every caller of drawStatus',
        doing: '',
        calls: 4,
        startedAt: 1794636081680,
      },
    ]);
    expect(room(o, 't3').people.find((p) => p.role === 'backend')!.tasks).toHaveLength(1);
    expect(room(o, 't3').people[0].tasks).toHaveLength(0);
  });

  it('counts what the desktop counts', () => {
    const talk = [talkAt(0, 1000), talkAt(1, 2000)];
    const { office: o } = buildOffice(input({ talk }), newTracker(), NOW);
    expect(o.stats).toEqual({ working: 7, assistants: 2, asking: 1, messages: 2, rooms: 3 });
    expect(o.quiet).toBe(false);
  });

  it('is quiet with nothing to show, and the summary says so', () => {
    const quiet = input({ projects: [], seats: [], tasks: [] });
    const { office: o } = buildOffice(quiet, newTracker(), NOW);
    expect(o.quiet).toBe(true);
    expect(o.floors).toEqual([]);
    expect(officeSummary(quiet, NOW).faces).toEqual([]);
    expect(officeSummary(input(), NOW).faces[0].state).toBe('asking');
  });

  it('knows who a message is for, within and across rooms, and marks an unopened one', () => {
    const across = talkAt(2, 3000); // t1 designer -> t2 tester
    expect(addressees(across)).toEqual([{ thread: 't2', role: 'tester' }]);
    const mention = talkAt(4, 3000); // @backend in t3
    expect(addressees(mention).map((a) => a.role)).toEqual(['backend']);
    const { office: o } = buildOffice(input({ talk: [across, mention] }), newTracker(), NOW);
    const tester = room(o, 't2').people.find((p) => p.role === 'tester')!;
    expect(tester.state).toBe('mentioned');
    expect(tester.mail?.seq).toBe(across.seq);
    expect(room(o, 't2').lines[0]).toMatchObject({
      direction: 'in',
      across: 'Redesign the home screen',
      from: 'designer',
    });
    expect(room(o, 't1').lines[0]).toMatchObject({
      direction: 'out',
      across: 'Fix the flaky mailbox test',
    });
    // the envelope is gone once it has been opened
    const opened = buildOffice(
      input({ talk: [across], seen: new Set([across.seq]) }),
      newTracker(),
      NOW,
    );
    expect(room(opened.office, 't2').people.find((p) => p.role === 'tester')!.state).toBe('idle');
    // and once it is a minute old
    const old = buildOffice(input({ talk: [talkAt(2, 61000)] }), newTracker(), NOW);
    expect(room(old.office, 't2').people.find((p) => p.role === 'tester')!.mail).toBeNull();
  });

  it('lets the speaker speak for a few seconds, then says it reported', () => {
    const fresh = buildOffice(input({ talk: [talkAt(3, 1000)] }), newTracker(), NOW).office;
    const tester = room(fresh, 't2').people.find((p) => p.role === 'tester')!;
    expect(tester.speaking).toBe(true);
    expect(tester.bubble).toMatchObject({ kind: 'talk', to: 'lead' });
    expect(tester.state).toBe('reported');
    const later = buildOffice(
      input({ talk: [talkAt(3, FRESH_MS + 1000)] }),
      newTracker(),
      NOW,
    ).office;
    expect(room(later, 't2').people.find((p) => p.role === 'tester')!.speaking).toBe(false);
    expect(fresh.nextChange).toBeGreaterThan(NOW);
  });

  it('is the same hue and face for the same agent', () => {
    expect(hueOf({ role: 'lead', agent: 'claude' })).toBe(hueOf({ role: 'lead', agent: 'claude' }));
    expect(hueOf({ role: 'lead', agent: 'claude' })).not.toBe(
      hueOf({ role: 'designer', agent: 'codex' }),
    );
    const a = buildOffice(input(), newTracker(), NOW).office;
    const b = buildOffice(input(), newTracker(), NOW + 5).office;
    expect(room(a, 't1').people.map((p) => p.hue)).toEqual(room(b, 't1').people.map((p) => p.hue));
  });
});

describe('the chart', () => {
  it('hangs members under the lead, and under whoever hired them, and cuts a loop', () => {
    const agent = (role: string, over: object = {}) => ({
      role,
      agent: 'x',
      provider: 'p',
      model: '',
      seats: 1,
      live: 1,
      lead: false,
      background: false,
      last_at: 0,
      ...over,
    });
    const th = {
      agents: [
        agent('lead', { lead: true }),
        agent('a'),
        agent('b', { manager: 'a' }),
        agent('c', { manager: 'c2' }),
        agent('c2', { manager: 'c' }),
      ],
    } as any;
    expect(chart(th).map((x) => [x.member.role, x.depth])).toEqual([
      ['lead', 0],
      ['a', 1],
      ['b', 2],
      ['c', 0],
      ['c2', 1],
    ]);
  });
});

describe('arrivals, departures and rooms that end', () => {
  const all = (o: OfficeInput) =>
    o.projects.flatMap((p) => p.threads.map((th: any) => ({ ...th, where: p.name })));
  it('draws the first look at rest, then lets a finished agent wave and leave', () => {
    const t0 = buildOffice(input(), newTracker(), NOW);
    expect(t0.tracker.leaving.size).toBe(0);
    expect(t0.tracker.fresh.size).toBe(0);
    const ended = input();
    for (const th of ended.projects[0].threads)
      for (const m of th.agents) if (m.role === 'reviewer') m.live = 0;
    ended.seats = ended.seats.filter((s) => s.seat_id !== 'c');
    const t1 = buildOffice(ended, t0.tracker, NOW + 1000);
    expect(room(t1.office, 't1').people.find((p) => p.role === 'reviewer')!.state).toBe('leaving');
    expect(t1.tracker.until).toBe(NOW + 1000 + 2400);
    // after the farewell the reviewer is no longer drawn; the lead always is
    const t2 = buildOffice(ended, t1.tracker, NOW + 4000);
    expect(room(t2.office, 't1').people.map((p) => p.role)).toEqual(['lead', 'designer']);
    // back to work: drawn again
    const t3 = buildOffice(input(), t2.tracker, NOW + 5000);
    expect(room(t3.office, 't1').people).toHaveLength(3);
  });

  it('keeps a finished conversation as done for a minute, and closes a deleted one', () => {
    const t0 = buildOffice(input(), newTracker(), NOW);
    const done = input();
    const th = done.projects[1].threads.find((x: any) => x.id === 't3')!;
    th.status = 'unread';
    for (const m of th.agents) m.live = 0;
    const t1 = buildOffice(done, t0.tracker, NOW + 1000);
    const r = room(t1.office, 't3');
    expect(r).toMatchObject({ done: true, status: 'done' });
    expect(r.people.length).toBeGreaterThan(0);
    expect(room(buildOffice(done, t1.tracker, NOW + 62000).office, 't3')).toBeUndefined();
    const gone = input();
    gone.projects[1].threads = gone.projects[1].threads.filter((x: any) => x.id !== 't3');
    const t2 = buildOffice(gone, t0.tracker, NOW + 1000);
    expect(room(t2.office, 't3')).toMatchObject({ closing: true });
    expect(t2.tracker.wrap.get('t3')!.until).toBe(NOW + 1000 + 1300);
  });

  it('never edits the tracker it is given', () => {
    const t0 = buildOffice(input(), newTracker(), NOW).tracker;
    const before = JSON.stringify([...t0.leaving]);
    const ended = input();
    ended.projects[0].threads[0].agents[1].live = 0;
    stepOffice(
      t0,
      all(ended),
      all(ended).filter((x) => x.status in ROOM_ORDER),
      NOW + 10,
    );
    expect(JSON.stringify([...t0.leaving])).toBe(before);
  });

  it('marks a room that has just opened', () => {
    const t0 = buildOffice(
      input({ projects: [structuredClone(office.sidebar[1])] }),
      newTracker(),
      NOW,
    );
    const t1 = buildOffice(input(), t0.tracker, NOW + 100);
    expect(room(t1.office, 't2').arriving).toBe(true);
  });
});
