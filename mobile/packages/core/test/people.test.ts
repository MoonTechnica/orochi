import { describe, expect, it } from 'vitest';
import { kindOf, lookFor, seedOf } from '../src/avatar';
import {
  isAnonymous,
  leadRoleOf,
  lookOfWho,
  personStateOf,
  whoKey,
  whoNamed,
  whoOf,
} from '../src/people';

const thread = {
  agents: [
    { role: 'lead', agent: 'claude', provider: 'anthropic', lead: true },
    { role: 'implementer', agent: 'opencode', provider: 'opencode', lead: false },
  ],
} as any;

describe('people', () => {
  it('a chat speaker has the Office look for the same role and agent', () => {
    const w = whoOf({ role: 'implementer', agent: 'opencode', provider: 'opencode' }, 'lead');
    expect(lookOfWho(w)).toEqual(
      lookFor(seedOf({ role: 'implementer', agent: 'opencode' }), kindOf({ role: 'implementer' })),
    );
  });
  it('the lead is the lead kind, wherever the thread names its role', () => {
    expect(leadRoleOf(thread)).toBe('lead');
    expect(whoOf({ role: 'fixer', agent: 'x' }, 'fixer').lead).toBe(true);
    expect(lookOfWho(whoOf({ role: 'lead', agent: 'claude' }, 'lead')).kind).toBe('lead');
  });
  it('a name resolves through members, then the thread agents', () => {
    const w = whoNamed('lead', [], thread);
    expect(w).toMatchObject({ role: 'lead', agent: 'claude', lead: true, you: false });
    expect(whoNamed('Implementer', [], thread).agent).toBe('opencode');
  });
  it('you is a calm character, never an agent look, stable per seed', () => {
    const me = whoOf({ mine: true, name: 'you' }, 'lead');
    expect(me.you).toBe(true);
    const a = lookOfWho(me, 'seed-1');
    expect(a).toEqual(lookOfWho(me, 'seed-1'));
    expect(a.accessory).toBe('none');
    expect(a.kind).toBe('other');
    expect(whoKey(me)).toBe('you');
  });
  it('unplaceable speakers are anonymous', () => {
    expect(isAnonymous(whoOf({ name: 'mailbox' }, 'lead'))).toBe(true);
    expect(isAnonymous(whoOf({ role: 'x' }, 'lead'))).toBe(false);
  });
  it('member states map onto the Office states', () => {
    expect(personStateOf({ state: 'asking', live: true } as any)).toBe('asking');
    expect(personStateOf({ state: 'working', live: true } as any)).toBe('working');
    expect(personStateOf({ state: 'done', live: false, reported: true } as any)).toBe('reported');
    expect(personStateOf({ state: 'failed', live: false } as any)).toBe('idle');
  });
});
