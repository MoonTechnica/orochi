import type { ItemRow, MediaData } from '@orochi/protocol';
import { mediaLinks, type MediaSource, type MediaType } from './markdown';

export interface GalleryItem {
  id: string;
  kind: MediaType;
  name: string;
  mime: string | null;
  bytes: number | null;
  source: MediaSource;
  at: number;
}

const mimeOf = (name: string): string | null => {
  const ext = name.split(/[?#]/)[0]!.split('.').pop()?.toLowerCase();
  const table: Record<string, string> = {
    png: 'image/png',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    gif: 'image/gif',
    webp: 'image/webp',
    svg: 'image/svg+xml',
    bmp: 'image/bmp',
    avif: 'image/avif',
    heic: 'image/heic',
    mp4: 'video/mp4',
    m4v: 'video/x-m4v',
    webm: 'video/webm',
    mov: 'video/quicktime',
    mp3: 'audio/mpeg',
    m4a: 'audio/mp4',
    wav: 'audio/wav',
    ogg: 'audio/ogg',
    flac: 'audio/flac',
    aac: 'audio/aac',
  };
  return ext ? (table[ext] ?? null) : null;
};
export { mimeOf };

const nameOf = (s: MediaSource) =>
  (s.type === 'file' ? s.path : s.type === 'url' ? s.url.split(/[?#]/)[0]! : s.sha)
    .split('/')
    .pop() ?? '';

export const sourceId = (s: MediaSource) =>
  s.type === 'file' ? `file:${s.path}` : s.type === 'orochi-media' ? `m:${s.sha}` : `url:${s.url}`;

/** Every piece of media in a conversation in order: stored media items and message links. */
export function galleryOf(
  items: ItemRow[],
  ctx: { thread: string; cwd: string | null | undefined },
  extraText: { text: string; at: number }[] = [],
): GalleryItem[] {
  const seen = new Set<string>();
  const out: GalleryItem[] = [];
  const add = (g: GalleryItem) => {
    if (seen.has(g.id)) return;
    seen.add(g.id);
    out.push(g);
  };
  const texts = [
    ...items
      .filter((i) => i.kind === 'agent_message' || i.kind === 'user_message')
      .map((i) => ({ text: i.text, at: i.at })),
    ...extraText,
  ];
  const stored = items.filter((i) => i.kind === 'media' && i.data?.sha && i.data.kept !== false);
  const events: { at: number; run: () => void }[] = [
    ...stored.map((i) => ({
      at: i.at,
      run: () => {
        const d = i.data as MediaData;
        add({
          id: `m:${d.sha}`,
          kind: d.kind,
          name: d.name,
          mime: d.mime,
          bytes: d.bytes,
          source: { type: 'orochi-media', thread: ctx.thread, sha: d.sha },
          at: i.at,
        });
      },
    })),
    ...texts.map((tx) => ({
      at: tx.at,
      run: () => {
        for (const r of mediaLinks(tx.text, ctx)) {
          if (!r.media || !r.source) continue;
          add({
            id: sourceId(r.source),
            kind: r.media,
            name: nameOf(r.source) || r.label,
            mime: mimeOf(r.target),
            bytes: null,
            source: r.source,
            at: tx.at,
          });
        }
      },
    })),
  ].sort((a, b) => a.at - b.at);
  events.forEach((e) => e.run());
  return out;
}

export type FileView = 'text' | 'diff' | 'image' | 'video' | 'audio' | 'binary';

export function viewKindOf(path: string): FileView {
  if (/\.(png|jpe?g|gif|webp|svg|bmp|avif|heic)$/i.test(path)) return 'image';
  if (/\.(mp4|m4v|webm|mov)$/i.test(path)) return 'video';
  if (/\.(mp3|m4a|wav|ogg|flac|aac)$/i.test(path)) return 'audio';
  if (/\.(patch|diff)$/i.test(path)) return 'diff';
  if (/\.(docx?|xlsx?|pptx?|zip|gz|tar|bin|exe|dylib|so|o|a)$/i.test(path)) return 'binary';
  return 'text';
}
