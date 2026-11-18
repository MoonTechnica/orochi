/**
 * Who is speaking, as a face. The conversation and the Office draw the same person: the look is
 * `lookFor(seedOf({ role, agent }), kindOf(...))`, the one function the Office uses, so an agent is
 * one character wherever it appears. The human is not an agent: a calm character chosen by a
 * seed kept per install, never by role or agent. Pure; the drawing is the app's.
 */
import type { AgentUse, Member, ThreadRow } from '@orochi/protocol';
import { kindOf, lookFor, seedOf, type Look } from './avatar';
import type { PersonState } from './office';

const lowered = (v: unknown) => String(v ?? '').toLowerCase();

export interface Who {
  /** The name shown beside the face (also its accessibility label). */
  name: string;
  role: string | null;
  agent: string | null;
  provider: string | null;
  lead: boolean;
  /** The person using the app. */
  you: boolean;
}

/** The role the conversation's lead goes by (usually `lead`; an old thread may name it otherwise). */
export function leadRoleOf(
  thread: { agents?: AgentUse[] | null } | null | undefined,
): string | null {
  const lead = thread?.agents?.find((a) => a.lead);
  return lead ? lead.role || 'lead' : null;
}

export function whoOf(
  ref: {
    name?: string | null;
    role?: string | null;
    agent?: string | null;
    provider?: string | null;
    mine?: boolean;
  },
  leadRole: string | null,
): Who {
  const role = ref.role || null;
  const you = Boolean(ref.mine);
  return {
    name: ref.name || role || ref.agent || '',
    role,
    agent: ref.agent ?? null,
    provider: ref.provider ?? null,
    lead:
      !you && Boolean(role) && (lowered(role) === lowered(leadRole) || lowered(role) === 'lead'),
    you,
  };
}

/** A speaker nobody can place (no role, no agent, not you): the app draws an initial, not a face. */
export const isAnonymous = (w: Who) => !w.you && !w.role && !w.agent;

/** Stable identity of a drawing, for memoizing: the same key is the same pixels. */
export const whoKey = (w: Who) =>
  w.you ? 'you' : isAnonymous(w) ? `?${w.name}` : `${seedOf(w)}${w.lead ? '|L' : ''}`;

export function lookOfWho(w: Who, youSeed = ''): Look {
  if (w.you) return { ...lookFor(`you|${youSeed}`, 'other'), accessory: 'none' };
  return lookFor(seedOf(w), kindOf({ role: w.role, lead: w.lead }));
}

/** Who a name in a conversation is: a member of it, the lead, or whoever the thread's agents say. */
export function whoNamed(
  name: string,
  members: readonly Member[],
  thread: { agents?: AgentUse[] | null } | null | undefined,
): Who {
  const key = lowered(name);
  const leadRole = leadRoleOf(thread);
  const m = members.find((x) => lowered(x.name) === key);
  if (m)
    return whoOf({ name: m.name, role: m.role, agent: m.agent, provider: m.provider }, leadRole);
  const lead = thread?.agents?.find((a) => a.lead);
  if (lead && (key === 'lead' || key === lowered(lead.role)))
    return whoOf(
      { name, role: lead.role || 'lead', agent: lead.agent, provider: lead.provider },
      leadRole,
    );
  const other = thread?.agents?.find((a) => lowered(a.role) === key);
  return whoOf(
    { name, role: other?.role ?? name, agent: other?.agent, provider: other?.provider },
    leadRole,
  );
}

/** The Office's states for a member, so a bar and the Office show one face for one situation. */
export function personStateOf(m: Member | undefined | null): PersonState {
  if (!m) return 'reported';
  if (m.state === 'asking') return 'asking';
  if (m.live || ['working', 'choosing', 'starting'].includes(m.state)) return 'working';
  if (['failed', 'interrupted', 'cancelled', 'stopped'].includes(m.state)) return 'idle';
  return 'reported';
}

/** The agent behind a name in a thread row (a mention names a role, never an agent). */
export function agentNamed(
  thread: Pick<ThreadRow, 'agents'> | null | undefined,
  name: string | null | undefined,
): AgentUse | undefined {
  const key = lowered(name);
  if (!key) return undefined;
  return thread?.agents?.find((a) => lowered(a.role) === key || (key === 'lead' && a.lead));
}
