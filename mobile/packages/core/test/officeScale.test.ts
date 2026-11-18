import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  COMPACT_FACES,
  COMPACT_FLOOR_ROOMS,
  COMPACT_ROOM_PEOPLE,
  MOTION_PEOPLE_LIMIT,
  ROOM_CAP,
  buildOffice,
  faceRow,
  filterCounts,
  filterFloors,
  isCompact,
  isCrowded,
  listPeople,
  motionAllowed,
  newTracker,
  officeItems,
  roomAsks,
  selectPeople,
  type Floor,
  type Person,
  type PersonState,
  type Room,
} from '../src';

const NOW = 1_800_000_000_000;
const person = (i: number, state: PersonState): Person =>
  ({ key: `r:p${i}`, role: `p${i}`, state, lead: i === 0 }) as Person;
const people = (states: PersonState[]) => states.map((s, i) => person(i, s));
const room = (id: string, status: string, n = 2, state: PersonState = 'idle'): Room =>
  ({
    id,
    title: id,
    project: 'x',
    status,
    done: false,
    closing: false,
    thread: { id, status, asking: status === 'needs_you' ? 1 : 0 },
    people: people(Array(n).fill(state)),
    lines: [],
  }) as unknown as Room;
const floor = (name: string, rooms: Room[]): Floor => ({ name, rooms });

describe('who is drawn in a crowded room', () => {
  it('draws everyone up to the cap, in the room order', () => {
    const p = people(['idle', 'working', 'asking', 'idle', 'idle', 'idle']);
    const s = selectPeople(p);
    expect(s.shown).toEqual(p);
    expect(s.more).toBe(0);
  });

  it('puts asking, then working, then message and report, then idle and leaving first', () => {
    const states: PersonState[] = [
      'idle',
      'leaving',
      'reported',
      'working',
      'idle',
      'mentioned',
      'asking',
      'working',
      'idle',
    ];
    const s = selectPeople(people(states));
    expect(s.shown.map((x) => x.state).sort()).toEqual(
      ['asking', 'working', 'working', 'mentioned', 'reported', 'idle'].sort(),
    );
    expect(s.more).toBe(3);
    expect(s.hidden.map((x) => x.role)).toEqual(['p1', 'p4', 'p8']);
    expect(s.shown.length + s.hidden.length).toBe(states.length);
  });

  it('counts what the hidden are doing', () => {
    const states: PersonState[] = Array(ROOM_CAP).fill('working');
    const s = selectPeople(people([...states, 'idle', 'idle', 'leaving', 'reported']));
    expect(s.more).toBe(4);
    expect(s.summary).toEqual({
      asking: 0,
      working: 0,
      mentioned: 0,
      reported: 1,
      idle: 2,
      leaving: 1,
    });
  });

  it('keeps the room order within a state and among the drawn', () => {
    const p = people(Array(10).fill('working'));
    const s = selectPeople(p);
    expect(s.shown.map((x) => x.role)).toEqual(['p0', 'p1', 'p2', 'p3', 'p4', 'p5']);
    expect(s.hidden.map((x) => x.role)).toEqual(['p6', 'p7', 'p8', 'p9']);
  });

  it('moves one person in or out when a state changes and leaves the rest alone', () => {
    const base = people(Array(9).fill('idle'));
    base[0]!.state = 'working';
    const before = selectPeople(base);
    expect(before.shown.map((x) => x.role)).toEqual(['p0', 'p1', 'p2', 'p3', 'p4', 'p5']);
    const after = people(base.map((x) => x.state));
    after[8]!.state = 'asking';
    const s = selectPeople(after);
    expect(s.shown.map((x) => x.role)).toEqual(['p0', 'p1', 'p2', 'p3', 'p4', 'p8']);
    // The ones that stay are in the same order as before.
    const kept = s.shown.filter((x) => before.shown.some((y) => y.role === x.role));
    expect(kept.map((x) => x.role)).toEqual(['p0', 'p1', 'p2', 'p3', 'p4']);
    // The selection is a pure function of the states.
    expect(selectPeople(after).shown.map((x) => x.role)).toEqual(s.shown.map((x) => x.role));
  });

  it('lists everyone for the full list, the ones that need attention first', () => {
    const p = people(['idle', 'working', 'asking', 'idle']);
    expect(listPeople(p).map((x) => x.state)).toEqual(['asking', 'working', 'idle', 'idle']);
    expect(listPeople(p).map((x) => x.role)).toEqual(['p2', 'p1', 'p0', 'p3']);
  });

  it('gives a compact row its faces and the count left over', () => {
    const r = faceRow(people(Array(9).fill('working')));
    expect(r.faces).toHaveLength(COMPACT_FACES);
    expect(r.more).toBe(9 - COMPACT_FACES);
    expect(faceRow(people(['idle', 'idle'])).more).toBe(0);
  });
});

describe('which rooms are one line', () => {
  it('names its thresholds', () => {
    expect([
      ROOM_CAP,
      COMPACT_FLOOR_ROOMS,
      COMPACT_ROOM_PEOPLE,
      COMPACT_FACES,
      MOTION_PEOPLE_LIMIT,
    ]).toEqual([6, 8, 12, 5, 24]);
  });

  it('is compact by the floor size or by the room size', () => {
    expect(isCompact(room('a', 'working', 3), COMPACT_FLOOR_ROOMS)).toBe(false);
    expect(isCompact(room('a', 'working', 3), COMPACT_FLOOR_ROOMS + 1)).toBe(true);
    expect(isCompact(room('a', 'working', COMPACT_ROOM_PEOPLE), 2)).toBe(false);
    expect(isCompact(room('a', 'working', COMPACT_ROOM_PEOPLE + 1), 2)).toBe(true);
  });

  const many = floor('big', [
    room('q', 'working'),
    room('ask', 'needs_you'),
    ...Array.from({ length: COMPACT_FLOOR_ROOMS }, (_, i) => room(`w${i}`, 'working')),
  ]);

  it('opens the rooms that ask by default and the one picked, only one picked per floor', () => {
    const rooms = (c = {}) =>
      officeItems([many], c).filter((i) => i.kind === 'room') as Extract<
        ReturnType<typeof officeItems>[number],
        { kind: 'room' }
      >[];
    expect(
      rooms()
        .filter((r) => r.expanded)
        .map((r) => r.room.id),
    ).toEqual(['ask']);
    expect(rooms().every((r) => r.compact)).toBe(true);
    const picked = rooms({ picked: { big: 'w2' } });
    expect(picked.filter((r) => r.expanded).map((r) => r.room.id)).toEqual(['ask', 'w2']);
    const closed = rooms({ picked: { big: 'q' }, closed: new Set(['ask']) });
    expect(closed.filter((r) => r.expanded).map((r) => r.room.id)).toEqual(['q']);
  });

  it('draws a small floor in full and skips the rooms of a folded floor', () => {
    const small = floor('small', [room('a', 'working'), room('b', 'needs_you')]);
    const items = officeItems([small, many], { folded: new Set(['big']) });
    expect(items.map((i) => i.kind)).toEqual(['floor', 'room', 'room', 'floor']);
    const head = items[3]!;
    expect(head.kind === 'floor' && head.folded && head.rooms === many.rooms.length).toBe(true);
    expect(
      items.filter((i) => i.kind === 'room').every((i) => i.kind === 'room' && !i.compact),
    ).toBe(true);
  });

  it('keeps a room compact when a filter leaves few rooms on its floor', () => {
    const items = officeItems([many], { filter: 'needs' });
    const r = items.find((i) => i.kind === 'room');
    expect(items).toHaveLength(2);
    expect(r?.kind === 'room' && r.compact && r.expanded).toBe(true);
  });
});

describe('filters', () => {
  const rooms = [
    room('a', 'needs_you'),
    room('b', 'working', 2, 'working'),
    room('c', 'queued'),
    room('d', 'background', 3, 'working'),
    Object.assign(room('e', 'needs_you'), { done: true }),
  ];
  const floors = [floor('one', rooms.slice(0, 3)), floor('two', rooms.slice(3))];

  it('counts rooms per chip', () => {
    expect(filterCounts(floors)).toEqual({ all: 5, needs: 1, working: 2 });
  });

  it('keeps the matching rooms and drops floors left empty', () => {
    expect(filterFloors(floors, 'needs').map((f) => f.rooms.map((r) => r.id))).toEqual([['a']]);
    expect(filterFloors(floors, 'working').map((f) => f.rooms.map((r) => r.id))).toEqual([
      ['b'],
      ['d'],
    ]);
    expect(filterFloors(floors, 'all')).toHaveLength(2);
    expect(rooms[0]!.id).toBe('a');
    expect(floors[0]!.rooms).toHaveLength(3);
  });

  it('does not count a finished room as asking', () => {
    expect(roomAsks(rooms[4]!)).toBe(false);
    expect(roomAsks(rooms[0]!)).toBe(true);
  });
});

describe('motion', () => {
  const on = { foreground: true, crowded: false, visible: new Set(['a']) };
  it('runs only in front, uncrowded and where the room is on screen', () => {
    expect(motionAllowed(on, 'a')).toBe(true);
    expect(motionAllowed(on, 'b')).toBe(false);
    expect(motionAllowed(on, null)).toBe(true);
    expect(motionAllowed({ ...on, foreground: false }, 'a')).toBe(false);
    expect(motionAllowed({ ...on, crowded: true }, 'a')).toBe(false);
  });

  it('counts the people drawn as seats, not the faces of a one-line row', () => {
    const rooms = Array.from({ length: 5 }, (_, i) => room(`r${i}`, 'working', 9, 'working'));
    const open = officeItems([floor('f', rooms)]);
    expect(isCrowded(open)).toBe(true); // 5 rooms x 6 seats
    expect(isCrowded(open.slice(0, 4))).toBe(false); // 3 rooms x 6 = 18
    expect(isCrowded(open.slice(0, 5))).toBe(false); // 4 rooms x 6 = 24 is not over the limit
  });
});

describe('a large office, 120 people in 36 rooms', () => {
  it('is capped, compacted and bounded', async () => {
    const path = resolve(
      dirname(fileURLToPath(import.meta.url)),
      '../../../tools/office-large.mjs',
    );
    const { largeOffice } = (await import(/* @vite-ignore */ pathToFileURL(path).href)) as any;
    const big = largeOffice(NOW);
    const { office } = buildOffice(
      { projects: big.sidebar, seats: big.seats, tasks: big.tasks, talk: big.talk(NOW) },
      newTracker(),
      NOW,
    );
    const rooms = office.floors.flatMap((f) => f.rooms);
    expect(rooms).toHaveLength(36);
    expect(rooms.reduce((n, r) => n + r.people.length, 0)).toBe(120);
    const items = officeItems(office.floors);
    const seated = items.reduce(
      (n, i) =>
        n + (i.kind === 'room' && i.expanded ? selectPeople(i.room.people).shown.length : 0),
      0,
    );
    expect(seated).toBeLessThan(120);
    expect(items.some((i) => i.kind === 'room' && i.compact && !i.expanded)).toBe(true);
    expect(items.some((i) => i.kind === 'room' && i.compact && i.expanded)).toBe(true);
    const crowd = rooms.find((r) => r.people.length === 16)!;
    const s = selectPeople(crowd.people);
    expect(s.shown).toHaveLength(ROOM_CAP);
    expect(s.more).toBe(10);
    // Every filter keeps some rooms, and the chips add up with it.
    const counts = filterCounts(office.floors);
    expect(counts.all).toBe(36);
    expect(counts.needs).toBeGreaterThan(0);
    expect(counts.working).toBeGreaterThan(counts.needs);
    expect(filterFloors(office.floors, 'needs').flatMap((f) => f.rooms)).toHaveLength(counts.needs);
  });
});
