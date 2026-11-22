import { describe, expect, it } from 'vitest';
import type { ItemRow, RoutineRow, Thread } from '@orochi/protocol';
import { buildChat, folded, proposalWords, routineRows, sourcesOf, wokenBy } from '../src';

let seq = 0;
const item = (over: Partial<ItemRow>): ItemRow => ({
  seq: ++seq,
  turn: 't1',
  turn_ordinal: 1,
  lane: null,
  role: null,
  agent: 'claude',
  model: 'opus',
  kind: 'agent_message',
  status: null,
  text: '',
  data: null,
  truncated: false,
  patches: 0,
  at: 1_000 + seq,
  failed: false,
  turn_origin: null,
  provider: 'anthropic',
  ...over,
});

const thread = (items: ItemRow[], status = 'idle'): Thread =>
  ({
    thread: { id: 'r1', status, cwd: '/home/me/assistant', agents: [] },
    items,
    seats: [],
    files: [],
  }) as unknown as Thread;

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

describe('a resident conversation as a messenger', () => {
  it('shows the person and the assistant as bubbles, and what it read as links under the answer', () => {
    const chat = buildChat({
      thread: thread([
        item({ kind: 'user_message', text: '引っ越し先の住所教えて', agent: null }),
        item({ kind: 'route', data: { agent: 'claude' } }),
        item({
          kind: 'tool_call',
          data: {
            tool_kind: 'read',
            title: 'Read quote.pdf',
            locations: [{ path: '/home/me/mail/quote.pdf', line: null }],
          },
        }),
        item({
          kind: 'tool_call',
          data: { tool_kind: 'fetch', raw_input: { url: 'https://www.example.com/move?id=1' } },
        }),
        item({ kind: 'tool_call', data: { tool_kind: 'execute', raw_input: { command: 'ls' } } }),
        item({ kind: 'agent_message', text: '東京都北区中里3丁目' }),
      ]),
      said: [],
      members: [],
    });
    const kinds = chat.filter((b) => b.type !== 'day').map((b) => b.type);
    expect(kinds).toEqual(['me', 'agent']);
    const answer = chat.find((b) => b.type === 'agent');
    if (answer?.type !== 'agent') throw new Error();
    expect(answer.sources).toEqual([
      { kind: 'file', target: '/home/me/mail/quote.pdf', label: 'quote.pdf' },
      { kind: 'web', target: 'https://www.example.com/move?id=1', label: 'example.com' },
    ]);
  });

  it('draws a routine wake as a line naming the routine, not as something the person said', () => {
    const chat = buildChat({
      thread: thread([
        item({
          kind: 'user_message',
          turn_origin: 'routine',
          agent: null,
          text: 'You are assistant. \n\nThis turn was started by a routine, "Morning brief" (due 2026-11-21 08:00). Nobody is at the keyboard.',
        }),
        item({ kind: 'agent_message', text: 'Two meetings today.' }),
      ]),
      said: [],
      members: [],
    });
    const woke = chat.find((b) => b.type === 'woke');
    expect(woke).toMatchObject({ type: 'woke', routine: 'Morning brief' });
    expect(chat.some((b) => b.type === 'me')).toBe(false);
  });

  it('joins a reply streamed as several messages into one bubble', () => {
    const chat = buildChat({
      thread: thread([
        item({ kind: 'user_message', text: 'hi', agent: null }),
        item({ kind: 'agent_message', text: 'One.' }),
        item({ kind: 'agent_message', text: 'Two.' }),
      ]),
      said: [],
      members: [],
    });
    const answers = chat.filter((b) => b.type === 'agent');
    expect(answers).toHaveLength(1);
    if (answers[0]?.type === 'agent') expect(answers[0].text).toBe('One.\n\nTwo.');
  });

  it('puts what the agent proposed as a card for the person, and only this conversation’s', () => {
    const chat = buildChat({
      thread: thread([item({ kind: 'agent_message', text: '提案しました' })], 'working'),
      said: [],
      members: [],
      routines: [
        routine({}),
        routine({ id: 'mine', proposed: false, paused: false, made_by: 'user' }),
        routine({ id: 'other', thread_id: 'r2' }),
      ],
    });
    expect(chat.filter((b) => b.type === 'proposal').map((b) => b.key)).toEqual(['prp1']);
    expect(chat.at(-1)?.type).toBe('typing');
  });
});

describe('the pieces of a bubble', () => {
  it('folds a long reply at a sentence end and leaves a short one whole', () => {
    expect(folded('short')).toEqual({ head: 'short', more: false });
    const long = `${'あ'.repeat(200)}。${'い'.repeat(300)}`;
    const f = folded(long);
    expect(f.more).toBe(true);
    expect(f.head).toBe(`${'あ'.repeat(200)}。…`);
  });

  it('says a proposal’s time, or the command a watch would run', () => {
    expect(proposalWords(routine({}))).toEqual({ when: 'daily at 07:30', command: null });
    expect(
      proposalWords(routine({ schedule: null, watch: 'gh run list', every_secs: 900 })),
    ).toEqual({ when: 'every 15m', command: 'gh run list' });
  });

  it('reads only reads and fetches as sources, and only web addresses as pages', () => {
    expect(sourcesOf(item({ kind: 'tool_call', data: { tool_kind: 'edit' } }))).toEqual([]);
    expect(
      sourcesOf(
        item({
          kind: 'tool_call',
          data: { tool_kind: 'fetch', raw_input: { url: 'file:///etc/passwd' } },
        }),
      ),
    ).toEqual([]);
    expect(wokenBy('nothing here')).toBeNull();
  });
});

describe('the list of what an assistant has scheduled', () => {
  it('is its running routines, soonest first, held ones last, and never a proposal', () => {
    const r = (over: Partial<RoutineRow>) =>
      routine({ next_at: 100, proposed: false, paused: false, ...over });
    const rows = routineRows(
      [
        r({ id: 'held', paused: true, next_at: 50 }),
        r({ id: 'late', next_at: 300 }),
        r({ id: 'soon', next_at: 200 }),
        r({ id: 'ask', proposed: true }),
        r({ id: 'other', thread_id: 'elsewhere' }),
      ],
      routine({}).thread_id,
    );
    expect(rows.map((x) => x.id)).toEqual(['soon', 'late', 'held']);
  });
});
