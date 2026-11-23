import { describe, expect, it } from 'vitest';
import type { RoutineRow } from '@orochi/protocol';
import { proposalWords } from '../src';

const routine = (over: Partial<RoutineRow>): RoutineRow => ({
  id: 'p1',
  thread_id: 'r1',
  thread_title: 'assistant',
  name: 'news',
  instruction: 'Brief me on the news',
  schedule: 'daily at 07:30',
  watch: null,
  every_secs: null,
  access: 'read',
  notify: 'mention',
  catch_up: 'once',
  made_by: 'agent',
  expires_at: null,
  paused: true,
  proposed: true,
  next_at: null,
  last_at: null,
  last_skipped: null,
  last_state: null,
  tokens_today: 0,
  ...over,
});

describe('a proposal', () => {
  it('says its time, or the command a watch would run', () => {
    expect(proposalWords(routine({}))).toEqual({ when: 'daily at 07:30', command: null });
    expect(
      proposalWords(routine({ schedule: null, watch: 'gh run list', every_secs: 900 })),
    ).toEqual({ when: 'every 15m', command: 'gh run list' });
  });
});
