import type { AgentUse, LiveSeat, LiveTask, ProjectRow, Talk, ThreadRow } from '@orochi/protocol';

/**
 * The desktop's Office (desktop/src/app.js, "Office ---") as data: a project is a floor, a
 * conversation a room, a member of its team someone in it. Everything here is read from what the
 * sidebar, `live_seats`, `live_tasks` and `chatter` already carry. The rules are the desktop's;
 * only the drawing differs (rooms stack, cross-room wires become lines in the room).
 */

export const ROOM_ORDER: Record<string, number> = {
  needs_you: 0,
  working: 1,
  queued: 2,
  background: 3,
  held: 4,
};
export const CHATTER_MS = 5 * 60 * 1000;
/** A message is news for this long: the speaker's mouth moves and its envelope hops once. */
export const FRESH_MS = 8 * 1000;
/** An envelope waits on the one it was for until it is opened or this long has passed. */
export const UNREAD_MS = 60 * 1000;
export const WRAP_UP_MS = 60 * 1000;
export const LEAVE_MS = 2400;
export const ARRIVE_MS = 900;
export const CLOSE_MS = 1300;

export type OfficeThread = ThreadRow & { where: string };
export type PersonState = 'working' | 'asking' | 'mentioned' | 'reported' | 'idle' | 'leaving';

export const lowered = (value: unknown) => String(value ?? '').toLowerCase();
export const sameRole = (a?: string | null, b?: string | null) =>
  Boolean(a) && Boolean(b) && lowered(a) === lowered(b);

/** The conversation's members; an old row with only an agent is its lead. */
export function membersOf(thread: ThreadRow): AgentUse[] {
  if (thread.agents?.length) return thread.agents;
  return thread.agent
    ? [
        {
          role: '',
          agent: thread.agent,
          provider: thread.provider ?? '',
          model: '',
          seats: 1,
          live: 0,
          lead: true,
          background: false,
          last_at: 0,
        },
      ]
    : [];
}

/** Everyone a message is for: the seat it was sent to and whoever its text mentions. */
export function addressees(line: Talk): { thread: string; role: string }[] {
  const out: { thread: string; role: string }[] = [];
  if (line.to_thread && line.to_role) out.push({ thread: line.to_thread, role: line.to_role });
  for (const name of line.mentions || []) {
    if (['you', 'channel'].includes(lowered(name)) || !line.from_thread) continue;
    if (!out.some((o) => o.thread === line.from_thread && sameRole(o.role, name)))
      out.push({ thread: line.from_thread, role: name });
  }
  return out;
}

/** The same hue for the same agent every time. */
export function hueOf(m: { role?: string | null; agent?: string | null }): number {
  const seed = [...`${m.role || ''}|${m.agent || ''}`].reduce(
    (n, c) => (n * 31 + c.charCodeAt(0)) >>> 0,
    7,
  );
  return (seed * 137) % 360;
}

/** Hair (0 to 4) and accessory (0 to 3: glasses, headset, antenna, none) of a character. */
export function lookOf(m: { role?: string | null; agent?: string | null }) {
  const seed = [...`${m.role || ''}|${m.agent || ''}`].reduce(
    (n, c) => (n * 31 + c.charCodeAt(0)) >>> 0,
    7,
  );
  return { hair: seed % 5, extra: (seed >> 4) % 4 };
}

const clip = (value: string, n: number) => (value.length > n ? `${value.slice(0, n - 1)}…` : value);

// Tracking ------------------------------------------------------------------------------

interface Seen {
  thread: OfficeThread;
  live: Set<string>;
  roles: Set<string>;
  lead: Set<string>;
}
export interface Wrap {
  thread: OfficeThread;
  kind: 'done' | 'closing';
  until: number;
}
/** What changed since the last look; each step returns a new one and never edits the old. */
export interface OfficeTracker {
  last: Map<string, Seen> | null;
  leaving: Map<string, number>;
  hidden: Set<string>;
  fresh: Map<string, number>;
  wrap: Map<string, Wrap>;
  /** The time the next arrival, departure or wrap-up ends (0: none). */
  until: number;
}
export const newTracker = (): OfficeTracker => ({
  last: null,
  leaving: new Map(),
  hidden: new Set(),
  fresh: new Map(),
  wrap: new Map(),
  until: 0,
});

/**
 * A room opening, an agent that finished leaving, a conversation that ended staying a minute as
 * done, one that was deleted closing (desktop `stepOffice`). The first look draws everything at rest.
 */
export function stepOffice(
  prev: OfficeTracker,
  all: OfficeThread[],
  active: OfficeThread[],
  now: number,
): OfficeTracker {
  if (!prev.last && !all.length) return prev;
  const t: OfficeTracker = {
    last: prev.last,
    leaving: new Map(prev.leaving),
    hidden: new Set(prev.hidden),
    fresh: new Map(prev.fresh),
    wrap: new Map(prev.wrap),
    until: prev.until,
  };
  const cur = new Map<string, Seen>(
    active.map((th) => {
      const ms = membersOf(th);
      return [
        th.id,
        {
          thread: th,
          live: new Set(ms.filter((m) => m.live > 0).map((m) => lowered(m.role))),
          roles: new Set(ms.map((m) => lowered(m.role))),
          lead: new Set(ms.filter((m) => m.lead).map((m) => lowered(m.role))),
        },
      ];
    }),
  );
  if (prev.last) {
    for (const [id, was] of prev.last) {
      const is = cur.get(id);
      if (is) {
        for (const role of was.live)
          if (!is.live.has(role) && !was.lead.has(role))
            t.leaving.set(`${id}:${role}`, now + LEAVE_MS);
        for (const role of is.live) {
          t.leaving.delete(`${id}:${role}`);
          t.hidden.delete(`${id}:${role}`);
        }
        for (const role of is.roles)
          if (!was.roles.has(role)) t.fresh.set(`${id}:${role}`, now + ARRIVE_MS);
      } else if (!t.wrap.has(id)) {
        const still = all.find((th) => th.id === id);
        t.wrap.set(
          id,
          still
            ? { thread: was.thread, kind: 'done', until: now + WRAP_UP_MS }
            : { thread: was.thread, kind: 'closing', until: now + CLOSE_MS },
        );
      }
    }
    for (const id of cur.keys()) {
      if (!prev.last.has(id)) t.fresh.set(id, now + ARRIVE_MS);
      t.wrap.delete(id);
    }
  }
  t.last = cur;
  for (const [id, wrap] of t.wrap) {
    if (wrap.kind === 'done' && !all.some((th) => th.id === id))
      t.wrap.set(id, { ...wrap, kind: 'closing', until: Math.min(wrap.until, now + CLOSE_MS) });
    else if (wrap.until <= now) t.wrap.delete(id);
  }
  for (const [key, until] of t.leaving)
    if (until <= now) {
      t.leaving.delete(key);
      t.hidden.add(key);
    }
  for (const [key, until] of t.fresh) if (until <= now) t.fresh.delete(key);
  const untils = [...t.wrap.values()]
    .map((w) => w.until)
    .concat([...t.leaving.values(), ...t.fresh.values()]);
  t.until = untils.length ? Math.min(...untils) : 0;
  return t;
}

// The office ----------------------------------------------------------------------------

export interface OfficeTask {
  id: number;
  title: string;
  doing: string;
  calls: number;
  startedAt: number;
}

export interface Bubble {
  kind: 'ask' | 'talk' | 'work';
  /** Empty for the app's own wording ("Needs you", "working…"), which the screen translates. */
  text: string;
  /** For a message: who it is for; null is everyone. */
  to: string | null;
  /** The title of another conversation's room the message goes to. */
  away: string;
}

export interface Person {
  key: string;
  thread: string;
  role: string;
  lead: boolean;
  /** The member that hired this one, if the lead is not. */
  manager: string | null;
  depth: number;
  agent: string;
  provider: string;
  model: string;
  background: boolean;
  live: number;
  state: PersonState;
  seatId: string | null;
  readOnly: boolean;
  /** What it is doing, first line, as the desk says it. */
  doing: string;
  startedAt: number | null;
  tasks: OfficeTask[];
  bubble: Bubble | null;
  /** It spoke a moment ago: its mouth moves. */
  speaking: boolean;
  /** An unopened message addressed to it. */
  mail: Talk | null;
  arriving: boolean;
  hue: number;
  hair: number;
  extra: number;
}

export interface RoomLine {
  seq: number;
  from: string;
  to: string | null;
  text: string;
  at: number;
  fresh: boolean;
  /** The other conversation's title when the line crosses rooms. */
  across: string;
  direction: 'in' | 'out' | 'within';
}

export interface Room {
  id: string;
  title: string;
  project: string;
  thread: OfficeThread;
  status: string;
  done: boolean;
  closing: boolean;
  arriving: boolean;
  since: number | null;
  people: Person[];
  lines: RoomLine[];
}

export interface Floor {
  name: string;
  rooms: Room[];
}

export interface OfficeStats {
  working: number;
  assistants: number;
  asking: number;
  messages: number;
  rooms: number;
}

export interface Office {
  floors: Floor[];
  stats: OfficeStats;
  quiet: boolean;
  /** When the picture changes by itself next (an arrival ends, an envelope goes stale); 0 for never. */
  nextChange: number;
}

export interface OfficeInput {
  projects: ProjectRow[];
  seats: LiveSeat[];
  tasks: LiveTask[];
  talk: Talk[];
  /** Messages whose envelope the person has opened. */
  seen?: ReadonlySet<number>;
}

/** The lead on top, each member under the lead that hired it, flattened depth first (`chart`). */
export function chart(thread: ThreadRow): { member: AgentUse; depth: number }[] {
  const all = membersOf(thread);
  const heads = all.filter((m) => m.lead && !m.manager);
  const root = heads[0];
  const under = new Map<AgentUse, AgentUse[]>();
  const roots: AgentUse[] = [];
  const taken = new Set<AgentUse>();
  for (const m of all) {
    let boss = m.manager ? all.find((o) => o !== m && sameRole(o.role, m.manager)) : undefined;
    if (!boss && !heads.includes(m) && root) boss = root;
    if (boss) {
      if (!under.has(boss)) under.set(boss, []);
      under.get(boss)!.push(m);
    } else roots.push(m);
  }
  const out: { member: AgentUse; depth: number }[] = [];
  const walk = (m: AgentUse, depth: number, path: Set<AgentUse>) => {
    taken.add(m);
    out.push({ member: m, depth });
    for (const k of under.get(m) ?? []) if (!path.has(k)) walk(k, depth + 1, new Set([...path, k]));
  };
  for (const m of roots) walk(m, 0, new Set([m]));
  for (const m of all) if (!taken.has(m)) walk(m, 0, new Set([m]));
  return out;
}

const stripTask = (s: string) => s.replace(/^Task:\s*/, '');

/**
 * The office as the desktop draws it: rooms for the conversations with something going on, grouped
 * by project, the ones that need the person first. Pure: the tracker in is the tracker out, advanced.
 */
export function buildOffice(
  input: OfficeInput,
  prev: OfficeTracker,
  now: number,
): { office: Office; tracker: OfficeTracker } {
  const seen = input.seen ?? new Set<number>();
  const all: OfficeThread[] = input.projects.flatMap((p) =>
    p.threads.map((th) => ({ ...th, where: p.name })),
  );
  const busy = (th: ThreadRow) => th.status in ROOM_ORDER || membersOf(th).some((m) => m.live > 0);
  const active = all.filter(busy);
  const tracker = stepOffice(prev, all, active, now);
  const wrapped = [...tracker.wrap].map(([id, w]) => {
    const now_ = all.find((th) => th.id === id);
    return { thread: now_ ?? w.thread, kind: w.kind as 'done' | 'closing' };
  });
  const titleOf = (id: string) => all.find((th) => th.id === id)?.title ?? '';
  const talk = input.talk;

  const make = (th: OfficeThread, wrap: 'done' | 'closing' | null): Room => {
    const finished = wrap === 'done';
    const people: Person[] = [];
    for (const { member: m, depth } of chart(th)) {
      const seat = input.seats.find((s) => s.thread_id === th.id && s.role === m.role);
      const working = m.live > 0;
      const asking = th.status === 'needs_you' && m.lead;
      const key = `${th.id}:${lowered(m.role)}`;
      const leaving = tracker.leaving.has(key);
      if (!working && !leaving && !m.lead && tracker.hidden.has(key)) continue;
      const what = stripTask((seat?.doing || seat?.title || '').split('\n')[0]);
      const said = finished
        ? undefined
        : talk.find((l) => l.from_thread === th.id && sameRole(l.from_role, m.role));
      const heard = finished
        ? undefined
        : talk.find(
            (l) =>
              addressees(l).some((o) => o.thread === th.id && sameRole(o.role, m.role)) &&
              l !== said,
          );
      const unopened =
        heard && heard.read !== true && now - heard.at < UNREAD_MS && !seen.has(heard.seq)
          ? heard
          : null;
      const speaking = Boolean(said && now - said.at < FRESH_MS);
      let bubble: Bubble | null = null;
      if (asking) bubble = { kind: 'ask', text: '', to: null, away: '' };
      else if (said) {
        const to = addressees(said)[0];
        const away = to && to.thread !== th.id ? titleOf(to.thread) : '';
        const who = to?.role || (said.to && said.to !== 'channel' ? said.to : null);
        bubble = { kind: 'talk', text: clip(said.text, 90), to: who, away: clip(away, 18) };
      } else if (working) bubble = { kind: 'work', text: clip(what, 64), to: null, away: '' };
      const state: PersonState = leaving
        ? 'leaving'
        : asking
          ? 'asking'
          : working
            ? 'working'
            : unopened
              ? 'mentioned'
              : said
                ? 'reported'
                : 'idle';
      const look = lookOf(m);
      people.push({
        key,
        thread: th.id,
        role: m.role,
        lead: m.lead,
        manager: m.manager ?? null,
        depth,
        agent: m.agent,
        provider: m.provider,
        model: m.model,
        background: m.background,
        live: m.live,
        state,
        seatId: seat?.seat_id ?? null,
        readOnly: Boolean(seat?.read_only),
        doing: what,
        startedAt: working && seat ? seat.started_at : null,
        tasks: seat
          ? input.tasks
              .filter((k) => k.seat_id === seat.seat_id)
              .map((k) => ({
                id: k.item_id,
                title: stripTask(k.title ?? ''),
                doing: (k.doing ?? '').split('\n')[0],
                calls: k.calls,
                startedAt: k.started_at,
              }))
          : [],
        bubble,
        speaking,
        mail: unopened,
        arriving: tracker.fresh.has(key),
        hue: hueOf(m),
        hair: look.hair,
        extra: look.extra,
      });
    }
    const lines: RoomLine[] = [];
    if (!finished)
      for (const l of talk.slice(0, 8)) {
        const to = addressees(l);
        const inRoom = l.from_thread === th.id;
        const toRoom = to.some((o) => o.thread === th.id);
        if (!inRoom && !toRoom) continue;
        const sender = people.some(
          (p) => p.thread === l.from_thread && sameRole(p.role, l.from_role),
        );
        if (inRoom && !sender && !toRoom) continue;
        const other = inRoom ? to.find((o) => o.thread !== th.id) : { thread: l.from_thread ?? '' };
        lines.push({
          seq: l.seq,
          from: l.from_role || l.from,
          to: to[0]?.role || (l.to && l.to !== 'channel' ? l.to : null),
          text: l.text,
          at: l.at,
          fresh: now - l.at < FRESH_MS,
          across: other && other.thread && other.thread !== th.id ? titleOf(other.thread) : '',
          direction: inRoom ? (other?.thread && other.thread !== th.id ? 'out' : 'within') : 'in',
        });
      }
    return {
      id: th.id,
      title: th.title,
      project: th.where,
      thread: th,
      status: wrap === 'done' ? 'done' : th.status,
      done: finished,
      closing: wrap === 'closing',
      arriving: tracker.fresh.has(th.id),
      since: th.running_since ?? th.updated_at ?? null,
      people,
      lines: lines.slice(0, 3),
    };
  };

  const rooms = [
    ...active.map((th) => make(th, null)),
    ...wrapped.map((w) => make(w.thread as OfficeThread, w.kind)),
  ];
  const rank = (r: Room) => (r.done || r.closing ? 6 : (ROOM_ORDER[r.thread.status] ?? 5));
  const byFloor = new Map<string, Room[]>();
  for (const r of rooms) {
    if (!byFloor.has(r.project)) byFloor.set(r.project, []);
    byFloor.get(r.project)!.push(r);
  }
  const floors: Floor[] = [...byFloor]
    .sort((a, b) => Math.min(...a[1].map(rank)) - Math.min(...b[1].map(rank)))
    .map(([name, rs]) => ({ name, rooms: [...rs].sort((a, b) => rank(a) - rank(b)) }));

  const seated = new Set<string>();
  for (const r of rooms) for (const p of r.people) seated.add(p.key);
  const stats: OfficeStats = {
    working: input.seats.length,
    assistants: input.tasks.length,
    asking: all.reduce((n, th) => n + (th.asking > 0 ? 1 : 0), 0),
    messages: talk.filter((l) => seated.has(`${l.from_thread}:${lowered(l.from_role)}`)).length,
    rooms: active.length,
  };

  let next = tracker.until > now ? tracker.until : 0;
  const soonest = (at: number) => {
    if (at > now && (!next || at < next)) next = at;
  };
  for (const l of talk) {
    soonest(l.at + FRESH_MS);
    soonest(l.at + UNREAD_MS);
  }
  return {
    office: { floors, stats, quiet: rooms.length === 0, nextChange: next },
    tracker,
  };
}

/** The numbers and the first few faces, for a card on Home; nothing is tracked. */
export function officeSummary(input: OfficeInput, now: number) {
  const { office } = buildOffice({ ...input, talk: [] }, newTracker(), now);
  const people = office.floors.flatMap((f) => f.rooms.flatMap((r) => r.people));
  const order: Record<PersonState, number> = {
    asking: 0,
    working: 1,
    mentioned: 2,
    reported: 3,
    idle: 4,
    leaving: 5,
  };
  return {
    stats: office.stats,
    faces: [...people].sort((a, b) => order[a.state] - order[b.state]).slice(0, 6),
  };
}
