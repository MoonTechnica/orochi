import type { Mention, ProjectRow, Prompt, ThreadStatus } from '@orochi/protocol';

export interface Snapshot {
  prompts: string[];
  mentions: number[];
  status: Record<string, ThreadStatus>;
}

export interface Notice {
  key: string;
  kind: 'prompt' | 'mention' | 'finished';
  thread: string;
  title: string;
  body: string;
}

export const snapshotOf = (
  projects: ProjectRow[],
  prompts: Prompt[],
  mentions: Mention[],
): Snapshot => ({
  prompts: prompts.map((p) => p.id),
  mentions: mentions.map((m) => m.message_id),
  status: Object.fromEntries(projects.flatMap((p) => p.threads).map((t) => [t.id, t.status])),
});

const RUNNING: ThreadStatus[] = ['working', 'background', 'queued'];

/**
 * What is new since `before`: questions waiting, @you mentions, turns that stopped running.
 * The first snapshot (before === null) announces nothing.
 */
export function noticesFor(
  before: Snapshot | null,
  projects: ProjectRow[],
  prompts: Prompt[],
  mentions: Mention[],
): Notice[] {
  if (!before) return [];
  const out: Notice[] = [];
  for (const p of prompts) {
    if (before.prompts.includes(p.id)) continue;
    out.push({
      key: `p${p.id}`,
      kind: 'prompt',
      thread: p.thread_id,
      title: p.thread_title || p.project,
      body: p.title || 'Needs you',
    });
  }
  for (const m of mentions) {
    if (before.mentions.includes(m.message_id)) continue;
    out.push({
      key: `m${m.message_id}`,
      kind: 'mention',
      thread: m.thread_id,
      title: `${m.who} → you`,
      body: m.text.slice(0, 140),
    });
  }
  const asking = new Set(prompts.map((p) => p.thread_id));
  for (const th of projects.flatMap((p) => p.threads)) {
    const was = before.status[th.id];
    if (!was || !RUNNING.includes(was) || RUNNING.includes(th.status)) continue;
    if (th.status === 'needs_you' || asking.has(th.id)) continue;
    out.push({
      key: `f${th.id}:${th.updated_at}`,
      kind: 'finished',
      thread: th.id,
      title: th.title,
      body: th.status === 'failed' ? 'Failed' : 'Finished',
    });
  }
  return out;
}

/** The app icon badge: questions plus mentions waiting. */
export const badgeCount = (prompts: Prompt[], mentions: Mention[]) =>
  prompts.length + mentions.length;

/** With more than one desktop paired a notice says which one it came from. */
export const withMachine = <N extends { title: string }>(
  notice: N,
  machineName: string,
  machines: number,
): N => (machines > 1 ? { ...notice, title: `${machineName} · ${notice.title}` } : notice);
