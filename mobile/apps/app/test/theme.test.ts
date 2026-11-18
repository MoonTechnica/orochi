/// <reference types="node" />
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { palette, type Colors } from '../src/ui/theme';

const channel = (hex: string, at: number) => {
  const v = parseInt(hex.slice(at, at + 2), 16) / 255;
  return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
};
const luminance = (hex: string) =>
  0.2126 * channel(hex, 1) + 0.7152 * channel(hex, 3) + 0.0722 * channel(hex, 5);
export const contrast = (a: string, b: string) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi! + 0.05) / (lo! + 0.05);
};

/** Every colour that is drawn as text, and the surfaces it is drawn on. */
const TEXT: (keyof Colors)[] = ['text', 'sub', 'dangerInk'];
const SURFACES: (keyof Colors)[] = ['bg', 'card', 'raised', 'mine', 'code', 'mark'];

describe('WCAG AA (4.5:1) for text colours on every surface, in both modes', () => {
  for (const mode of ['light', 'dark'] as const) {
    const p = palette[mode];
    for (const fg of TEXT)
      for (const bg of SURFACES)
        it(`${mode}: ${fg} on ${bg}`, () => {
          expect(contrast(p[fg], p[bg])).toBeGreaterThanOrEqual(4.5);
        });
    it(`${mode}: label colours on the fills they sit on`, () => {
      expect(contrast(p.onPrimary, p.primary)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(p.onPrimary, p.attention)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(p.onDanger, p.danger)).toBeGreaterThanOrEqual(4.5);
      // the badge digit is drawn in the page colour on the attention fill
      expect(contrast(p.bg, p.attention)).toBeGreaterThanOrEqual(4.5);
    });
  }
  it('the plain red is a fill: red text on a dark card needs the lifted dangerInk', () => {
    expect(contrast(palette.dark.danger, palette.dark.card)).toBeLessThan(4.5);
    expect(palette.dark.dangerInk).not.toBe(palette.dark.danger);
  });
});

describe('one meaning for red', () => {
  const red = (hex: string) => {
    const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
    return Math.max(r!, g!, b!) - Math.min(r!, g!, b!) > 8;
  };
  it('only danger, dangerInk and brandAccent are coloured; everything else is grey', () => {
    for (const mode of ['light', 'dark'] as const) {
      const coloured = (Object.keys(palette[mode]) as (keyof Colors)[]).filter((k) =>
        red(palette[mode][k]),
      );
      expect(coloured.sort()).toEqual(['brandAccent', 'danger', 'dangerInk']);
    }
  });
  it('the main action and what waits for you are the inverted black/white, not red', () => {
    expect(palette.dark.primary).toBe(palette.dark.text);
    expect(palette.dark.attention).toBe(palette.dark.text);
    expect(palette.light.primary).toBe(palette.light.text);
    expect(palette.light.attention).toBe(palette.light.text);
  });

  const walk = (dir: string): string[] =>
    readdirSync(dir).flatMap((f) => {
      const p = join(dir, f);
      return statSync(p).isDirectory() ? walk(p) : /\.tsx?$/.test(p) ? [p] : [];
    });
  const files = ['app', 'src'].flatMap((d) => walk(join(__dirname, '..', d)));
  const usedIn = (pattern: RegExp) =>
    files
      .filter((f) => !f.endsWith('theme.ts') && pattern.test(readFileSync(f, 'utf8')))
      .map((f) => f.split('/apps/app/')[1]!)
      .sort();

  // Where the red tokens may appear: error text, failed checks, destructive buttons and the
  // stop button, and nowhere else. Adding a file here is a decision about what red means.
  const DANGER = [
    'app/(tabs)/settings.tsx',
    'app/pair.tsx',
    'app/thread/[id]/files.tsx',
    'app/thread/[id]/work.tsx',
    'src/ui/Composer.tsx',
    'src/ui/Drawer.tsx',
    'src/ui/Entries.tsx',
    'src/ui/Machines.tsx',
    'src/ui/NewThread.tsx',
    'src/ui/Sheet.tsx',
    'src/ui/home/Needs.tsx',
    'src/ui/office/SeatSheet.tsx',
  ];
  it('the danger tokens are read only where an error or a destructive action is drawn', () => {
    expect(usedIn(/colors\.(danger|dangerInk|onDanger)\b/)).toEqual([...DANGER].sort());
  });
  it('the brand red is reserved for a few pixels of the icon, and no screen uses it yet', () => {
    // A red dot in a serpent head read as an error in review, so it was left out: red means error.
    expect(usedIn(/brandAccent/)).toEqual([]);
  });
  // The one exception to "every colour is a token": the Office's people and the provider's badge
  // are drawn in colour, as on the desktop, and their colours live in this file alone.
  const ILLUSTRATION = 'src/ui/office/avatarPalette.ts';
  const rel = (f: string) => f.split('/apps/app/')[1]!;
  it('colour literals (hex or hsl) exist only in the theme and the illustration palette', () => {
    const literal = files
      .filter((f) => !f.endsWith('src/ui/theme.ts'))
      .filter((f) => /#[0-9a-fA-F]{6}\b|hsla?\(/.test(readFileSync(f, 'utf8')))
      .map(rel);
    expect(literal).toEqual([ILLUSTRATION]);
  });
  it('the illustration palette is read only by the avatar, its character, its parts and nothing in the chrome', () => {
    const readers = files
      .filter(
        (f) => !f.endsWith('avatarPalette.ts') && /avatarPalette/.test(readFileSync(f, 'utf8')),
      )
      .map(rel)
      .sort();
    expect(readers).toEqual([
      'src/ui/office/Avatar.tsx',
      'src/ui/office/Character.tsx',
      'src/ui/office/parts/Accessory.tsx',
      'src/ui/office/parts/Hair.tsx',
    ]);
  });
  it('the avatar is always drawn inside a solid rim in the text colour', () => {
    const avatar = readFileSync(join(__dirname, '..', 'src/ui/office/Avatar.tsx'), 'utf8');
    expect(avatar).toMatch(/borderWidth: 1\.5,\s*borderColor: colors\.text/);
  });
});
