import { describe, expect, it } from 'vitest';
import {
  parseDiff,
  routeChoices,
  routeWith,
  sortTree,
  crumbs,
  fileKind,
  noticesFor,
  snapshotOf,
} from '../src';

describe('diff', () => {
  it('numbers the new side and skips deletions', () => {
    const d = parseDiff('diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n');
    expect(d.filter((l) => l.line !== null).map((l) => [l.kind, l.line])).toEqual([
      ['ctx', 1],
      ['add', 2],
      ['ctx', 3],
    ]);
    expect(d[0]!.kind).toBe('meta');
  });
});

describe('files and routes', () => {
  it('sorts directories first and builds crumbs', () => {
    const e = (name: string, kind: any) => ({ name, path: name, kind }) as any;
    expect(
      sortTree([e('b.rs', 'file'), e('src', 'dir'), e('A.md', 'file')]).map((x) => x.name),
    ).toEqual(['src', 'A.md', 'b.rs']);
    expect(crumbs('a/b')).toEqual([
      { name: 'a', path: 'a' },
      { name: 'b', path: 'a/b' },
    ]);
    expect(fileKind('x/y.PNG')).toBe('image');
  });
  it('offers auto, routes seen and agents', () => {
    const c = routeChoices(
      [
        { agent: 'claude', model: 'opus' },
        { agent: 'claude', model: 'opus' },
      ],
      [{ agent: 'claude' }, { agent: 'codex' }],
    );
    expect(c.map((x) => x.route)).toEqual([null, 'claude/opus', 'claude', 'codex']);
    expect(routeWith('claude', 'opus', 'high')).toBe('claude/opus high');
  });
});

describe('notices', () => {
  const th = (id: string, status: string) => ({ id, title: id, status, updated_at: 5 }) as any;
  const proj = (s: string) => [{ threads: [th('t1', s)] }] as any;
  it('announces nothing the first time and what is new after', () => {
    expect(noticesFor(null, proj('idle'), [], [])).toEqual([]);
    const before = snapshotOf(proj('working'), [], []);
    const prompt = {
      id: 'p1',
      thread_id: 't1',
      thread_title: 'T',
      project: 'P',
      title: 'Run?',
    } as any;
    const out = noticesFor(before, proj('idle'), [prompt], []);
    expect(out.map((n) => n.kind)).toEqual(['prompt']);
    const done = noticesFor(before, proj('idle'), [], []);
    expect(done.map((n) => n.kind)).toEqual(['finished']);
    expect(noticesFor(snapshotOf(proj('idle'), [], []), proj('idle'), [], [])).toEqual([]);
  });
});
