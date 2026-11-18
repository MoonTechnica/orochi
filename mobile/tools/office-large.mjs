// A big office for load tests: 120 people in 36 rooms over 5 projects, made from the shape of the
// desktop's recorded Office data (desktop/tests/office.json) and a fixed seed, so every run draws
// the same picture. Used by `mock-server.mjs --office-large` and by packages/core's tests.

const rng = (seed) => () => {
  seed = (seed + 0x6d2b79f5) >>> 0;
  let t = seed;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const AGENTS = [
  { agent: 'claude', provider: 'anthropic', models: ['opus', 'sonnet'] },
  { agent: 'codex', provider: 'openai', models: ['gpt-6.1-sol', 'gpt-6.1-luna'] },
  { agent: 'antigravity', provider: 'google', models: ['gemini'] },
];
const ROLES = [
  'designer',
  'reviewer',
  'tester',
  'implementer',
  'researcher',
  'writer',
  'planner',
  'debugger',
  'migrator',
  'linter',
  'profiler',
  'scribe',
];
const DOING = [
  'Reading src/scheduler.rs',
  'Running cargo test --test adaptive',
  'Editing desktop/src/app.js',
  'Searching for every caller of stepOffice',
  'Writing the migration',
  'Reviewing the diff',
  'Fixing a flaky test',
  'Profiling the first paint',
  'Drafting the release notes',
];
const TITLES = [
  'Redesign the home screen',
  'Fix the flaky mailbox test',
  'Tune the scorer weights',
  'Add sign-in with a magic link',
  'Split the activity store',
  'Cache the route stats',
  'Port the Office to the phone',
  'Retry the quota probe',
  'Rename the classifier seats',
  'Trim the sidebar query',
  'Explain the merge trial',
  'Move the sandbox image',
  'Handle a revoked token',
  'Draft the design doc',
  'Shrink the avatar sprites',
  'Stream the diff viewer',
  'Plan the 2.0 migration',
  'Audit the keychain use',
  'Make the pairing link shorter',
  'Document the places command',
  'Check Windows paths',
  'Batch the changes feed',
  'Index the runs table',
  'Review the OAuth flow',
  'Rework the empty state',
  'Cut the cold start',
  'Test the reclaim sweep',
  'Fold the Git tab',
  'Speed up the file search',
  'Write the Japanese strings',
  'Pin the Node version',
  'Clean up the worktrees',
  'Explain the EWMA',
  'Fix the paste of long paths',
  'Compare two models',
  'Tidy the settings',
];
const PROJECTS = [
  ['orochi', 12],
  ['desktop', 9],
  ['mobile', 7],
  ['docs', 5],
  ['infra', 3],
];
/** Explicit sizes for a few rooms (project index, room index): a crowd and a room just over the cap. */
const BIG = { '0:0': 16, '1:2': 14, '3:0': 8, '2:1': 7 };
const FORCE = {
  '0:0': 'working',
  '0:1': 'needs_you',
  '0:5': 'needs_you',
  '1:0': 'needs_you',
  '1:4': 'needs_you',
  '2:0': 'needs_you',
  '3:0': 'working',
  '3:1': 'needs_you',
  '4:0': 'working',
};
const STATUSES = ['working', 'working', 'working', 'background', 'queued', 'held', 'needs_you'];

export function largeOffice(now = Date.now(), seed = 7) {
  const next = rng(seed);
  const pick = (xs) => xs[Math.floor(next() * xs.length)];
  const sizes = new Map();
  let left = 120;
  for (const [k, n] of Object.entries(BIG)) {
    sizes.set(k, n);
    left -= n;
  }
  const total = PROJECTS.reduce((n, [, c]) => n + c, 0);
  const plain = [];
  PROJECTS.forEach(([, c], p) => {
    for (let i = 0; i < c; i++) if (!sizes.has(`${p}:${i}`)) plain.push(`${p}:${i}`);
  });
  left -= plain.length * 2;
  for (const k of plain) sizes.set(k, 2);
  while (left > 0) {
    const k = pick(plain);
    if (sizes.get(k) < 6) {
      sizes.set(k, sizes.get(k) + 1);
      left--;
    }
  }

  const seats = [];
  const tasks = [];
  const talkSeeds = [];
  let n = 0;
  let item = 1;
  const sidebar = PROJECTS.map(([name, count], p) => {
    const threads = [];
    for (let i = 0; i < count; i++) {
      const key = `${p}:${i}`;
      const id = `L${String(n + 1).padStart(2, '0')}`;
      const status = FORCE[key] ?? pick(STATUSES);
      const size = sizes.get(key);
      const lives = status === 'working' || status === 'needs_you' || status === 'background';
      const agents = [];
      for (let m = 0; m < size; m++) {
        const a = pick(AGENTS);
        const role =
          m === 0
            ? 'lead'
            : `${ROLES[(m - 1) % ROLES.length]}${m > ROLES.length ? `-${Math.ceil(m / ROLES.length)}` : ''}`;
        const live = lives && (m === 0 || next() < 0.55) ? 1 : 0;
        agents.push({
          role,
          agent: a.agent,
          provider: a.provider,
          model: pick(a.models),
          seats: 1,
          live,
          lead: m === 0,
          background: status === 'background' && m > 0,
          last_at: now - Math.floor(next() * 600000),
        });
        if (live) {
          const seat_id = `${id}-s${m}`;
          const started_at = now - 20000 - Math.floor(next() * 900000);
          seats.push({
            seat_id,
            thread_id: id,
            project_id: name,
            turn: 'x',
            ordinal: m,
            role,
            lead: m === 0,
            read_only: role === 'reviewer',
            title: 'Work',
            background: false,
            state: 'working',
            started_at,
            agent: a.agent,
            model: agents[m].model,
            doing: pick(DOING),
            provider: a.provider,
          });
          if (next() < 0.2)
            tasks.push({
              item_id: item++,
              thread_id: id,
              project_id: name,
              turn: 'x',
              seat_id,
              role,
              key: 'k',
              title: `Task: ${pick(DOING).toLowerCase()}`,
              started_at: now - 5000 - Math.floor(next() * 60000),
              calls: 1 + Math.floor(next() * 9),
              doing: null,
            });
        }
        if (m > 0 && next() < 0.12)
          talkSeeds.push({ id, role, size, age: 3000 + Math.floor(next() * 280000) });
      }
      const live = agents.filter((a) => a.live > 0).length;
      const lead = agents[0];
      threads.push({
        id,
        project: name,
        project_id: name,
        project_root: `/work/${name}`,
        title: TITLES[n % TITLES.length],
        cwd: `/work/${name}`,
        branch: 'main',
        origin: 'console',
        worktree: false,
        pinned: false,
        archived: false,
        updated_at: now - Math.floor(next() * 3600000),
        status,
        seats: live,
        asking: status === 'needs_you' ? 1 : 0,
        unread: 0,
        terminal: false,
        doing: null,
        background: status === 'background' ? 1 : 0,
        running_since: lives ? now - 60000 * (1 + (n % 30)) : null,
        agent: lead.agent,
        provider: lead.provider,
        agents,
        mentions: 0,
        overrides: { agent: null, model: null, reasoning: null, mode: null },
      });
      n++;
    }
    return {
      id: name,
      name,
      root: `/work/${name}`,
      pinned: false,
      collapsed: false,
      missing: false,
      threads,
    };
  });

  let seq = 1000;
  const talk = (at = Date.now()) =>
    talkSeeds
      .map((s, i) => ({
        seq: seq + i,
        project_id: 'p',
        via: 'mailbox',
        from: s.role,
        from_thread: s.id,
        from_role: s.role,
        to: 'lead',
        to_thread: s.id,
        to_role: 'lead',
        text: `${s.role}: a part of the work is ready for a look.`,
        mentions: [],
        at: at - s.age,
        read: null,
      }))
      .sort((a, b) => b.at - a.at);
  return { sidebar, seats, tasks, talk };
}
