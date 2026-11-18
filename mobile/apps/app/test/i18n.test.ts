/// <reference types="node" />
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { hasTranslation } from '@orochi/core';

const walk = (dir: string): string[] =>
  readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? walk(p) : /\.tsx?$/.test(p) ? [p] : [];
  });

describe('every string the screens pass to t() has a Japanese translation', () => {
  const missing: string[] = [];
  for (const root of ['app', 'src'])
    for (const file of walk(join(__dirname, '..', root))) {
      const source = readFileSync(file, 'utf8');
      for (const m of source.matchAll(/\bt\(\s*'((?:[^'\\]|\\.)*)'/g)) {
        const key = m[1]!.replace(/\\'/g, "'");
        if (!hasTranslation(key)) missing.push(`${file.split('/apps/app/')[1]}: ${key}`);
      }
    }
  it('has none missing', () => expect(missing).toEqual([]));
});
