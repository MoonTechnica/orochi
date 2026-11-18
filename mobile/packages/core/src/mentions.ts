import type { ItemRow, Member, ThreadRow } from '@orochi/protocol';
import { memberState } from './status';
import { toMs } from './time';

export interface Candidate {
  name: string;
  agent?: string | null;
  provider?: string | null;
  model?: string | null;
  role?: string;
  live: boolean;
  at: number;
  state: string;
  channel?: boolean;
}

const lowered = (v: unknown) => String(v ?? '').toLowerCase();

/** Who typing `@` offers: members (working first), the lead, `@channel` last. */
export function mentionCandidates(
  query: string,
  members: Member[],
  thread: { thread: ThreadRow; items: ItemRow[] } | null,
): Candidate[] {
  const q = lowered(query);
  const last = (m: Member) =>
    Math.max(toMs(m.last_reply_at) ?? 0, m.ended_at ?? 0, m.started_at ?? 0);
  const all: Candidate[] = members.map((m) => ({
    name: m.name,
    agent: m.agent,
    provider: m.provider,
    model: m.model,
    role: m.role,
    live: m.live,
    at: last(m),
    state: memberState(m).label,
  }));
  if (thread) {
    const lead = (thread.thread.agents || []).find((a) => a.lead);
    const at = Math.max(0, ...thread.items.filter((i) => !i.member).map((i) => i.at || 0));
    all.push({
      name: 'lead',
      agent: lead?.agent ?? thread.thread.agent,
      provider: lead?.provider ?? thread.thread.provider,
      model: lead?.model ?? '',
      role: 'lead',
      live: false,
      at,
      state: thread.thread.status === 'working' ? 'working' : 'lead',
    });
  }
  const hits = all
    .filter((c) => [c.name, c.agent, c.role].some((w) => lowered(w).startsWith(q)))
    .sort((a, b) => Number(b.live) - Number(a.live) || b.at - a.at);
  if ('channel'.startsWith(q))
    hits.push({ name: 'channel', channel: true, live: false, at: 0, state: 'everyone' });
  return hits;
}

/** The `@word` being typed at the caret, if that is what is being typed. */
export function mentionQuery(
  value: string,
  caret: number = value.length,
): { query: string; start: number; caret: number } | null {
  const at = Math.min(caret, value.length);
  const found = value.slice(0, at).match(/(^|\s)@([\w.-]*)$/);
  return found ? { query: found[2]!, start: at - found[2]!.length - 1, caret: at } : null;
}

export function applyMention(
  value: string,
  q: { start: number; caret: number },
  name: string,
): { value: string; caret: number } {
  const after = value.slice(q.caret);
  const insert = `@${name}${after.startsWith(' ') ? '' : ' '}`;
  const next = value.slice(0, q.start) + insert + after;
  return { value: next, caret: q.start + insert.length + (after.startsWith(' ') ? 1 : 0) };
}

/** Names mentioned in a text that exist in the conversation (never trusting the sender). */
export function mentionsIn(text: string, names: string[]): string[] {
  const known = new Set([...names.map(lowered), 'you', 'channel']);
  const out: string[] = [];
  for (const m of text.matchAll(/(^|[^\w@])@([A-Za-z][\w.-]*)/g)) {
    const n = lowered(m[2]);
    if (known.has(n) && !out.includes(n)) out.push(n);
  }
  return out;
}
