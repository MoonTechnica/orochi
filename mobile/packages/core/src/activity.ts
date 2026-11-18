import type { LiveSeat, LiveTask, ProjectRow } from '@orochi/protocol';

export interface SeatNode {
  seat: LiveSeat;
  tasks: LiveTask[];
}
export interface ThreadNode {
  thread_id: string;
  title: string;
  project: string;
  seats: SeatNode[];
}

/** Live seats and the sub-agents under them, grouped by conversation (the activity tree). */
export function activityTree(
  seats: LiveSeat[],
  tasks: LiveTask[],
  projects: ProjectRow[],
): ThreadNode[] {
  const info = new Map<string, { title: string; project: string }>();
  for (const p of projects)
    for (const th of p.threads) info.set(th.id, { title: th.title, project: p.name });
  const nodes = new Map<string, ThreadNode>();
  for (const seat of seats) {
    let node = nodes.get(seat.thread_id);
    if (!node) {
      const meta = info.get(seat.thread_id);
      node = {
        thread_id: seat.thread_id,
        title: meta?.title || seat.title || seat.thread_id,
        project: meta?.project ?? seat.project_id,
        seats: [],
      };
      nodes.set(seat.thread_id, node);
    }
    node.seats.push({ seat, tasks: tasks.filter((t) => t.seat_id === seat.seat_id) });
  }
  for (const n of nodes.values()) n.seats.sort((a, b) => a.seat.ordinal - b.seat.ordinal);
  return [...nodes.values()].sort(
    (a, b) =>
      Number(b.seats.some((s) => s.seat.state === 'asking')) -
      Number(a.seats.some((s) => s.seat.state === 'asking')),
  );
}

export const taskTitle = (title: string | null) => (title ?? '').replace(/^Task:\s*/, '');
