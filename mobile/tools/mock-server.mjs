#!/usr/bin/env node
// A stand-in for the desktop's remote server (docs/mobile-remote-design.md), built from the
// recorded fixtures in desktop/tests so the app can be exercised without the desktop.
//   node tools/mock-server.mjs [--port 7421] [--host 127.0.0.1] [--code MOCK-1234] [--once]
import http from 'node:http';
import zlib from 'node:zlib';
import crypto from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { WebSocketServer } from 'ws';
import { largeOffice } from './office-large.mjs';
import { installResidents } from './mock-residents.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const load = (name) =>
  JSON.parse(readFileSync(resolve(here, '../../desktop/tests', `${name}.json`), 'utf8'));
const arg = (name, fallback) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
};
const HOST = arg('host', '127.0.0.1');
const PORT = Number(arg('port', '7421'));
const CODE = arg('code', 'MOCK-1234');
const MACHINE = 'Mock Mac';

// ---- data --------------------------------------------------------------------------------
const sidebar = load('sidebar');
const channel = load('channel');
const office = load('office');
const files = load('files');
const now = Date.now();

const projects = structuredClone(sidebar.projects ?? sidebar);
const root = projects[0].root;
const CH = channel.thread.thread.id;
const MEDIA = 'media-thread-0000-0000-000000000001';
const SHA_PNG = 'a1'.repeat(32);
const SHA_WAV = 'b2'.repeat(32);

const rowOf = (id) => projects.flatMap((p) => p.threads).find((th) => th.id === id);
const channelRow = {
  ...structuredClone(channel.thread.thread),
  status: 'needs_you',
  asking: 1,
  unread: 3,
  updated_at: now,
};
projects[0].threads = projects[0].threads.filter((th) => th.id !== CH);
projects[0].threads.unshift(channelRow);
projects[0].threads[1].status = 'working';
projects[0].threads[1].running_since = now - 120000;
projects[0].threads[1].doing = 'Reading src/scheduler.rs';
projects[0].threads[1].updated_at = now - 60000;
const mediaRow = {
  ...structuredClone(channelRow),
  id: MEDIA,
  title: 'Screenshots and recordings',
  status: 'unread',
  asking: 0,
  unread: 2,
  mentions: 0,
  updated_at: now - 300000,
  agents: channelRow.agents.slice(0, 1),
  pinned: true,
};
projects[0].threads.push(mediaRow);
projects.push({
  id: 'web-project',
  name: 'web',
  root: '/tmp/web',
  pinned: true,
  collapsed: false,
  missing: false,
  threads: [
    {
      ...structuredClone(mediaRow),
      id: 'web-1',
      title: 'Magic link login',
      project: 'web',
      project_id: 'web-project',
      status: 'background',
      pinned: false,
      unread: 0,
      updated_at: now - 900000,
      cwd: '/tmp/web',
      project_root: '/tmp/web',
    },
  ],
});

const mediaItems = [
  {
    seq: 1,
    turn: 'm1',
    turn_ordinal: 1,
    lane: null,
    role: null,
    agent: null,
    model: null,
    kind: 'user_message',
    status: null,
    text: 'Show me the login screen and the demo recording',
    data: null,
    truncated: false,
    patches: 0,
    at: now - 600000,
    failed: false,
    turn_origin: null,
    provider: null,
    member: null,
  },
  {
    seq: 2,
    turn: 'm1',
    turn_ordinal: 1,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'route',
    status: null,
    text: '',
    data: {
      agent: 'claude',
      provider: 'anthropic',
      model: 'opus',
      reasoning: 'medium',
      resumed: false,
    },
    truncated: false,
    patches: 0,
    at: now - 599000,
    failed: false,
    turn_origin: null,
    provider: 'anthropic',
    member: null,
  },
  {
    seq: 3,
    turn: 'm1',
    turn_ordinal: 1,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'tool_call',
    status: null,
    text: '',
    data: { title: 'Bash', detail: 'npm run screenshot' },
    truncated: false,
    patches: 0,
    at: now - 598000,
    failed: false,
    turn_origin: null,
    provider: 'anthropic',
    member: null,
  },
  {
    seq: 4,
    turn: 'm1',
    turn_ordinal: 1,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'agent_message',
    status: 'completed',
    text: 'Here is the screen:\n\n![Login screen](artifacts/login.png)\n\nThe recording: [demo](artifacts/demo.wav) and the change in [src/main.rs:3](src/main.rs:3).\n\n- tapped **Sign in**\n- saw `ok`\n\nRemote: ![remote](https://example.invalid/none.png)',
    data: null,
    truncated: false,
    patches: 0,
    at: now - 597000,
    failed: false,
    turn_origin: null,
    provider: 'anthropic',
    member: null,
  },
  {
    seq: 5,
    turn: 'm1',
    turn_ordinal: 1,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'media',
    status: null,
    text: '',
    data: {
      sha: SHA_PNG,
      kind: 'image',
      mime: 'image/png',
      bytes: 5120,
      name: 'stored-screenshot.png',
      kept: true,
    },
    truncated: false,
    patches: 0,
    at: now - 596000,
    failed: false,
    turn_origin: null,
    provider: 'anthropic',
    member: null,
  },
  {
    seq: 6,
    turn: 'm1',
    turn_ordinal: 1,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'media',
    status: null,
    text: '',
    data: {
      sha: SHA_WAV,
      kind: 'audio',
      mime: 'audio/wav',
      bytes: 88244,
      name: 'tone.wav',
      kept: true,
    },
    truncated: false,
    patches: 0,
    at: now - 595000,
    failed: false,
    turn_origin: null,
    provider: 'anthropic',
    member: null,
  },
  {
    seq: 7,
    turn: 'm1',
    turn_ordinal: 1,
    lane: 0,
    role: 'lead',
    agent: 'claude',
    model: 'opus',
    kind: 'checks',
    status: null,
    text: '',
    data: { outcome: 'success', checks: [{ name: 'npm test', passed: true }] },
    truncated: false,
    patches: 0,
    at: now - 594000,
    failed: false,
    turn_origin: null,
    provider: 'anthropic',
    member: null,
  },
];
const mediaThread = {
  thread: mediaRow,
  items: mediaItems,
  seats: [
    {
      seat_id: 'ms1',
      turn: 'm1',
      ordinal: 0,
      role: 'lead',
      lead: true,
      read_only: false,
      phase: null,
      state: 'done',
      agent: 'claude',
      model: 'opus',
      reasoning: 'medium',
      outcome: 'success',
      peer: null,
      peer_status: null,
      doing: null,
      title: null,
      background: false,
      reported: false,
      member: null,
    },
  ],
  files: [
    { turn: 'm1', path: 'src/main.rs', change: 'modified', added: 2, removed: 1, latest_patch: 42 },
  ],
};

const threads = new Map([
  [CH, { ...structuredClone(channel.thread), thread: channelRow }],
  [MEDIA, mediaThread],
]);
for (const p of projects)
  for (const th of p.threads) {
    if (!threads.has(th.id)) {
      threads.set(th.id, {
        thread: th,
        items: [
          {
            seq: 1,
            turn: 'x',
            turn_ordinal: 1,
            lane: null,
            role: null,
            agent: null,
            model: null,
            kind: 'user_message',
            status: null,
            text: th.title,
            data: null,
            truncated: false,
            patches: 0,
            at: th.updated_at - 1000,
            failed: false,
            turn_origin: null,
            provider: null,
            member: null,
          },
          {
            seq: 2,
            turn: 'x',
            turn_ordinal: 1,
            lane: 0,
            role: 'lead',
            agent: 'claude',
            model: 'opus',
            kind: 'agent_message',
            status: 'completed',
            text: 'Working on it.',
            data: null,
            truncated: false,
            patches: 0,
            at: th.updated_at,
            failed: false,
            turn_origin: null,
            provider: 'anthropic',
            member: null,
          },
        ],
        seats: [],
        files: [],
      });
    }
  }
const room = new Map([[CH, structuredClone(channel.room)]]);
const members = new Map([[CH, structuredClone(channel.members)]]);
let prompts = [
  {
    id: 'prompt-1',
    thread_id: CH,
    thread_title: channelRow.title,
    project: 'repo',
    kind: 'permission',
    agent: 'claude',
    model: 'opus',
    role: 'lead',
    title: 'Run python3 -m unittest?',
    detail: 'python3 -m unittest -v',
    options: [
      ['allow_once', 'Allow once'],
      ['allow_always', 'Always allow'],
    ],
    created_at: now - 30000,
    read_only: false,
    lead: true,
    workspace: null,
  },
];
let mentions = structuredClone(channel.mentions).map((m) => ({
  ...m,
  at: Math.floor(now / 1000) - 120,
}));
const tokens = new Set();
let paired = false;
const ONCE = process.argv.includes('--once');

// ---- media -------------------------------------------------------------------------------
function png(w, h, rgb) {
  const crc = (buf) => {
    let c,
      crc = ~0;
    for (const b of buf) {
      c = (crc ^ b) & 0xff;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crc = (crc >>> 8) ^ c;
    }
    return ~crc >>> 0;
  };
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const body = Buffer.concat([Buffer.from(type), data]);
    const sum = Buffer.alloc(4);
    sum.writeUInt32BE(crc(body));
    return Buffer.concat([len, body, sum]);
  };
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++) {
    raw[y * (w * 3 + 1)] = 0;
    for (let x = 0; x < w; x++) {
      const o = y * (w * 3 + 1) + 1 + x * 3;
      raw[o] = (rgb[0] + x) & 255;
      raw[o + 1] = (rgb[1] + y) & 255;
      raw[o + 2] = (rgb[2] + ((x + y) >> 1)) & 255;
    }
  }
  const head = Buffer.alloc(13);
  head.writeUInt32BE(w, 0);
  head.writeUInt32BE(h, 4);
  head[8] = 8;
  head[9] = 2;
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', head),
    chunk('IDAT', zlib.deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}
function wav(seconds = 2) {
  const rate = 22050,
    n = rate * seconds,
    data = Buffer.alloc(n * 2);
  for (let i = 0; i < n; i++)
    data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 440 * i) / rate) * 8000), i * 2);
  const h = Buffer.alloc(44);
  h.write('RIFF', 0);
  h.writeUInt32LE(36 + data.length, 4);
  h.write('WAVEfmt ', 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate * 2, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write('data', 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}
const PNG = png(480, 320, [40, 90, 160]);
const WAV = wav();
const TEXTS = {
  'src/main.rs': 'fn main() {\n    println!("hello");\n    changed();\n}\n',
  'README.md': '# Title\n\nbody\n',
};
const DIFF =
  '--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    println!("hello");\n+    changed();\n }\n';

const MIME = { png: 'image/png', wav: 'audio/wav', rs: 'text/plain', md: 'text/markdown' };
function sendBytes(req, res, buf, mime) {
  const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || '');
  const headers = { 'Content-Type': mime, 'Accept-Ranges': 'bytes' };
  if (range) {
    const start = range[1] ? Number(range[1]) : 0;
    const end = range[2] ? Math.min(Number(range[2]), buf.length - 1) : buf.length - 1;
    res.writeHead(206, {
      ...headers,
      'Content-Range': `bytes ${start}-${end}/${buf.length}`,
      'Content-Length': end - start + 1,
    });
    res.end(buf.subarray(start, end + 1));
  } else {
    res.writeHead(200, { ...headers, 'Content-Length': buf.length });
    res.end(buf);
  }
}

// ---- rpc ---------------------------------------------------------------------------------
const clients = new Set();
const broadcast = (event) =>
  clients.forEach((ws) => ws.readyState === 1 && ws.send(JSON.stringify(event)));
const changed = (...ids) => broadcast({ type: 'changed', threads: ids });
const sync = (id) => {
  const row = rowOf(id);
  const th = threads.get(id);
  if (row && th) {
    row.updated_at = Date.now();
    th.thread = row;
  }
};

// --office: the desktop's recorded Office data (tests/office.json) instead of the sample
// conversations, with its messages replayed on a loop so envelopes and speech can be watched.
const OFFICE = process.argv.includes('--office');
const QUIET = process.argv.includes('--quiet');
// --office-large: 120 people in 36 rooms over 5 projects (tools/office-large.mjs), for load tests.
const LARGE = process.argv.includes('--office-large');
const stopped = new Set();
const officeShift = Date.now() - 1794636100000;
const officeLoop = office.loop * 1000;
const officeBorn = Date.now();
const officeTalk = () => {
  const at = (Date.now() - officeBorn) % officeLoop;
  const lap = Math.floor((Date.now() - officeBorn) / officeLoop);
  const out = [];
  office.script.forEach((step, i) => {
    for (const l of [lap, lap - 1]) {
      const age = at - step.t * 1000 + (lap - l) * officeLoop;
      if (age < 0 || age > 16000) continue;
      out.push({
        seq: l * 100 + i + 1,
        project_id: 'p',
        via: 'mailbox',
        from: step.from[1],
        from_thread: step.from[0],
        from_role: step.from[1],
        to: step.to[1],
        to_thread: step.to[0],
        to_role: step.to[1],
        text: step.text,
        mentions: step.mentions ?? [],
        at: Date.now() - age,
        read: null,
      });
    }
  });
  return out.sort((a, b) => b.at - a.at);
};
const shifted = (row) => ({
  ...row,
  started_at: row.started_at + officeShift,
});

const rpc = {
  sidebar: () => projects,
  thread: ({ id }) => threads.get(id) ?? null,
  patch: () => DIFF,
  room: ({ thread }) => room.get(thread) ?? [],
  members: ({ thread }) => members.get(thread) ?? [],
  chatter: () =>
    (room.get(CH) ?? [])
      .filter((r) => r.kind === 'message')
      .map((r) => ({
        seq: r.seq,
        project_id: channelRow.project_id,
        via: r.via,
        from: r.who,
        from_thread: CH,
        from_role: r.role,
        to: r.whom ?? 'all',
        to_thread: CH,
        to_role: null,
        text: r.text,
        at: r.at,
        mentions: r.mentions,
        read: null,
      })),
  mentions: () => mentions,
  team: () => channel.team,
  board: () => [],
  open_prompts: () => prompts,
  live_seats: () =>
    office.live_seats.map((s) => ({ ...s, thread_id: s.thread_id === 't1' ? CH : s.thread_id })),
  live_tasks: () =>
    office.live_tasks.map((s) => ({ ...s, thread_id: s.thread_id === 't1' ? CH : s.thread_id })),
  search: ({ query }) =>
    [...threads.values()]
      .filter((t) =>
        (t.thread.title + t.items.map((i) => i.text).join(' '))
          .toLowerCase()
          .includes(String(query).toLowerCase()),
      )
      .map((t) => ({
        thread_id: t.thread.id,
        title: t.thread.title,
        project: t.thread.project,
        snippet: null,
      })),
  agents: () => [
    {
      agent: 'claude',
      model: '*',
      status: 'ready',
      cooling: 0,
      reset_at: null,
      quota_estimate: 0.6,
      failures: 0,
      windows: [['five_hour', 0.7, null, true]],
    },
    {
      agent: 'codex',
      model: '*',
      status: 'ready',
      cooling: 0,
      reset_at: null,
      quota_estimate: null,
      failures: 0,
      windows: [['codex/primary', 0.42, Math.floor(Date.now() / 1000) + 1200, true]],
      credits: [['codex/credits', '12.50']],
    },
  ],
  insights: () => [
    {
      agent: 'claude',
      model: 'opus',
      task_type: 'implementation',
      reasoning: 'medium',
      verified: 12,
      failures: 1,
      weak: 2,
      success_rate: 0.92,
      mean_tokens: 24000,
      mean_duration_ms: 94000,
      last_at: now,
    },
    {
      agent: 'codex',
      model: 'gpt-5.6',
      task_type: 'review',
      reasoning: 'medium',
      verified: 7,
      failures: 2,
      weak: 0,
      success_rate: 0.78,
      mean_tokens: 15000,
      mean_duration_ms: 61000,
      last_at: now,
    },
  ],
  folders: () => projects.map((p) => ({ name: p.name, root: p.root })),
  tree_read: ({ path }) => {
    if (path === 'blob.bin') return files.binary;
    const text = TEXTS[path] ?? files.text.text;
    return { ...files.text, path, text, lines: text.split('\n').length - 1, bytes: text.length };
  },
  tree_patch: () => DIFF,
  tree_files: () => [],
  tree_list: () => files.root,
  agent_icons: () => ({}),
  watch: () => ({ lost: false, queued: false }),
  send: ({ thread, text }) => {
    const th = threads.get(thread);
    if (!th) throw new Error('no such thread');
    const turn = `t-${Date.now()}`;
    const seq = th.items.length + 1;
    th.items.push({
      seq,
      turn,
      turn_ordinal: 99,
      lane: null,
      role: null,
      agent: null,
      model: null,
      kind: 'user_message',
      status: null,
      text,
      data: null,
      truncated: false,
      patches: 0,
      at: Date.now(),
      failed: false,
      turn_origin: null,
      provider: null,
      member: null,
    });
    th.thread.status = 'working';
    sync(thread);
    changed(thread);
    setTimeout(() => {
      th.items.push({
        seq: seq + 1,
        turn,
        turn_ordinal: 99,
        lane: 0,
        role: 'lead',
        agent: 'claude',
        model: 'opus',
        kind: 'agent_message',
        status: 'completed',
        text: `Echo from the mock: ${text}`,
        data: null,
        truncated: false,
        patches: 0,
        at: Date.now(),
        failed: false,
        turn_origin: null,
        provider: 'anthropic',
        member: null,
      });
      th.thread.status = 'unread';
      th.thread.unread += 1;
      sync(thread);
      changed(thread);
    }, 1200);
    return turn;
  },
  say: ({ thread, to, text, member }) => {
    const list = room.get(thread) ?? [];
    const mentioned = [...String(text).matchAll(/@([\w.-]+)/g)].map((m) => m[1].toLowerCase());
    list.push({
      seq: Math.max(0, ...list.map((l) => l.seq)) + 1,
      kind: 'message',
      who: 'you',
      whom: to ?? 'all',
      via: 'user',
      text,
      at: Date.now(),
      role: null,
      model: null,
      agent: null,
      provider: null,
      parent: null,
      member: member ?? null,
      mentions: mentioned,
      broadcast: false,
    });
    room.set(thread, list);
    changed(thread);
    return null;
  },
  answer: ({ prompt }) => {
    prompts = prompts.filter((p) => p.id !== prompt);
    broadcast({ type: 'prompts' });
    const row = rowOf(CH);
    if (row && !prompts.length) {
      row.status = 'working';
      row.asking = 0;
    }
    changed(CH);
    return true;
  },
  interrupt: ({ thread }) => {
    const row = rowOf(thread);
    if (row) row.status = 'interrupted';
    sync(thread);
    changed(thread);
    return null;
  },
  stop_seat: ({ seat }) => {
    stopped.add(seat);
    changed();
    return null;
  },
  seen: ({ thread }) => {
    const row = rowOf(thread);
    if (row) row.unread = 0;
    changed(thread);
    return null;
  },
  see_mentions: () => {
    mentions = [];
    return null;
  },
  new_thread: ({ root: r }) => {
    const id = crypto.randomUUID();
    const project = projects.find((p) => p.root === r) ?? projects[0];
    const row = {
      ...structuredClone(mediaRow),
      id,
      title: 'New conversation',
      status: 'idle',
      unread: 0,
      pinned: false,
      project: project.name,
      project_id: project.id,
      project_root: project.root,
      cwd: project.root,
      updated_at: Date.now(),
    };
    project.threads.unshift(row);
    threads.set(id, { thread: row, items: [], seats: [], files: [] });
    changed(id);
    return id;
  },
  ensure_host: () => false,
  rename_thread: ({ thread, title }) => {
    const r = rowOf(thread);
    if (r) r.title = title;
    changed(thread);
    return null;
  },
  pin_thread: ({ thread, pinned }) => {
    const r = rowOf(thread);
    if (r) r.pinned = pinned;
    changed(thread);
    return null;
  },
  archive_thread: ({ thread, archived }) => {
    const r = rowOf(thread);
    if (r) r.archived = archived;
    changed(thread);
    return null;
  },
  set_route: () => null,
  comment: () => 'ok',
};
installResidents({ projects, threads, changed, sync, rpc });
if (OFFICE) {
  Object.assign(rpc, {
    sidebar: () =>
      office.sidebar.map((p) => ({
        ...p,
        missing: false,
        threads: p.threads.map((th) => ({
          ...th,
          updated_at: th.updated_at + officeShift,
          running_since: th.running_since ? th.running_since + officeShift : null,
          overrides: th.overrides ?? { agent: null, model: null, reasoning: null, mode: null },
          agents: th.agents.map((a) => ({
            ...a,
            last_at: a.last_at + officeShift,
            live: office.live_seats.some(
              (x) => x.thread_id === th.id && x.role === a.role && stopped.has(x.seat_id),
            )
              ? 0
              : a.live,
          })),
        })),
      })),
    live_seats: () =>
      office.live_seats
        .filter((x) => !stopped.has(x.seat_id))
        .map((x) => {
          const list = office.doings[x.seat_id] ?? [x.doing];
          return shifted({
            ...x,
            doing: list[Math.floor((Date.now() - officeBorn) / 6000) % list.length],
          });
        }),
    live_tasks: () => office.live_tasks.map(shifted),
    chatter: () => officeTalk(),
    open_prompts: () => [],
    mentions: () => [],
  });
  if (QUIET) {
    const loud = rpc.sidebar;
    Object.assign(rpc, {
      sidebar: () =>
        loud().map((p) => ({
          ...p,
          threads: p.threads.map((th) => ({
            ...th,
            status: 'idle',
            asking: 0,
            agents: th.agents.map((a) => ({ ...a, live: 0 })),
          })),
        })),
      live_seats: () => [],
      live_tasks: () => [],
      chatter: () => [],
    });
  }
}

if (LARGE) {
  const big = largeOffice(Date.now());
  Object.assign(rpc, {
    sidebar: () =>
      big.sidebar.map((p) => ({
        ...p,
        threads: p.threads.map((th) => ({
          ...th,
          agents: th.agents.map((a) =>
            big.seats.some(
              (x) => x.thread_id === th.id && x.role === a.role && stopped.has(x.seat_id),
            )
              ? { ...a, live: 0 }
              : a,
          ),
        })),
      })),
    live_seats: () => big.seats.filter((x) => !stopped.has(x.seat_id)),
    live_tasks: () => big.tasks,
    chatter: () => big.talk(),
    open_prompts: () => [],
    mentions: () => [],
  });
}

// ---- http --------------------------------------------------------------------------------
const json = (res, status, body) => {
  const text = JSON.stringify(body);
  res.writeHead(status, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(text),
  });
  res.end(text);
};
const bearer = (req, url) => {
  const header = req.headers.authorization?.replace(/^Bearer\s+/i, '');
  return header || url.searchParams.get('token') || '';
};
const authed = (req, url) => tokens.has(bearer(req, url));
const body = (req) =>
  new Promise((ok) => {
    const parts = [];
    req.on('data', (c) => parts.push(c));
    req.on('end', () => ok(Buffer.concat(parts).toString()));
  });

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  // Only so the web build can be tried in a browser; a phone needs no CORS.
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Headers', 'authorization, content-type, range');
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    return res.end();
  }
  try {
    if (url.pathname === '/health')
      return json(res, 200, { ok: true, name: MACHINE, version: 'mock' });
    if (url.pathname === '/pair' && req.method === 'POST') {
      const { code, device } = JSON.parse((await body(req)) || '{}');
      if (code !== CODE || (ONCE && paired))
        return json(res, 401, { error: 'invalid or used pairing code' });
      paired = true;
      const token = crypto.randomBytes(32).toString('hex');
      tokens.add(token);
      console.log(`paired device: ${device}`);
      return json(res, 200, { token, name: MACHINE });
    }
    if (!authed(req, url)) return json(res, 401, { error: 'unauthorized' });
    if (url.pathname === '/me') return json(res, 200, { device: 'mock-device', paired_at: now });
    if (url.pathname.startsWith('/rpc/') && req.method === 'POST') {
      const command = decodeURIComponent(url.pathname.slice(5));
      const handler = rpc[command];
      if (!handler) return json(res, 404, { error: `unknown command ${command}` });
      const args = JSON.parse((await body(req)) || '{}');
      return json(res, 200, handler(args) ?? null);
    }
    if (url.pathname.startsWith('/media/')) {
      const [, , , sha] = url.pathname.split('/');
      if (sha === SHA_WAV) return sendBytes(req, res, WAV, 'audio/wav');
      return sendBytes(req, res, PNG, 'image/png');
    }
    if (url.pathname === '/file') {
      const path = url.searchParams.get('path') || '';
      if (path.includes('..')) return json(res, 400, { error: 'outside the working folder' });
      const ext = path.split('.').pop();
      if (ext === 'png') return sendBytes(req, res, PNG, 'image/png');
      if (ext === 'wav') return sendBytes(req, res, WAV, 'audio/wav');
      if (TEXTS[path])
        return sendBytes(req, res, Buffer.from(TEXTS[path]), MIME[ext] ?? 'text/plain');
      return json(res, 404, { error: `${path} not found` });
    }
    json(res, 404, { error: 'not found' });
  } catch (error) {
    json(res, 500, { error: String(error?.message ?? error) });
  }
});

const wss = new WebSocketServer({ noServer: true });
server.on('upgrade', (req, socket, head) => {
  const url = new URL(req.url, `http://${req.headers.host}`);
  if (url.pathname !== '/events' || !authed(req, url)) {
    socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n');
    socket.destroy();
    return;
  }
  wss.handleUpgrade(req, socket, head, (ws) => {
    clients.add(ws);
    ws.on('close', () => clients.delete(ws));
    ws.send(JSON.stringify({ type: 'hello' }));
  });
});
// A little life: the working thread's "doing" changes and the store announces it.
setInterval(() => {
  const r = projects[0].threads[1];
  if (r) {
    r.doing = `Reading file ${Math.floor(Math.random() * 90)}`;
    changed(r.id);
  }
}, 8000);

server.listen(PORT, HOST, () => {
  console.log(`mock orochi remote on http://${HOST}:${PORT}`);
  console.log(
    `pairing link: orochi://pair?host=${HOST}&port=${PORT}&code=${CODE}&name=${encodeURIComponent(MACHINE)}`,
  );
});
