import type { ItemRow, Prompt, RoutineRow } from '@orochi/protocol';
import { toMs } from './time';
import { buildTimeline, type Entry, type TimelineInput } from './timeline';

/**
 * A resident's conversation as a messenger shows it: the person's bubbles and the assistant's,
 * a line where a routine woke it, its proposals and questions as cards, and what it read to
 * answer as links under the answer. The work itself (tool calls, routes, checks) stays in the
 * full conversation view.
 */
export interface Source {
  kind: 'file' | 'web';
  /** A path for a file, an address for a page. */
  target: string;
  label: string;
}

export type ChatLine =
  | { type: 'me'; key: string; text: string; at: number | null }
  | { type: 'agent'; key: string; text: string; at: number | null; sources: Source[] }
  | { type: 'proposal'; key: string; routine: RoutineRow }
  | { type: 'ask'; key: string; prompt: Prompt }
  | { type: 'typing'; key: string }
  | { type: 'day'; key: string; at: number };

const MAX_SOURCES = 4;

const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

const host = (url: string) => {
  const m = /^https?:\/\/([^/?#]+)/i.exec(url);
  return m ? m[1]!.replace(/^www\./, '') : url;
};

/** What one tool call read or fetched, if it was reading or fetching. */
export function sourcesOf(item: ItemRow): Source[] {
  if (item.kind !== 'tool_call' || !item.data) return [];
  const d = item.data;
  const kind = String(d.tool_kind ?? '');
  const raw = (d.raw_input ?? {}) as Record<string, unknown>;
  if (kind === 'fetch' || typeof raw.url === 'string') {
    const url = typeof raw.url === 'string' ? raw.url : '';
    return /^https?:\/\//i.test(url) ? [{ kind: 'web', target: url, label: host(url) }] : [];
  }
  if (kind !== 'read') return [];
  const paths: string[] = Array.isArray(d.locations)
    ? d.locations.map((l: { path?: unknown }) => String(l?.path ?? '')).filter(Boolean)
    : [];
  if (!paths.length && typeof raw.file_path === 'string') paths.push(raw.file_path);
  return paths.map((p) => ({ kind: 'file', target: p, label: baseName(p) }));
}

export interface ChatInput extends TimelineInput {
  /** The routines on this conversation; those still proposed become cards. */
  routines?: RoutineRow[];
}

export function buildChat(input: ChatInput): ChatLine[] {
  const entries = buildTimeline(input);
  const items = new Map(input.thread.items.map((i) => [`u${i.seq}`, i]));
  const out: ChatLine[] = [];
  let read: Source[] = [];
  const collect = (e: Extract<Entry, { type: 'work' }>) => {
    for (const item of e.items) {
      for (const s of sourcesOf(item)) {
        if (!read.some((r) => r.target === s.target)) read.push(s);
      }
    }
  };
  for (const e of entries) {
    switch (e.type) {
      case 'day':
        out.push({ type: 'day', key: e.key, at: e.at });
        break;
      case 'work':
        collect(e);
        break;
      case 'post': {
        const v = e.voice;
        if (v.mine) {
          const item = items.get(e.key);
          read = [];
          // A routine's wake is not something the person said, and is not shown.
          if (item?.turn_origin !== 'routine') {
            out.push({ type: 'me', key: e.key, text: v.text, at: v.at });
          }
          break;
        }
        const last = out.at(-1);
        // One reply streamed as several messages reads as one bubble.
        if (last?.type === 'agent' && e.run) {
          last.text = `${last.text}\n\n${v.text}`;
          last.sources = mergeSources(last.sources, read);
        } else {
          out.push({
            type: 'agent',
            key: e.key,
            text: v.text,
            at: v.at,
            sources: read.slice(0, MAX_SOURCES),
          });
        }
        read = [];
        break;
      }
      case 'ask':
        out.push({ type: 'ask', key: e.key, prompt: e.prompt });
        break;
      case 'live':
        out.push({ type: 'typing', key: e.key });
        break;
      default:
        break;
    }
  }
  const proposals = (input.routines ?? []).filter(
    (r) => r.proposed && r.thread_id === input.thread.thread.id,
  );
  for (const routine of proposals) {
    out.push({ type: 'proposal', key: `pr${routine.id}`, routine });
  }
  // Something waiting on the person sits below a reply still being typed.
  const typing = out.findIndex((b) => b.type === 'typing');
  if (typing >= 0 && typing < out.length - 1) out.push(...out.splice(typing, 1));
  return out;
}

function mergeSources(a: Source[], b: Source[]): Source[] {
  const all = [...a];
  for (const s of b) if (!all.some((x) => x.target === s.target)) all.push(s);
  return all.slice(0, MAX_SOURCES);
}

/** A long reply shown folded: the first `limit` characters at a paragraph or sentence end. */
export function folded(text: string, limit = 280): { head: string; more: boolean } {
  if (text.length <= limit + 40) return { head: text, more: false };
  const cut = text.slice(0, limit);
  const at = Math.max(cut.lastIndexOf('\n'), cut.lastIndexOf('。'), cut.lastIndexOf('. '));
  return { head: `${(at > limit / 2 ? cut.slice(0, at + 1) : cut).trimEnd()}…`, more: true };
}

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

/** A line with the day before the first bubble of each day. */
export function chatWithDays(bubbles: ChatLine[]): ChatLine[] {
  const out: ChatLine[] = [];
  let day: string | null = null;
  for (const b of bubbles) {
    const raw = 'at' in b ? b.at : null;
    const at = raw ? toMs(raw) : null;
    const d = at ? new Date(at).toDateString() : null;
    if (at && d !== day) {
      out.push({ type: 'day', key: `day-${b.key}`, at });
      day = d;
    }
    out.push(b);
  }
  return out;
}
