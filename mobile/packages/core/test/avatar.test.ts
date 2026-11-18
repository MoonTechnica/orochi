import { describe, expect, it } from 'vitest';
import {
  ACCESSORIES,
  COUNTS,
  STATES,
  faceOf,
  kindOf,
  lookFor,
  seedOf,
  type Kind,
} from '../src/avatar';

const ROLES = [
  'lead',
  'implementer',
  'reviewer',
  'designer',
  'tester',
  'docs',
  'researcher',
  'planner',
];
const AGENTS = ['claude', 'codex', 'gemini', 'opencode', 'sonnet', 'opus', 'gpt-6.1-sol'];
const keys = ROLES.flatMap((r) => AGENTS.map((a) => seedOf({ role: r, agent: a })));

describe('a name is always the same face', () => {
  it('is deterministic', () => {
    for (const k of keys) expect(lookFor(k, 'other')).toEqual(lookFor(k, 'other'));
    expect(lookFor('lead|claude', 'lead')).toEqual(lookFor('lead|claude', 'lead'));
  });
  it('stays inside the parts the drawing has', () => {
    for (const k of keys)
      for (const kind of ['lead', 'implementer', 'reviewer', 'sub', 'other'] as Kind[]) {
        const l = lookFor(k, kind);
        expect(l.hair).toBeLessThan(COUNTS.hair);
        expect(l.hairColor).toBeLessThan(COUNTS.hairColor);
        expect(l.skin).toBeLessThan(COUNTS.skin);
        expect(l.outfit).toBeLessThan(COUNTS.outfit);
        expect(l.ground).toBeLessThan(COUNTS.ground);
        expect(ACCESSORIES[kind]).toContain(l.accessory);
      }
  });
  it('gives different names many different looks (well over thirty)', () => {
    const seen = new Set(
      keys.map((k) => JSON.stringify(lookFor(k, kindOf({ role: k.split('|')[0] })))),
    );
    expect(seen.size).toBeGreaterThanOrEqual(30);
    expect(seen.size).toBeGreaterThan(keys.length * 0.9);
  });
  it('uses every part somewhere', () => {
    const many = Array.from({ length: 2000 }, (_, i) => `agent-${i}|m${i * 7}`);
    for (const kind of Object.keys(ACCESSORIES) as Kind[]) {
      const looks = many.map((k) => lookFor(k, kind));
      for (const [part, n] of Object.entries(COUNTS))
        expect(
          new Set(looks.map((l) => l[part as keyof typeof COUNTS])).size,
          `${kind} ${part}`,
        ).toBe(n);
      expect(new Set(looks.map((l) => l.accessory)).size).toBe(new Set(ACCESSORIES[kind]).size);
    }
  });
});

describe('roles are told apart', () => {
  it('reads the kind from the role and the lead flag', () => {
    expect(kindOf({ role: 'anything', lead: true })).toBe('lead');
    expect(kindOf({ role: 'Reviewer' })).toBe('reviewer');
    expect(kindOf({ role: 'security-audit' })).toBe('reviewer');
    expect(kindOf({ role: 'implementer' })).toBe('implementer');
    expect(kindOf({ role: 'backend-dev' })).toBe('implementer');
    expect(kindOf({ role: 'designer' })).toBe('other');
    expect(kindOf({ role: 'x', sub: true })).toBe('sub');
    expect(kindOf({})).toBe('other');
  });
  it('marks the role quietly: a blazer, headphones or a cap, thin glasses; nothing on a sub-agent', () => {
    expect(lookFor('a', 'lead').accessory).toBe('blazer');
    expect(lookFor('a', 'reviewer').accessory).toBe('glasses');
    expect(['headphones', 'cap']).toContain(lookFor('a', 'implementer').accessory);
    expect(lookFor('a', 'sub').accessory).toBe('none');
  });
});

describe('a state is an expression', () => {
  it('has a face for every state', () => {
    for (const st of STATES) {
      const f = faceOf(st, false);
      expect(['focus', 'asking', 'smile', 'sleepy']).toContain(f.expression);
    }
  });
  it('works focused and blinking, asks with raised brows, smiles when done, sleeps when idle', () => {
    expect(faceOf('working', false)).toMatchObject({ expression: 'focus', blink: true });
    expect(faceOf('asking', false).expression).toBe('asking');
    expect(faceOf('mentioned', false).expression).toBe('asking');
    expect(faceOf('reported', false).expression).toBe('smile');
    expect(faceOf('leaving', false).expression).toBe('smile');
    expect(faceOf('idle', false).expression).toBe('sleepy');
  });
  it('talks when it spoke, but never in its sleep', () => {
    expect(faceOf('working', true).talking).toBe(true);
    expect(faceOf('asking', true).talking).toBe(true);
    expect(faceOf('idle', true).talking).toBe(false);
    expect(faceOf('working', false).talking).toBe(false);
  });
});
