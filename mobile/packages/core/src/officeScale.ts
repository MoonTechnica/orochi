import type { Floor, Person, PersonState, Room } from './office';

/**
 * What the Office does so that 100 people in 30 rooms stay light: at most `ROOM_CAP` people are
 * drawn in a room, a crowded floor or room becomes one line that opens in place, the list is flat
 * (so a screen can mount only what is near the viewport), motion runs only where it can be seen,
 * and the chips narrow the rooms. All of it is a function of the office and a few choices; nothing
 * here reads the screen.
 */

/** People drawn in a room; the rest are one "+N" tile. */
export const ROOM_CAP = 6;
/** A floor with more rooms than this draws them as one-line rows. */
export const COMPACT_FLOOR_ROOMS = 8;
/** A room with more people than this is a one-line row, whatever the floor. */
export const COMPACT_ROOM_PEOPLE = 12;
/** Faces on a one-line row. */
export const COMPACT_FACES = 5;
/** With more people than this on screen at once, nobody moves. */
export const MOTION_PEOPLE_LIMIT = 24;

const RANK: Record<PersonState, number> = {
  asking: 0,
  working: 1,
  mentioned: 2,
  reported: 2,
  idle: 3,
  leaving: 3,
};
export const PERSON_STATES: PersonState[] = [
  'asking',
  'working',
  'mentioned',
  'reported',
  'idle',
  'leaving',
];

export type StateCounts = Record<PersonState, number>;
const zero = (): StateCounts => ({
  asking: 0,
  working: 0,
  mentioned: 0,
  reported: 0,
  idle: 0,
  leaving: 0,
});

export interface Selection {
  /** Who is drawn, in the room's own order (the lead first). */
  shown: Person[];
  /** Everyone else, in the room's own order. */
  hidden: Person[];
  /** How many are behind the "+N" tile. */
  more: number;
  /** What the hidden are doing. */
  summary: StateCounts;
}

export const countStates = (people: readonly Person[]): StateCounts => {
  const n = zero();
  for (const p of people) n[p.state]++;
  return n;
};

/**
 * Who is drawn when there is room for `cap`: the ones asking first, then working, then the ones
 * with a message or a report, then idle and leaving; within a state the room's own order. The
 * drawn ones keep that order too, so a change of state moves a person in or out and never shuffles
 * the others.
 */
export function selectPeople(people: readonly Person[], cap = ROOM_CAP): Selection {
  if (people.length <= cap) return { shown: [...people], hidden: [], more: 0, summary: zero() };
  const order = people
    .map((p, i) => ({ i, rank: RANK[p.state] }))
    .sort((a, b) => a.rank - b.rank || a.i - b.i);
  const keep = new Set(order.slice(0, cap).map((o) => o.i));
  const shown = people.filter((_, i) => keep.has(i));
  const hidden = people.filter((_, i) => !keep.has(i));
  return { shown, hidden, more: hidden.length, summary: countStates(hidden) };
}

/** Everyone in a room for the full list, the ones that need attention first. */
export const listPeople = (people: readonly Person[]): Person[] =>
  people
    .map((p, i) => ({ p, i }))
    .sort((a, b) => RANK[a.p.state] - RANK[b.p.state] || a.i - b.i)
    .map((o) => o.p);

/** The faces on a one-line row: the same pick as a room's, and how many are left out. */
export function faceRow(people: readonly Person[], faces = COMPACT_FACES) {
  const s = selectPeople(people, faces);
  return { faces: s.shown, more: s.more };
}

// Rooms ----------------------------------------------------------------------------------

/** A room with a question for the person. */
export const roomAsks = (room: Room): boolean =>
  !room.done && !room.closing && (room.thread.status === 'needs_you' || room.thread.asking > 0);

export const isCompact = (room: Room, roomsOnFloor: number): boolean =>
  roomsOnFloor > COMPACT_FLOOR_ROOMS || room.people.length > COMPACT_ROOM_PEOPLE;

export type OfficeFilter = 'all' | 'needs' | 'working';
export const FILTERS: OfficeFilter[] = ['all', 'needs', 'working'];

export function roomMatches(room: Room, filter: OfficeFilter): boolean {
  if (filter === 'all') return true;
  if (filter === 'needs') return roomAsks(room);
  return (
    !room.done &&
    !room.closing &&
    (room.thread.status === 'working' || room.people.some((p) => p.state === 'working'))
  );
}

/** The floors with only the rooms the filter lets through; a floor with none is left out. */
export function filterFloors(floors: readonly Floor[], filter: OfficeFilter): Floor[] {
  if (filter === 'all') return [...floors];
  return floors
    .map((f) => ({ ...f, rooms: f.rooms.filter((r) => roomMatches(r, filter)) }))
    .filter((f) => f.rooms.length > 0);
}

/** Rooms per filter, for the chips. */
export function filterCounts(floors: readonly Floor[]): Record<OfficeFilter, number> {
  const out: Record<OfficeFilter, number> = { all: 0, needs: 0, working: 0 };
  for (const f of floors)
    for (const r of f.rooms) {
      out.all++;
      if (roomMatches(r, 'needs')) out.needs++;
      if (roomMatches(r, 'working')) out.working++;
    }
  return out;
}

// The flat list ----------------------------------------------------------------------------

export type OfficeItem =
  | { kind: 'floor'; key: string; floor: string; rooms: number; asking: boolean; folded: boolean }
  | {
      kind: 'room';
      key: string;
      floor: string;
      room: Room;
      /** Drawn as a one-line row when not `expanded`. */
      compact: boolean;
      expanded: boolean;
    };

export interface ItemChoices {
  filter?: OfficeFilter;
  /** Floors the person folded. */
  folded?: ReadonlySet<string>;
  /** The one compact room the person opened on a floor (an accordion). */
  picked?: Readonly<Record<string, string>>;
  /** Rooms that ask but that the person closed. */
  closed?: ReadonlySet<string>;
}

/**
 * The office as one flat list, floor headers and room rows, so a list can mount what is near the
 * viewport and nothing else. A room is compact by the size of its floor and of itself before the
 * filter (narrowing must not redraw the rooms that stay); a compact room is open when it asks (until
 * the person closes it) or is the one picked on its floor.
 */
export function officeItems(floors: readonly Floor[], c: ItemChoices = {}): OfficeItem[] {
  const filter = c.filter ?? 'all';
  const items: OfficeItem[] = [];
  for (const floor of floors) {
    const rooms = floor.rooms.filter((r) => roomMatches(r, filter));
    if (!rooms.length) continue;
    const folded = c.folded?.has(floor.name) ?? false;
    items.push({
      kind: 'floor',
      key: `floor:${floor.name}`,
      floor: floor.name,
      rooms: rooms.length,
      asking: rooms.some(roomAsks),
      folded,
    });
    if (folded) continue;
    for (const room of rooms) {
      const compact = isCompact(room, floor.rooms.length);
      const expanded =
        !compact || (roomAsks(room) ? !c.closed?.has(room.id) : c.picked?.[floor.name] === room.id);
      items.push({
        kind: 'room',
        key: `room:${room.id}`,
        floor: floor.name,
        room,
        compact,
        expanded,
      });
    }
  }
  return items;
}

// Motion --------------------------------------------------------------------------------

export interface MotionState {
  foreground: boolean;
  /** More people on screen than `MOTION_PEOPLE_LIMIT`. */
  crowded: boolean;
  /** Rooms the list reports as on screen. */
  visible: ReadonlySet<string>;
}

/** People moving: only while the app is in front, the screen is not crowded, and the room is seen. */
export const motionAllowed = (s: MotionState, room: string | null): boolean =>
  s.foreground && !s.crowded && (room === null || s.visible.has(room));

/** People drawn as seats in an item (a one-line row draws faces that never move). */
export const seatsDrawn = (item: OfficeItem): number =>
  item.kind === 'room' && item.expanded ? Math.min(item.room.people.length, ROOM_CAP) : 0;

export const isCrowded = (items: readonly OfficeItem[]): boolean =>
  items.reduce((n, i) => n + seatsDrawn(i), 0) > MOTION_PEOPLE_LIMIT;
