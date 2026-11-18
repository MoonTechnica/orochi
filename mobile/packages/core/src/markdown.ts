// A small Markdown reader for conversation text: it keeps what the app draws (blocks, inline
// code, links, images, emphasis, mentions) and resolves media links as the desktop does.

export type Inline =
  | { t: 'text'; text: string }
  | { t: 'code'; text: string }
  | { t: 'strong' | 'em' | 'del'; children: Inline[] }
  | { t: 'link'; label: string; target: string; image: boolean }
  | { t: 'mention'; name: string; text: string };

export type Block =
  | { t: 'p'; lines: Inline[][] }
  | { t: 'heading'; level: number; inline: Inline[] }
  | { t: 'code'; lang: string; text: string }
  | { t: 'list'; ordered: boolean; items: Inline[][] }
  | { t: 'quote'; blocks: Block[] }
  | { t: 'rule' }
  | { t: 'table'; head: Inline[][]; rows: Inline[][][] };

const FENCE = /^\s*(```|~~~)\s*([\w+#.-]*)\s*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const ROW = /^\s*\|.*\|\s*$/;
const DIVIDER = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

const INLINE =
  /(`+)([\s\S]*?[^`])\1(?!`)|!?\[([^\]]*)\]\((<[^>]+>|[^)\s]+)(?:\s+"[^"]*")?\)|\*\*([^*]+)\*\*|__([^_]+)__|~~([^~]+)~~|\*([^*\s][^*]*?)\*|(?<![\w])_([^_\s][^_]*?)_(?![\w])|(https?:\/\/[^\s<>()]+[^\s<>().,;:!?'"])/g;

const MENTION = /(^|[^\w@])@([A-Za-z][\w.-]*)/g;

function plain(out: Inline[], value: string, known?: Set<string>) {
  let at = 0;
  for (const m of value.matchAll(MENTION)) {
    const name = m[2]!;
    const lower = name.toLowerCase();
    if (known && !known.has(lower) && lower !== 'you' && lower !== 'channel') continue;
    const start = (m.index ?? 0) + m[1]!.length;
    if (start > at) out.push({ t: 'text', text: value.slice(at, start) });
    out.push({ t: 'mention', name, text: `@${name}` });
    at = start + name.length + 1;
  }
  if (at < value.length) out.push({ t: 'text', text: value.slice(at) });
}

export function parseInline(line: string, known?: Set<string>): Inline[] {
  const out: Inline[] = [];
  let at = 0;
  for (const m of line.matchAll(INLINE)) {
    const index = m.index ?? 0;
    if (index > at) plain(out, line.slice(at, index), known);
    at = index + m[0].length;
    if (m[1]) out.push({ t: 'code', text: m[2]!.trim() });
    else if (m[3] !== undefined)
      out.push({
        t: 'link',
        label: m[3],
        target: m[4]!.replace(/^<|>$/g, ''),
        image: m[0].startsWith('!'),
      });
    else if (m[5] || m[6]) out.push({ t: 'strong', children: parseInline((m[5] || m[6])!, known) });
    else if (m[7]) out.push({ t: 'del', children: parseInline(m[7], known) });
    else if (m[8] || m[9]) out.push({ t: 'em', children: parseInline((m[8] || m[9])!, known) });
    else if (m[10]) out.push({ t: 'link', label: m[10], target: m[10], image: false });
  }
  if (at < line.length) plain(out, line.slice(at), known);
  return out;
}

const cells = (row: string) =>
  row
    .trim()
    .replace(/^\||\|$/g, '')
    .split('|')
    .map((c) => c.trim());

export function parseMarkdown(source: string, known?: Set<string>): Block[] {
  const lines = String(source ?? '')
    .replace(/\r\n?/g, '\n')
    .split('\n');
  return blocks(lines, known);
}

function blocks(lines: string[], known?: Set<string>): Block[] {
  const out: Block[] = [];
  let paragraph: string[] = [];
  const flush = () => {
    if (!paragraph.length) return;
    out.push({ t: 'p', lines: paragraph.map((l) => parseInline(l.trim(), known)) });
    paragraph = [];
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    let m: RegExpMatchArray | null;
    if ((m = line.match(FENCE))) {
      flush();
      const close = m[1]!;
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i]!.trim().startsWith(close)) body.push(lines[i++]!);
      i++;
      out.push({ t: 'code', lang: m[2] ?? '', text: body.join('\n') });
      continue;
    }
    if (!line.trim()) {
      flush();
      i++;
      continue;
    }
    if ((m = line.match(HEADING))) {
      flush();
      out.push({ t: 'heading', level: m[1]!.length, inline: parseInline(m[2]!, known) });
      i++;
      continue;
    }
    if (RULE.test(line)) {
      flush();
      out.push({ t: 'rule' });
      i++;
      continue;
    }
    if (QUOTE.test(line)) {
      flush();
      const body: string[] = [];
      while (i < lines.length && (m = lines[i]!.match(QUOTE))) {
        body.push(m[1]!);
        i++;
      }
      out.push({ t: 'quote', blocks: blocks(body, known) });
      continue;
    }
    if (ROW.test(line) && i + 1 < lines.length && DIVIDER.test(lines[i + 1]!)) {
      flush();
      const head = cells(line).map((c) => parseInline(c, known));
      i += 2;
      const rows: Inline[][][] = [];
      while (i < lines.length && ROW.test(lines[i]!))
        rows.push(cells(lines[i++]!).map((c) => parseInline(c, known)));
      out.push({ t: 'table', head, rows });
      continue;
    }
    if ((m = line.match(ITEM))) {
      flush();
      const ordered = /\d/.test(m[2]!);
      const items: Inline[][] = [];
      while (i < lines.length && (m = lines[i]!.match(ITEM))) {
        items.push(parseInline(m[3]!, known));
        i++;
      }
      out.push({ t: 'list', ordered, items });
      continue;
    }
    paragraph.push(line);
    i++;
  }
  flush();
  return out;
}

// Media links ---------------------------------------------------------------------------------

export type MediaType = 'image' | 'video' | 'audio';

export function mediaKind(target: string): MediaType | null {
  const path = target.split(/[?#]/)[0]!;
  if (/\.(png|jpe?g|gif|webp|svg|bmp|avif|heic)$/i.test(path)) return 'image';
  if (/\.(mp4|m4v|webm|mov)$/i.test(path)) return 'video';
  if (/\.(mp3|m4a|wav|ogg|flac|aac)$/i.test(path)) return 'audio';
  return null;
}

export type MediaSource =
  | { type: 'file'; path: string; line: number | null }
  | { type: 'orochi-media'; thread: string; sha: string }
  | { type: 'url'; url: string };

/** What a link in a message points at, and how to show it. */
export interface Resolved {
  media: MediaType | null;
  source: MediaSource | null;
  /** The label to show; falls back to the path. */
  label: string;
  target: string;
}

const HASH = /^[0-9a-f]{64}$/i;

const remoteOk = (target: string) =>
  /^https:\/\//i.test(target) ||
  /^http:\/\/(?:localhost|127\.0\.0\.1|\[::1\])(?::\d+)?(?:\/|$)/i.test(target) ||
  /^data:image\/(?:png|jpeg|gif|webp|bmp|avif);base64,[a-z0-9+/=]+$/i.test(target);

/** A path inside the thread's working folder, relative to it, or null when it lies outside. */
export function inTree(path: string, cwd: string | null | undefined): string | null {
  if (typeof path !== 'string' || !path) return null;
  if (!path.startsWith('/')) return path.replace(/^\.\//, '');
  const root = (cwd ?? '').replace(/\/+$/, '');
  if (root && path.startsWith(`${root}/`)) return path.slice(root.length + 1);
  return null;
}

export function fileTarget(
  target: string,
  cwd: string | null | undefined,
): { path: string; line: number | null } | null {
  let line: number | null = null;
  let value = target.replace(/:(\d+)(?::\d+)?$/, (_, n) => {
    line = Number(n);
    return '';
  });
  value = value.replace(/#L(\d+)(?:-L?\d+)?$/, (_, n) => {
    line = Number(n);
    return '';
  });
  try {
    value = decodeURIComponent(value);
  } catch {
    return null;
  }
  if (/^[a-z][a-z0-9+.-]*:/i.test(value)) {
    if (!value.startsWith('file:///')) return null;
    value = value.slice(7);
  }
  const path = inTree(value, cwd);
  if (!path || path.split('/').some((part) => part === '..' || part === '.git')) return null;
  return { path, line };
}

/** `orochi-media://localhost/<thread>/<sha>`, the Windows host form, or `orochi-media:<sha>`. */
export function orochiMedia(
  target: string,
  thread: string | null,
): { thread: string; sha: string } | null {
  const m =
    /^orochi-media:\/\/[^/]*\/(?:([^/]+)\/)?([0-9a-f]{64})\/?$/i.exec(target) ??
    /^http:\/\/orochi-media\.localhost\/(?:([^/]+)\/)?([0-9a-f]{64})\/?$/i.exec(target) ??
    /^orochi-media:(?:([^/:]+)[/:])?([0-9a-f]{64})$/i.exec(target);
  if (!m) return null;
  const id = m[1] ?? thread;
  if (!id || !HASH.test(m[2]!)) return null;
  return { thread: id, sha: m[2]!.toLowerCase() };
}

export function resolveLink(
  label: string,
  target: string,
  imageSyntax: boolean,
  ctx: { thread: string | null; cwd: string | null | undefined },
): Resolved {
  const bare = (media: MediaType | null, source: MediaSource | null): Resolved => ({
    media,
    source,
    label: label || target,
    target,
  });
  const stored = orochiMedia(target, ctx.thread);
  if (stored) {
    const kind = imageSyntax ? 'image' : (mediaKind(label) ?? 'image');
    return bare(kind, { type: 'orochi-media', thread: stored.thread, sha: stored.sha });
  }
  const kind = imageSyntax ? 'image' : mediaKind(target);
  const file = ctx.thread ? fileTarget(target, ctx.cwd) : null;
  if (file)
    return {
      media: kind,
      source: { type: 'file', path: file.path, line: file.line },
      label: label || file.path,
      target,
    };
  if (kind && remoteOk(target)) return bare(kind, { type: 'url', url: target });
  if (/^https?:\/\//i.test(target)) return bare(null, { type: 'url', url: target });
  return bare(null, null);
}

/** Every media link in a message, in order, for the conversation's gallery. */
export function mediaLinks(
  text: string,
  ctx: { thread: string | null; cwd: string | null | undefined },
): Resolved[] {
  const out: Resolved[] = [];
  const walk = (inl: Inline[]) => {
    for (const n of inl) {
      if (n.t === 'link') {
        const r = resolveLink(n.label, n.target, n.image, ctx);
        if (r.media && r.source) out.push(r);
      } else if ('children' in n) walk(n.children);
    }
  };
  const visit = (bs: Block[]) => {
    for (const b of bs) {
      if (b.t === 'p') b.lines.forEach(walk);
      else if (b.t === 'heading') walk(b.inline);
      else if (b.t === 'list') b.items.forEach(walk);
      else if (b.t === 'quote') visit(b.blocks);
      else if (b.t === 'table') {
        b.head.forEach(walk);
        b.rows.forEach((r) => r.forEach(walk));
      }
    }
  };
  visit(parseMarkdown(text));
  return out;
}
