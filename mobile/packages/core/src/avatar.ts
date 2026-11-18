/**
 * The Office's characters, as data. Everything here is pure: a name picks a look (the same name
 * always the same look), a state picks an expression. The drawing is the app's
 * (`apps/app/src/ui/office/`), the colours are the app's (`avatarPalette.ts`); these are indexes.
 */
import type { PersonState } from './office';

export type Kind = 'lead' | 'implementer' | 'reviewer' | 'sub' | 'other';

export type Accessory =
  'blazer' | 'headphones' | 'cap' | 'glasses' | 'scarf' | 'earring' | 'collar' | 'none';

/** How many of each part the drawing has: the ranges every index below stays within. */
export const COUNTS = { hair: 10, hairColor: 6, skin: 6, outfit: 8, ground: 6 } as const;

/** What a person of a kind can wear besides the mark of the role (kept small and plain). */
export const ACCESSORIES: Record<Kind, readonly Accessory[]> = {
  lead: ['blazer'],
  implementer: ['headphones', 'cap'],
  reviewer: ['glasses'],
  sub: ['none'],
  other: ['none', 'none', 'scarf', 'earring', 'collar'],
};

export interface Look {
  kind: Kind;
  hair: number;
  hairColor: number;
  skin: number;
  outfit: number;
  ground: number;
  accessory: Accessory;
}

/** FNV-1a: small, stable, and the same everywhere this runs. */
function hash(text: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

/** The role of a person, from what the desk calls it. */
export function kindOf(m: { role?: string | null; lead?: boolean; sub?: boolean }): Kind {
  if (m.sub) return 'sub';
  if (m.lead) return 'lead';
  const role = String(m.role ?? '').toLowerCase();
  if (/review|audit|qa\b|critic/.test(role)) return 'reviewer';
  if (/implement|develop|\bdev\b|build|code|coder|engineer|program/.test(role))
    return 'implementer';
  return 'other';
}

/** The look of a name: one index per part, each from its own hash so the parts vary freely. */
export function lookFor(seed: string, kind: Kind): Look {
  const pick = (part: string, n: number) => hash(`${seed}#${part}`) % n;
  const options = ACCESSORIES[kind];
  return {
    kind,
    hair: pick('hair', COUNTS.hair),
    hairColor: pick('hairColor', COUNTS.hairColor),
    skin: pick('skin', COUNTS.skin),
    outfit: pick('outfit', COUNTS.outfit),
    ground: pick('ground', COUNTS.ground),
    accessory: options[pick('accessory', options.length)]!,
  };
}

/** The same agent is the same face in every room: the name is the role and the agent. */
export const seedOf = (m: { role?: string | null; agent?: string | null }) =>
  `${m.role || ''}|${m.agent || ''}`;

/** Four quiet faces: eyes cast down, raised brows and a small round mouth, a deep smile, closed eyes. */
export type Expression = 'focus' | 'asking' | 'smile' | 'sleepy';

export interface Face {
  expression: Expression;
  /** Blinks now and then (a focused look). */
  blink: boolean;
  /** The mouth moves (it spoke a moment ago). */
  talking: boolean;
}

/** What a state looks like: working is focus, asking raises the brows, done is a smile. */
export function faceOf(state: PersonState, speaking: boolean): Face {
  const talking = speaking && state !== 'idle';
  switch (state) {
    case 'working':
      return { expression: 'focus', blink: true, talking };
    case 'asking':
    case 'mentioned':
      return { expression: 'asking', blink: false, talking };
    case 'reported':
    case 'leaving':
      return { expression: 'smile', blink: false, talking };
    case 'idle':
      return { expression: 'sleepy', blink: false, talking: false };
  }
}

/** Every state, for a preview and for a test that none is left without a face. */
export const STATES: readonly PersonState[] = [
  'working',
  'asking',
  'mentioned',
  'reported',
  'idle',
  'leaving',
];
