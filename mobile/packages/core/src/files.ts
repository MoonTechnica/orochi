import type { TreeEntry } from '@orochi/protocol';

/** Directories first, then names, case-insensitively; deleted-but-known entries stay listed. */
export const sortTree = (entries: TreeEntry[]): TreeEntry[] =>
  [...entries].sort(
    (a, b) =>
      Number(b.kind === 'dir') - Number(a.kind === 'dir') ||
      a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
  );

export const crumbs = (dir: string): { name: string; path: string }[] => {
  const parts = dir.split('/').filter(Boolean);
  return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join('/') }));
};

export const parentDir = (path: string) => path.split('/').slice(0, -1).join('/');

const IMAGE = /\.(png|jpe?g|gif|webp|bmp|svg|heic)$/i;
const VIDEO = /\.(mp4|m4v|mov|webm)$/i;
const AUDIO = /\.(mp3|m4a|wav|ogg|aac|flac)$/i;
export const fileKind = (path: string): 'image' | 'video' | 'audio' | 'text' =>
  IMAGE.test(path) ? 'image' : VIDEO.test(path) ? 'video' : AUDIO.test(path) ? 'audio' : 'text';

export const sizeLabel = (bytes: number | null | undefined) => {
  if (bytes == null) return '';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
};

export interface DiffLine {
  kind: 'add' | 'del' | 'ctx' | 'hunk' | 'meta';
  text: string;
  /** Line number in the new file; what a review comment refers to. */
  line: number | null;
}

/** A unified diff as lines with new-file line numbers. */
export function parseDiff(text: string): DiffLine[] {
  const out: DiffLine[] = [];
  let next = 0;
  let inHunk = false;
  for (const raw of text.replace(/\n$/, '').split('\n')) {
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw);
    if (hunk) {
      next = Number(hunk[1]);
      inHunk = true;
      out.push({ kind: 'hunk', text: raw, line: null });
    } else if (!inHunk || /^(diff --git|index |--- |\+\+\+ )/.test(raw)) {
      if (/^diff --git/.test(raw)) inHunk = false;
      out.push({ kind: 'meta', text: raw, line: null });
    } else if (raw.startsWith('+')) out.push({ kind: 'add', text: raw, line: next++ });
    else if (raw.startsWith('-')) out.push({ kind: 'del', text: raw, line: null });
    else if (raw.startsWith('\\')) out.push({ kind: 'meta', text: raw, line: null });
    else out.push({ kind: 'ctx', text: raw, line: next++ });
  }
  return out;
}

/** Route choices for `set_route`: `agent/model` for routes seen, then `agent` alone. */
export function routeChoices(
  routes: { agent: string; model: string }[],
  agents: { agent: string }[],
): { label: string; route: string | null }[] {
  const out: { label: string; route: string | null }[] = [{ label: 'Auto', route: null }];
  const seen = new Set<string>();
  for (const r of routes) {
    const key = `${r.agent}/${r.model}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ label: `${r.agent} · ${r.model}`, route: key });
  }
  for (const agent of new Set(agents.map((a) => a.agent))) {
    out.push({ label: `${agent} · any model`, route: agent });
  }
  return out;
}

export const EFFORTS: (string | null)[] = [null, 'low', 'medium', 'high', 'xhigh'];

export const routeWith = (agent: string, model: string | null, effort: string | null) => {
  const base = model ? `${agent}/${model}` : agent;
  return effort ? `${base} ${effort}` : base;
};
