import { toMs } from './time';
import { agentNamed } from './people';
import type {
  AgentUse,
  LiveSeat,
  LiveTask,
  Mention,
  ProjectRow,
  Prompt,
  ThreadRow,
} from '@orochi/protocol';

export type LinkState = 'online' | 'connecting' | 'offline' | 'repair';

/** What one desktop last told this phone (the active one live, the others polled). */
export interface MachineSnapshot {
  key: string;
  name: string;
  state: LinkState;
  error?: string | null;
  projects: ProjectRow[];
  prompts: Prompt[];
  mentions: Mention[];
  seats: LiveSeat[];
  tasks: LiveTask[];
}

export type AgentKind = 'approval' | 'mention' | 'working' | 'background';

export interface AgentRow {
  id: string;
  machineKey: string;
  machineName: string;
  thread: string;
  title: string;
  project: string;
  kind: AgentKind;
  /** Running agents and sub-agent tasks under this conversation. */
  agents: number;
  tasks: number;
}

export interface DesktopCard {
  key: string;
  name: string;
  state: LinkState;
  error: string | null;
  active: number;
  waiting: number;
  /** Conversations touched in each tenth of the last hour, oldest first. */
  bars: number[];
}

/** One thing that waits for the user: an open question or an @you line. */
export interface NeedItem {
  id: string;
  machineKey: string;
  machineName: string;
  thread: string;
  title: string;
  project: string;
  kind: 'approval' | 'mention';
  at: number;
  prompt?: Prompt;
  mention?: Mention;
  /** The agent behind a mention's speaker, when the thread names one (its face). */
  from?: AgentUse;
}

export interface Trail {
  key: string;
  project: string;
  machineKey: string;
  machineName: string;
  items: RecentRow[];
}

export interface RecentRow {
  machineKey: string;
  machineName: string;
  thread: ThreadRow;
}

export interface Home {
  agents: AgentRow[];
  needs: NeedItem[];
  desktops: DesktopCard[];
  recent: RecentRow[];
  /** Rows in the Agent section: what is running plus what waits for you. */
  active: number;
}

const ORDER: Record<AgentKind, number> = { approval: 0, mention: 1, working: 2, background: 3 };

/**
 * The home screen, across desktops. Approvals and mentions come first; a conversation that is
 * both asking and running is one row (its most urgent state). An unreachable desktop keeps
 * its card and contributes whatever it last said to nothing but the card.
 */
export function buildHome(snapshots: MachineSnapshot[], recentLimit = 10, now = Date.now()): Home {
  const agents: AgentRow[] = [];
  const needs: NeedItem[] = [];
  const recent: RecentRow[] = [];
  const desktops: DesktopCard[] = [];
  for (const m of snapshots) {
    const shows = m.state === 'online';
    const rows = new Map<string, AgentRow>();
    const threads = new Map<string, { th: ThreadRow; project: string }>();
    for (const p of m.projects)
      for (const th of p.threads) threads.set(th.id, { th, project: p.name });
    const put = (kind: AgentKind, id: string, thread: string, title: string, project: string) => {
      const was = rows.get(thread);
      if (was && ORDER[was.kind] <= ORDER[kind]) return;
      rows.set(thread, {
        id: `${m.key}:${id}`,
        machineKey: m.key,
        machineName: m.name,
        thread,
        title,
        project,
        kind,
        agents: m.seats.filter((s) => s.thread_id === thread).length,
        tasks: m.tasks.filter((k) => (k as { thread_id?: string }).thread_id === thread).length,
      });
    };
    if (shows) {
      for (const p of m.prompts) {
        needs.push({
          id: `${m.key}:p${p.id}`,
          machineKey: m.key,
          machineName: m.name,
          thread: p.thread_id,
          title: p.thread_title || p.project,
          project: p.project,
          kind: 'approval',
          at: toMs(p.created_at) ?? 0,
          prompt: p,
        });
        put('approval', `p${p.id}`, p.thread_id, p.thread_title || p.project, p.project);
      }
      for (const x of m.mentions) {
        const known = threads.get(x.thread_id);
        needs.push({
          id: `${m.key}:m${x.message_id}`,
          machineKey: m.key,
          machineName: m.name,
          thread: x.thread_id,
          title: known?.th.title || x.thread_title || x.who,
          project: known?.project ?? x.project,
          kind: 'mention',
          at: toMs(x.at) ?? 0,
          mention: x,
          from: agentNamed(known?.th, x.member ?? x.who),
        });
        put(
          'mention',
          `m${x.message_id}`,
          x.thread_id,
          known?.th.title ?? x.who,
          known?.project ?? '',
        );
      }
      for (const { th, project } of threads.values()) {
        if (th.archived) continue;
        if (th.status === 'working' || th.status === 'queued')
          put('working', th.id, th.id, th.title, project);
        else if (th.status === 'background') put('background', th.id, th.id, th.title, project);
      }
      for (const { th } of threads.values())
        if (!th.archived) recent.push({ machineKey: m.key, machineName: m.name, thread: th });
    }
    const mine = [...rows.values()];
    agents.push(...mine);
    desktops.push({
      key: m.key,
      name: m.name,
      state: m.state,
      error: m.error ?? null,
      active: mine.length,
      waiting: mine.filter((r) => r.kind === 'approval' || r.kind === 'mention').length,
      bars: shows ? activityBars(m.projects, now) : [0, 0, 0, 0, 0, 0],
    });
  }
  agents.sort((a, b) => ORDER[a.kind] - ORDER[b.kind]);
  needs.sort((a, b) => (a.kind === b.kind ? b.at - a.at : a.kind === 'approval' ? -1 : 1));
  recent.sort(
    (a, b) =>
      Number(b.thread.pinned) - Number(a.thread.pinned) ||
      b.thread.updated_at - a.thread.updated_at,
  );
  return { agents, needs, desktops, recent: recent.slice(0, recentLimit), active: agents.length };
}

/**
 * Dismissing one mention card. `see_mentions` is per thread, so the desktop is told only when
 * no other undismissed mention of that thread is left on the phone; until then the card is
 * hidden here alone and the others stay.
 */
export function dismissMention(
  needs: readonly NeedItem[],
  dismissed: readonly string[],
  item: NeedItem,
): { dismissed: string[]; seeThread: boolean } {
  const next = dismissed.includes(item.id) ? [...dismissed] : [...dismissed, item.id];
  const left = needs.some(
    (n) =>
      n.kind === 'mention' &&
      n.machineKey === item.machineKey &&
      n.thread === item.thread &&
      !next.includes(n.id),
  );
  return { dismissed: next, seeThread: !left };
}

/** Conversations updated in each of six ten-minute buckets ending now, oldest first. */
export function activityBars(projects: ProjectRow[], now: number): number[] {
  const bars = [0, 0, 0, 0, 0, 0];
  const bucket = 10 * 60 * 1000;
  for (const p of projects)
    for (const th of p.threads) {
      const at = toMs(th.updated_at);
      if (at === null || th.archived) continue;
      const ago = now - at;
      if (ago < 0 || ago >= bucket * 6) continue;
      bars[5 - Math.floor(ago / bucket)] += 1;
    }
  return bars;
}

/** The headline's numbers: heads that wait for you, heads working, heads watching in the background. */
export function headCounts(agents: AgentRow[]) {
  let need = 0;
  let working = 0;
  let background = 0;
  for (const a of agents) {
    if (a.kind === 'approval' || a.kind === 'mention') need++;
    else if (a.kind === 'working') working++;
    else background++;
  }
  return { need, working, background };
}

/** Recent conversations as branches: one per project (and desktop), in order of first appearance. */
export function groupTrail(rows: RecentRow[]): Trail[] {
  const out = new Map<string, Trail>();
  for (const r of rows) {
    const key = `${r.machineKey}:${r.thread.project_id || r.thread.project}`;
    let t = out.get(key);
    if (!t) {
      t = {
        key,
        project: r.thread.project,
        machineKey: r.machineKey,
        machineName: r.machineName,
        items: [],
      };
      out.set(key, t);
    }
    t.items.push(r);
  }
  return [...out.values()];
}
