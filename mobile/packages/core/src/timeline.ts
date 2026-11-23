import type {
  ItemRow,
  LiveSeat,
  LiveTask,
  MediaData,
  Member,
  Prompt,
  Said,
  Thread,
} from '@orochi/protocol';
import { memberState, type MemberState } from './status';
import { toMs } from './time';

const lowered = (v: unknown) => String(v ?? '').toLowerCase();

export interface Voice {
  who: string;
  to: string | null;
  role: string | null;
  agent: string | null;
  provider: string | null;
  model: string | null;
  at: number | null;
  text: string;
  mine: boolean;
  mentions: string[];
  mentionsYou: boolean;
  /** Also sent to the channel from a member's thread (`broadcast`). */
  broadcast?: boolean;
  /** The member whose thread this post belongs to (an assignment, or a reply also sent to the channel). */
  thread?: string | null;
}

export interface BarData {
  name: string;
  agent: string | null;
  provider: string | null;
  faces: { agent: string; provider: string | null }[];
  replies: number;
  lastAt: number | null;
  state: MemberState;
  needs: string | null;
}

export type Entry =
  | { type: 'post'; key: string; voice: Voice; run: boolean; turn: string | null }
  | { type: 'bar'; key: string; bar: BarData }
  | { type: 'route'; key: string; text: string; resumed: boolean }
  | { type: 'work'; key: string; items: ItemRow[]; tools: number; thoughts: number }
  | { type: 'aside'; key: string; notes: ItemRow[]; label: string | null }
  | {
      type: 'checks';
      key: string;
      outcome: string | null;
      checks: { name: string; passed: boolean; tail?: string }[];
    }
  | { type: 'media'; key: string; media: MediaData; at: number }
  | { type: 'reports'; key: string; origin: string; names: string[] }
  | { type: 'foot'; key: string; ms: number; endedAt: number; files: number }
  | { type: 'live'; key: string; seats: { role: string; what: string }[]; starting: boolean }
  | { type: 'ask'; key: string; prompt: Prompt }
  | { type: 'day'; key: string; at: number };

export interface TimelineInput {
  thread: Thread;
  said: Said[];
  members: Member[];
  prompts?: Prompt[];
  live?: LiveSeat[];
}

export const named = (line: { role?: string | null; who: string }) => line.role || line.who;

function barFor(name: string, said: Said[], members: Member[]): BarData {
  const key = lowered(name);
  const m = members.find((x) => lowered(x.name) === key);
  const rows = said.filter(
    (l) => l.kind === 'message' && lowered(l.member) === key && l.via !== 'handoff',
  );
  const faces: BarData['faces'] = [];
  const seen = new Set<string>();
  if (m?.agent) {
    faces.push({ agent: m.agent, provider: m.provider });
    seen.add(`${m.agent}/${m.provider}`);
  }
  for (const r of rows) {
    const id = `${r.agent}/${r.provider}`;
    if (!r.agent || seen.has(id)) continue;
    seen.add(id);
    faces.push({ agent: r.agent, provider: r.provider });
  }
  return {
    name,
    agent: m?.agent ?? null,
    provider: m?.provider ?? null,
    faces,
    replies: m ? m.replies : rows.length,
    lastAt: m?.last_reply_at ? toMs(m.last_reply_at) : (rows.at(-1)?.at ?? null),
    state: memberState(m),
    needs: m?.needs ?? null,
  };
}

/** Which names a completion turn's text carries reports from. */
export function reportNames(body: string, said: Said[], members: Member[]): string[] {
  const names = new Map<string, string>();
  for (const m of members) names.set(lowered(m.name), m.name);
  for (const l of said) if (l.via === 'handoff' && l.member) names.set(lowered(l.member), l.member);
  return [...names.entries()]
    .filter(([key]) =>
      new RegExp(`(^|[^\\w])${key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}($|[^\\w])`, 'i').test(
        body || '',
      ),
    )
    .map(([, name]) => name);
}

/**
 * The channel as the desktop draws it (`drawTimeline`): the person's messages, the lead's
 * replies, one assignment per member with a bar under it, and what members posted to the channel.
 * A member's own work and replies belong to its thread and are not in the body.
 */
export function buildTimeline(input: TimelineInput): Entry[] {
  const { thread, said, members } = input;
  const out: Entry[] = [];
  // The turn each entry belongs to, parallel to `out`; turns that overlap in time are regrouped
  // at the end so a turn's footer lands under that turn's own entries.
  const tags: (string | null)[] = [];
  let tagNow: string | null = null;
  const push = (entry: Entry, turn: string | null = tagNow) => {
    out.push(entry);
    tags.push(turn);
  };
  let pending: ItemRow[] = [];
  let notes: ItemRow[] = [];
  const turnFirst = new Map<string, ItemRow>();
  const flush = () => {
    if (pending.length) {
      push(
        {
          type: 'work',
          key: `w${pending[0]!.seq}`,
          items: pending,
          tools: pending.filter((i) => i.kind === 'tool_call').length,
          thoughts: pending.filter((i) => i.kind === 'thought').length,
        },
        pending[0]!.turn,
      );
    }
    pending = [];
    if (notes.length)
      push({ type: 'aside', key: `a${notes[0]!.seq}`, notes, label: null }, notes[0]!.turn);
    notes = [];
  };

  const spoken = said
    .filter((l) => l.kind === 'message')
    .map((l) => ({ ...l, k: 'said' as const }));
  const handedOff = new Set(
    spoken.filter((l) => l.via === 'handoff' && l.member).map((l) => lowered(l.member)),
  );
  const roles = new Map(said.map((l) => [l.who, l.role] as const).filter(([, r]) => r));
  const barred = new Set<string>();
  const bar = (name: string) => {
    if (barred.has(lowered(name))) return;
    barred.add(lowered(name));
    push({ type: 'bar', key: `bar-${lowered(name)}`, bar: barFor(name, said, members) });
  };

  const stream = [...thread.items.map((i) => ({ ...i, k: 'item' as const })), ...spoken].sort(
    (a, b) => (a.at ?? 0) - (b.at ?? 0) || (a.seq ?? 0) - (b.seq ?? 0),
  );

  let voice: string | null = null;
  const post = (v: Voice, run: boolean, key: string) =>
    push({ type: 'post', key, voice: v, run, turn: tagNow });

  for (const item of stream) {
    if (item.k === 'item') {
      if (item.kind === 'user_message') tagNow = item.turn;
      else if (!item.member && !(item.lane != null && item.lane > 0) && item.turn) {
        if (pending.length && pending[0]!.turn !== item.turn) flush();
        if (notes.length && notes[0]!.turn !== item.turn) flush();
        tagNow = item.turn;
      }
    }
    if (item.k === 'said') {
      const key = item.member ? lowered(item.member) : null;
      if (key && item.via === 'handoff' && barred.has(key)) continue;
      if (key && item.via !== 'handoff' && !item.broadcast) {
        if (!handedOff.has(key) && !barred.has(key)) {
          flush();
          bar(item.member!);
          voice = null;
        }
        continue;
      }
      flush();
      const who = named(item);
      const to =
        item.via === 'handoff'
          ? null
          : !item.whom || item.whom === 'all'
            ? 'everyone'
            : roles.get(item.whom) || item.whom;
      post(
        {
          who,
          to,
          role: item.role,
          agent: item.agent,
          provider: item.provider,
          model: item.model,
          at: item.at,
          text: item.text,
          mine: item.via === 'user',
          mentions: item.mentions,
          mentionsYou: item.mentions.includes('you'),
          broadcast: Boolean(item.broadcast && item.member),
          thread: item.member ?? null,
        },
        voice === who && item.via !== 'handoff',
        `s${item.seq}`,
      );
      voice = who;
      if (key && (item.via === 'handoff' || !handedOff.has(key))) {
        bar(item.member!);
        voice = null;
      }
      continue;
    }
    // A member's own working notes are its thread's.
    if (item.member || (item.lane !== null && item.lane !== undefined && item.lane > 0)) continue;
    if (item.kind === 'user_message') {
      flush();
      turnFirst.set(item.turn, item);
      const origin = item.turn_origin;
      // A routine's wake is Orochi's text, not something anyone said, and nobody needs to be
      // told it woke: what it produced is the reply under it.
      if (origin === 'routine') {
        voice = null;
        continue;
      }
      if (origin === 'background' || origin === 'merge' || origin === 'mail') {
        push({
          type: 'reports',
          key: `r${item.seq}`,
          origin,
          names: reportNames(item.text, said, members),
        });
        voice = null;
        continue;
      }
      post(
        {
          who: 'you',
          to: null,
          role: null,
          agent: null,
          provider: null,
          model: null,
          at: item.at,
          text: item.text,
          mine: true,
          mentions: [],
          mentionsYou: false,
        },
        false,
        `u${item.seq}`,
      );
      voice = 'you';
      continue;
    }
    if (item.kind === 'thought' || item.kind === 'tool_call' || item.kind === 'plan') {
      pending.push(item);
      continue;
    }
    if (item.kind === 'note' || item.kind === 'unavailable') {
      notes.push(item);
      continue;
    }
    if (item.failed && item.text) {
      flush();
      push({
        type: 'aside',
        key: `f${item.seq}`,
        notes: [item],
        label: 'an attempt that failed, and what it said',
      });
      voice = null;
      continue;
    }
    flush();
    if (item.kind === 'route') {
      const d = item.data || {};
      push({
        type: 'route',
        key: `ro${item.seq}`,
        text: [d.agent, d.model, d.reasoning].filter(Boolean).join(' · '),
        resumed: !!d.resumed,
      });
    } else if (item.kind === 'agent_message') {
      const who = item.role || item.agent || 'agent';
      post(
        {
          who,
          to: null,
          role: item.role,
          agent: item.agent,
          provider: item.provider,
          model: item.model,
          at: item.at,
          text: item.text,
          mine: false,
          mentions: [],
          mentionsYou: false,
        },
        voice === who,
        `m${item.seq}`,
      );
      voice = who;
    } else if (item.kind === 'checks') {
      const d = item.data || {};
      push({
        type: 'checks',
        key: `c${item.seq}`,
        outcome: d.outcome ?? null,
        checks: d.checks || [],
      });
    } else if (item.kind === 'media' && item.data?.sha) {
      push({ type: 'media', key: `md${item.seq}`, media: item.data as MediaData, at: item.at });
    }
  }
  flush();

  // Regroup by turn (first appearance orders the turns; untagged entries stay with the turn
  // before them), then put each finished turn's footer under its own entries.
  let carry: string | null = null;
  const eff = tags.map((t) => (t ? (carry = t) : carry));
  const order: (string | null)[] = [];
  for (const t of eff) if (!order.includes(t)) order.push(t);
  const grouped: Entry[] = [];
  const running = thread.thread.status === 'working' ? thread.items.at(-1)?.turn : null;
  for (const id of order) {
    out.forEach((e, i) => {
      if (eff[i] === id) grouped.push(e);
    });
    const first = id ? turnFirst.get(id) : null;
    if (!id || !first || id === running) continue;
    const own = thread.items.filter((i) => i.turn === id);
    if (own.length < 2) continue;
    grouped.push({
      type: 'foot',
      key: `ft${id}`,
      ms: own.at(-1)!.at - first.at,
      endedAt: own.at(-1)!.at,
      files: thread.files.filter((f) => f.turn === id).length,
    });
  }
  out.length = 0;
  out.push(...grouped);

  if (thread.thread.status === 'working') {
    const lastTurn = thread.seats.at(-1)?.turn;
    const room = thread.seats.filter((s) => s.turn === lastTurn);
    const seat = room.length ? room.find((s) => !s.background) || room[0] : null;
    out.push({
      type: 'live',
      key: 'live',
      starting: !seat,
      seats: seat
        ? [{ role: seat.role, what: seat.peer_status || seat.doing?.split('\n')[0] || seat.state }]
        : [],
    });
  }
  for (const p of (input.prompts ?? []).filter((p) => p.thread_id === thread.thread.id)) {
    out.push({ type: 'ask', key: `ask${p.id}`, prompt: p });
  }
  return out;
}

/** The thread panel: assignment, replies and the member's own work in time order. */
export type ThreadLine =
  | { type: 'parent'; key: string; voice: Voice }
  | { type: 'post'; key: string; voice: Voice; run: boolean; report: boolean }
  | { type: 'work'; key: string; items: ItemRow[]; tools: number; thoughts: number }
  | { type: 'divider'; key: string; at: number; where: string | null }
  | { type: 'day'; key: string; at: number };

export function buildMemberThread(input: TimelineInput & { name: string }): {
  head: BarData;
  lines: ThreadLine[];
} {
  const { thread, said, members, name } = input;
  const key = lowered(name);
  const mine = (row: { member?: string | null }) => lowered(row.member) === key;
  const messages = said.filter((l) => l.kind === 'message');
  const handoffs = messages.filter((l) => l.via === 'handoff' && mine(l));
  const m = members.find((x) => lowered(x.name) === key);
  const lead = (thread.thread.agents || []).find((a) => a.lead);
  const asVoice = (l: Said, extra: Partial<Voice> = {}): Voice => ({
    who: named(l),
    to: null,
    role: l.role,
    agent: l.agent ?? lead?.agent ?? null,
    provider: l.provider ?? lead?.provider ?? null,
    model: l.model,
    at: l.at,
    text: l.text,
    mine: l.via === 'user',
    mentions: l.mentions,
    mentionsYou: l.mentions.includes('you'),
    broadcast: l.broadcast,
    thread: l.broadcast ? name : null,
    ...extra,
  });
  const lines: ThreadLine[] = [];
  const parent = handoffs[0];
  if (parent) lines.push({ type: 'parent', key: `p${parent.seq}`, voice: asVoice(parent) });
  else if (m?.title)
    lines.push({
      type: 'parent',
      key: 'p-title',
      voice: {
        who: 'lead',
        to: null,
        role: 'lead',
        agent: lead?.agent ?? null,
        provider: lead?.provider ?? null,
        model: null,
        at: null,
        text: `@${name} ${m.title}`,
        mine: false,
        mentions: [name],
        mentionsYou: false,
      },
    });

  type Raw = (Said & { k: 'said' }) | (Said & { k: 'again' }) | (ItemRow & { k: 'item' });
  const raw: Raw[] = [
    ...messages
      .filter((l) => mine(l) && l.via !== 'handoff')
      .map((l) => ({ ...l, k: 'said' as const })),
    ...handoffs.slice(1).map((l) => ({ ...l, k: 'again' as const })),
    ...thread.items.filter((i) => mine(i)).map((i) => ({ ...i, k: 'item' as const })),
  ].sort((a, b) => (a.at ?? 0) - (b.at ?? 0) || (a.seq ?? 0) - (b.seq ?? 0));
  const items = raw.filter((l): l is ItemRow & { k: 'item' } => l.k === 'item');
  const last = new Set<ItemRow>();
  items.forEach((item, index) => {
    if (item.kind !== 'agent_message') return;
    const after = items
      .slice(index + 1)
      .find((x) => x.kind === 'route' || x.kind === 'agent_message');
    if (!after || after.kind === 'route') last.add(item);
  });

  let pending: ItemRow[] = [];
  let routed = false;
  let voice: string | null = null;
  let againAt = -Infinity;
  const flush = () => {
    if (pending.length)
      lines.push({
        type: 'work',
        key: `tw${pending[0]!.seq}`,
        items: pending,
        tools: pending.filter((i) => i.kind === 'tool_call').length,
        thoughts: pending.filter((i) => i.kind === 'thought').length,
      });
    pending = [];
  };
  for (const line of raw) {
    if (line.k === 'again') {
      flush();
      lines.push({ type: 'divider', key: `d${line.seq}`, at: line.at, where: null });
      lines.push({
        type: 'post',
        key: `ag${line.seq}`,
        voice: asVoice(line),
        run: false,
        report: false,
      });
      againAt = line.at;
      voice = null;
      continue;
    }
    if (line.k === 'said') {
      flush();
      const who = named(line);
      lines.push({
        type: 'post',
        key: `s${line.seq}`,
        voice: asVoice(line),
        run: voice === who,
        report: false,
      });
      voice = who;
      continue;
    }
    if (line.kind === 'thought' || line.kind === 'tool_call' || line.kind === 'plan') {
      pending.push(line);
      continue;
    }
    flush();
    if (line.kind === 'route') {
      if (routed && line.at - againAt > 120000) {
        const d = line.data || {};
        const where = [d.agent, d.model, d.reasoning].filter(Boolean).join(' · ');
        lines.push({ type: 'divider', key: `dr${line.seq}`, at: line.at, where: where || null });
        voice = null;
      }
      routed = true;
    } else if (line.kind === 'agent_message') {
      const who = line.role || name;
      lines.push({
        type: 'post',
        key: `i${line.seq}`,
        voice: {
          who,
          to: null,
          role: line.role,
          agent: line.agent,
          provider: line.provider,
          model: line.model,
          at: line.at,
          text: line.text,
          mine: false,
          mentions: [],
          mentionsYou: false,
        },
        run: voice === who,
        report: last.has(line),
      });
      voice = who;
    }
  }
  flush();
  return { head: barFor(name, said, members), lines };
}

/** Threads (members) that are running or have something waiting on the person. */
export function activeMembers(members: Member[]): Member[] {
  return members.filter((m) => m.live || m.needs || m.state === 'asking');
}

/** The agents a seat's own CLI runs inside its tool calls, as the activity tree draws them. */
export const tasksOf = (seatId: string, tasks: LiveTask[]) =>
  tasks.filter((t) => t.seat_id === seatId);
