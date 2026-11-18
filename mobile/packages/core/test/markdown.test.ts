import { describe, expect, it } from 'vitest';
import {
  applyMention,
  fileTarget,
  galleryOf,
  mediaKind,
  mediaLinks,
  mentionCandidates,
  mentionQuery,
  mentionsIn,
  orochiMedia,
  parseMarkdown,
  resolveLink,
  t,
  toMs,
  viewKindOf,
  since,
  hasTranslation,
  translationKeys,
} from '../src';
import { fixture } from './fixtures';

const ctx = { thread: 'th1', cwd: '/work/repo' };
const SHA = 'a'.repeat(64);

describe('markdown', () => {
  it('parses blocks and inline', () => {
    const blocks = parseMarkdown(
      '# Title\n\nsome **bold** and `code` @lead\n\n- a\n- b\n\n```py\nx=1\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |',
    );
    expect(blocks.map((b) => b.t)).toEqual(['heading', 'p', 'list', 'code', 'table']);
    const p = blocks[1]!;
    if (p.t !== 'p') throw new Error();
    expect(p.lines[0]!.map((i) => i.t)).toEqual([
      'text',
      'strong',
      'text',
      'code',
      'text',
      'mention',
    ]);
  });
});

describe('media link resolution', () => {
  it('relative image markdown resolves to a file of the thread folder', () => {
    const r = resolveLink('Screenshot', 'artifacts/shot.png', true, ctx);
    expect(r.media).toBe('image');
    expect(r.source).toEqual({ type: 'file', path: 'artifacts/shot.png', line: null });
  });
  it('absolute paths inside the folder become relative, outside are refused', () => {
    expect(resolveLink('x', '/work/repo/a/b.png', true, ctx).source).toEqual({
      type: 'file',
      path: 'a/b.png',
      line: null,
    });
    expect(resolveLink('x', '/etc/passwd.png', true, ctx).source).toBeNull();
    expect(resolveLink('x', 'file:///work/repo/a/b.mp4', false, ctx)).toMatchObject({
      media: 'video',
      source: { type: 'file', path: 'a/b.mp4' },
    });
  });
  it('rejects traversal and .git', () => {
    expect(fileTarget('../secret.png', ctx.cwd)).toBeNull();
    expect(fileTarget('.git/config', ctx.cwd)).toBeNull();
  });
  it('reads :line and #L positions on a file link', () => {
    expect(fileTarget('src/a.rs:42', ctx.cwd)).toEqual({ path: 'src/a.rs', line: 42 });
    expect(fileTarget('src/a.rs#L7', ctx.cwd)).toEqual({ path: 'src/a.rs', line: 7 });
  });
  it('orochi-media hashes map to the media route', () => {
    expect(orochiMedia(`orochi-media://localhost/th9/${SHA}`, 'th1')).toEqual({
      thread: 'th9',
      sha: SHA,
    });
    expect(orochiMedia(`http://orochi-media.localhost/${SHA}`, 'th1')).toEqual({
      thread: 'th1',
      sha: SHA,
    });
    expect(resolveLink('pic', `orochi-media://localhost/th1/${SHA}`, true, ctx).source).toEqual({
      type: 'orochi-media',
      thread: 'th1',
      sha: SHA,
    });
  });
  it('https images show, plain http (non-loopback) does not, links stay links', () => {
    expect(resolveLink('i', 'https://x.test/a.png', false, ctx)).toMatchObject({
      media: 'image',
      source: { type: 'url' },
    });
    expect(resolveLink('i', 'http://evil.test/a.png', true, ctx).media).toBeNull();
    expect(resolveLink('i', 'http://localhost:3000/a.png', true, ctx).media).toBe('image');
    expect(resolveLink('docs', 'https://x.test/page', false, ctx)).toMatchObject({
      media: null,
      source: { type: 'url' },
    });
  });
  it('kinds by extension', () => {
    expect(mediaKind('a.MP4?x=1')).toBe('video');
    expect(mediaKind('a.m4a')).toBe('audio');
    expect(mediaKind('a.txt')).toBeNull();
  });
  it('collects every media link in a message', () => {
    const links = mediaLinks('![a](one.png)\n\ntext [clip](v/clip.mov) and [doc](README.md)', ctx);
    expect(links.map((l) => l.media)).toEqual(['image', 'video']);
  });
  it('builds a gallery from stored media items and message links without duplicates', () => {
    const items: any[] = [
      { seq: 1, kind: 'agent_message', text: '![a](one.png) and again ![a](one.png)', at: 1 },
      {
        seq: 2,
        kind: 'media',
        at: 2,
        data: {
          sha: SHA,
          kind: 'video',
          mime: 'video/mp4',
          bytes: 10,
          name: 'rec.mp4',
          kept: true,
        },
      },
      {
        seq: 3,
        kind: 'media',
        at: 3,
        data: {
          sha: 'b'.repeat(64),
          kind: 'image',
          mime: 'image/png',
          bytes: 1,
          name: 'gone.png',
          kept: false,
        },
      },
    ];
    const g = galleryOf(items, ctx);
    expect(g.map((x) => x.name)).toEqual(['one.png', 'rec.mp4']);
  });
  it('picks a viewer for a file', () => {
    expect(viewKindOf('a.png')).toBe('image');
    expect(viewKindOf('a.patch')).toBe('diff');
    expect(viewKindOf('deck.pptx')).toBe('binary');
    expect(viewKindOf('main.rs')).toBe('text');
  });
});

describe('files fixture', () => {
  it('knows text and binary read results', () => {
    const files = fixture('files');
    expect(files.binary.binary).toBe(true);
    expect(files.text.status).toBe('modified');
  });
});

describe('mentions', () => {
  const channel = fixture('channel');
  it('finds the @word at the caret', () => {
    expect(mentionQuery('hi @im')).toEqual({ query: 'im', start: 3, caret: 6 });
    expect(mentionQuery('hi @im there', 6)?.query).toBe('im');
    expect(mentionQuery('email a@b')).toBeNull();
  });
  it('lists members working first, then the lead, then @channel', () => {
    const all = mentionCandidates('', channel.members, channel.thread).map((c) => c.name);
    expect(all[0]).toBe('reviewer');
    expect([...all].sort()).toEqual(['channel', 'implementer', 'lead', 'reviewer']);
    expect(all.at(-1)).toBe('channel');
    expect(mentionCandidates('imp', channel.members, channel.thread).map((c) => c.name)).toEqual([
      'implementer',
    ]);
    expect(mentionCandidates('ch', channel.members, channel.thread).at(-1)!.channel).toBe(true);
  });
  it('inserts a completion', () => {
    const q = mentionQuery('ask @im')!;
    expect(applyMention('ask @im', q, 'implementer')).toEqual({
      value: 'ask @implementer ',
      caret: 17,
    });
  });
  it('only names that exist count as mentions', () => {
    expect(mentionsIn('@lead and @ghost and @you', ['lead'])).toEqual(['lead', 'you']);
  });
});

describe('time and i18n', () => {
  it('normalises seconds and milliseconds', () => {
    expect(toMs(1790000080)).toBe(1790000080000);
    expect(toMs(1790000080000)).toBe(1790000080000);
    expect(since(1790000000000, 1790000090000)).toBe('1m');
  });
  it('translates to Japanese and falls back to the key', () => {
    expect(t('Settings', 'ja')).toBe('設定');
    expect(t('Settings', 'en')).toBe('Settings');
    expect(t('no such key', 'ja')).toBe('no such key');
    expect(translationKeys().every((k) => hasTranslation(k))).toBe(true);
  });
});
