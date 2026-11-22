// A resident for the mock desktop: an assistant's conversation with a routine's wake, what it
// read to answer, and a proposal waiting to be allowed — what the Assistant tab shows. Asking it
// for something "every morning" (毎朝) makes it propose a routine, as a real one would.

const item = (now, seq, over) => ({
  seq,
  turn: 'a1',
  turn_ordinal: 1,
  lane: null,
  role: null,
  agent: null,
  model: null,
  kind: 'user_message',
  status: null,
  text: '',
  data: null,
  truncated: false,
  patches: 0,
  at: now,
  failed: false,
  turn_origin: null,
  provider: null,
  member: null,
  ...over,
});
const said = (now, seq, turn, text) =>
  item(now, seq, {
    turn,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'agent_message',
    status: 'completed',
    text,
    provider: 'anthropic',
  });
const read = (now, seq, turn, path) =>
  item(now, seq, {
    turn,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    kind: 'tool_call',
    status: 'completed',
    text: '',
    data: { tool_kind: 'read', title: `Read ${path}`, locations: [{ path, line: null }] },
    provider: 'anthropic',
  });

export function installResidents({ projects, threads, changed, sync, rpc }) {
  const now = Date.now();
  const ID = 'resident-assistant-0001';
  const home = '/Users/me/.local/share/orochi/residents/assistant';
  const row = {
    ...structuredClone(projects[0].threads[0]),
    id: ID,
    title: 'assistant',
    project: 'assistant',
    project_id: 'residents',
    cwd: home,
    project_root: home,
    status: 'idle',
    asking: 0,
    unread: 1,
    mentions: 0,
    pinned: true,
    updated_at: now - 60000,
    agents: (projects[0].threads[0].agents ?? []).slice(0, 1),
  };
  projects.push({
    id: 'residents',
    name: 'assistant',
    root: home,
    pinned: true,
    collapsed: false,
    missing: false,
    threads: [row],
  });
  const day = 86_400_000;
  threads.set(ID, {
    thread: row,
    seats: [],
    files: [],
    items: [
      item(now - day - 3600_000, 1, { turn: 'a1', text: '引っ越し先の住所教えて' }),
      read(now - day - 3590_000, 2, 'a1', `${home}/mail/SOLID HOUSE 火災保険見積書.pdf`),
      item(now - day - 3580_000, 3, {
        turn: 'a1',
        lane: 0,
        role: 'lead',
        agent: 'claude',
        kind: 'tool_call',
        status: 'completed',
        data: { tool_kind: 'fetch', raw_input: { url: 'https://www.city.kita.tokyo.jp/' } },
        provider: 'anthropic',
      }),
      said(
        now - day - 3570_000,
        4,
        'a1',
        '東京都北区中里3丁目19番13-2号\n\nSOLID HOUSE から届いた火災保険の見積書の「保険の対象の所在地」で確認しました。住所の表記は「上中里」ではなく「中里」です。',
      ),
      item(now - 3 * 3600_000, 5, {
        turn: 'a2',
        turn_origin: 'routine',
        text: 'You are assistant, a resident assistant that wakes on its own.\n\nThis turn was started by a routine, "Morning brief" (due 2026-11-21 08:00). Nobody is at the keyboard.',
      }),
      read(now - 3 * 3600_000 + 4000, 6, 'a2', `${home}/calendar/today.ics`),
      said(
        now - 3 * 3600_000 + 9000,
        7,
        'a2',
        'おはようございます。今日の予定は 2 件です。\n\n- 10:00 内覧の立ち会い（中里3丁目）\n- 15:00 保険の担当者と電話\n\n11/25（水）までに応募と録画提出の両方が必要です。5分以内・字幕だけでも可で、11/29（日）15:00〜16:00 に主催が再生します。最大8枠で、超えたら出展者優先。残枠は未確認です。公開版は macOS のみで、Windows・iOS 限定の代理展示には使えません。録画は先週の版で足りるので、新しいビルドは不要です。',
      ),
      item(now - 120_000, 8, { turn: 'a3', text: '毎朝7時半にニュースをまとめて教えて' }),
      said(
        now - 110_000,
        9,
        'a3',
        '毎朝 7:30 にニュースをまとめる予定を提案しました。下の「許可」を押すと始まります。',
      ),
    ],
  });

  const residents = [
    {
      thread_id: ID,
      name: 'assistant',
      home,
      instruction: 'My assistant: mail, calendar and the move.',
      access: 'read',
      paused: false,
      created_at: Math.floor((now - 30 * day) / 1000),
      next_at: Math.floor(now / 1000) + 5400,
      routines: 2,
      asking: 0,
      working: false,
    },
  ];
  const base = {
    thread_id: ID,
    thread_title: 'assistant',
    access: 'read',
    notify: 'always',
    catch_up: 'once',
    expires_at: null,
    last_skipped: null,
    tokens_today: 0,
    every_secs: null,
    watch: null,
  };
  let routines = [
    {
      ...base,
      id: 'routine-morning',
      name: 'Morning brief',
      instruction: 'Calendar, mail and anything due this week.',
      schedule: 'daily at 08:00',
      made_by: 'agent',
      paused: false,
      proposed: false,
      next_at: Math.floor(now / 1000) + 5400,
      last_at: Math.floor((now - 3 * 3600_000) / 1000),
      last_state: 'completed',
    },
    {
      ...base,
      id: 'routine-news',
      name: 'news',
      instruction: 'ニュースを 5 本にまとめて伝える',
      schedule: 'daily at 07:30',
      made_by: 'agent',
      paused: true,
      proposed: true,
      expires_at: Math.floor(now / 1000) + 7 * 86400,
      next_at: null,
      last_at: null,
      last_state: null,
    },
  ];

  const send = rpc.send;
  Object.assign(rpc, {
    residents: () => residents,
    routines: () => routines,
    routine_firings: () => [],
    notices: () => [],
    keeper: () => ({
      running: true,
      enabled: true,
      login: false,
      tokens_today: 41000,
      daily_tokens: 300000,
      next_at: Math.floor(now / 1000) + 5400,
    }),
    read_notices: () => null,
    resident_add: () => ({ thread: ID, home }),
    resident_action: ({ action }) => {
      residents[0].paused = action === 'pause';
      changed(ID);
      return true;
    },
    routine_action: ({ id, action }) => {
      const r = routines.find((x) => x.id === id);
      if (!r) throw new Error(`no routine ${id}`);
      if (action === 'decline') routines = routines.filter((x) => x.id !== id);
      else if (action === 'approve')
        Object.assign(r, {
          proposed: false,
          paused: false,
          expires_at: null,
          next_at: Math.floor(Date.now() / 1000) + 3600,
        });
      else if (action === 'pause') r.paused = true;
      else if (action === 'resume') r.paused = false;
      changed(ID);
      return true;
    },
    send: (args) => {
      const turn = send(args);
      if (args.thread === ID && /毎朝|every morning/i.test(args.text)) {
        setTimeout(() => {
          routines.push({
            ...base,
            id: `routine-${Date.now()}`,
            name: 'every morning',
            instruction: args.text,
            schedule: 'daily at 08:00',
            made_by: 'agent',
            paused: true,
            proposed: true,
            next_at: null,
            last_at: null,
            last_state: null,
          });
          sync(ID);
          changed(ID);
        }, 1400);
      }
      return turn;
    },
  });
}
