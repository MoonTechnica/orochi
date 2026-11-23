import type { RoutineRow } from '@orochi/protocol';

/** A proposal in words: when it would run (or the command it would watch) and what it does. */
export function proposalWords(r: RoutineRow): { when: string; command: string | null } {
  if (r.watch) {
    const secs = r.every_secs ?? 0;
    const every =
      secs >= 3600 && secs % 3600 === 0 ? `${secs / 3600}h` : `${Math.round(secs / 60)}m`;
    return { when: `every ${every}`, command: r.watch };
  }
  return { when: r.schedule ?? '', command: null };
}
