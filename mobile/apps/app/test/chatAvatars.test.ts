import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const read = (f: string) => readFileSync(join(__dirname, '..', f), 'utf8');

describe('faces outside the Office', () => {
  it('the letter placeholder is gone and the places that show people draw the Office characters', () => {
    expect(existsSync(join(__dirname, '..', 'src/ui/Face.tsx'))).toBe(false);
    for (const f of [
      'src/ui/Entries.tsx',
      'src/ui/Composer.tsx',
      'src/ui/Threads.tsx',
      'src/ui/home/Needs.tsx',
      'app/(workspace)/(sections)/activity.tsx',
      'app/(workspace)/(sections)/conversations.tsx',
      'app/(workspace)/thread/[id]/member/[name].tsx',
    ])
      expect(read(f), f).toMatch(/WhoFace|LiveFace/);
  });
  it('the conversation face is static: no reanimated hooks, no palette of its own', () => {
    const src = read('src/ui/people/WhoFace.tsx');
    expect(src).not.toMatch(/reanimated|useSharedValue|useAnimated/);
    expect(src).not.toMatch(/avatarPalette/);
    expect(src).toMatch(/\bstill\b/);
  });
  it('only the live face (bar, panel head) may move, and only through the Office avatar', () => {
    const users = [
      'src/ui/Entries.tsx',
      'src/ui/Composer.tsx',
      'src/ui/Threads.tsx',
      'src/ui/ThreadMenu.tsx',
      'src/ui/home/Needs.tsx',
      'app/(workspace)/(sections)/conversations.tsx',
    ];
    for (const f of users) expect(read(f), f).not.toMatch(/office\/Avatar'/);
  });
});
