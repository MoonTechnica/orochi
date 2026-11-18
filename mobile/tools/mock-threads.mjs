#!/usr/bin/env node
// The mock desktop with a team channel that has something to show in threads: the recorded
// channel moved to "now" (with an earlier turn from yesterday, for a day divider), a reviewer thread with
// four replies from three people, a reply also sent to the channel, and a third member (`docs`)
// that has mentioned you. It edits nothing: it reads tools/mock-server.mjs as it is, splices the
// rows in right after the channel fixture is loaded and runs that copy (tools/.threads.gen.mjs,
// deleted on exit). Every other argument goes to the mock server (--port, --code, ...).
//   node tools/mock-threads.mjs [--port 7421] [--code MOCK-1234] [--drip SECONDS]
import { spawn } from 'node:child_process';
import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const anchor = "const channel = load('channel');";
const source = readFileSync(resolve(here, 'mock-server.mjs'), 'utf8');
if (!source.includes(anchor)) {
  console.error(`mock-server.mjs no longer has: ${anchor}`);
  process.exit(1);
}
const splice = `
{
  const DAY = 86400000;
  const last = Math.max(...channel.room.map((r) => r.at));
  const shift = Date.now() - 90000 - last;
  const moved = (at) => at + shift;
  for (const r of channel.room) r.at = moved(r.at);
  for (const i of channel.thread.items) i.at = moved(i.at);
  for (const m of channel.members) {
    m.started_at = moved(m.started_at);
    if (m.ended_at) m.ended_at = moved(m.ended_at);
    if (m.last_reply_at) m.last_reply_at = Math.round(moved(m.last_reply_at * 1000) / 1000);
  }
  const min = (n) => Date.now() - n * 60000;
  // An earlier turn, yesterday, so the day divider has something to separate.
  const first = channel.thread.items[0];
  const y = Date.now() - DAY - 600000;
  channel.thread.items.unshift(
    { ...structuredClone(first), seq: 900, turn: 'y0', turn_ordinal: 0, kind: 'user_message', text: 'Where do the greet tests live?', at: y, role: null, agent: null, model: null, provider: null },
    { ...structuredClone(first), seq: 901, turn: 'y0', turn_ordinal: 0, kind: 'agent_message', role: 'lead', agent: 'claude', provider: 'anthropic', model: 'opus', text: 'There are none yet: tests/ has only test_wave.py.', at: y + 20000 },
  );
  let seq = 200;
  const row = (o) => ({ seq: ++seq, kind: 'message', whom: null, via: 'mailbox', role: null, model: null,
    agent: null, provider: null, parent: null, member: null, mentions: [], broadcast: false, ...o });
  const rev = { who: 'reviewer', role: 'reviewer', agent: 'codex', provider: 'openai', model: 'gpt-5.6', member: 'reviewer' };
  const lead = { who: 'lead', role: 'lead', agent: 'claude', provider: 'anthropic', model: 'opus' };
  const reviewerParent = channel.room.find((r) => r.via === 'handoff' && r.member === 'reviewer').seq;
  channel.room.push(
    row({ ...rev, whom: 'lead', text: 'The float case now asserts the value, not just the type.', at: min(9), parent: reviewerParent }),
    row({ ...lead, whom: 'reviewer', text: '@reviewer good, is bool covered too?', at: min(8), parent: reviewerParent, member: 'reviewer', mentions: ['reviewer'] }),
    row({ ...rev, whom: 'lead', text: 'bool is an int subclass, so it needs its own assert. I will add it.', at: min(6), parent: reviewerParent }),
    row({ who: 'you', via: 'user', text: 'Please keep the change to the tests only.', at: min(5), parent: reviewerParent, member: 'reviewer' }),
    row({ ...rev, whom: 'all', text: 'Reviewed: both asserts are strong now, 7/7 pass.', at: min(4), parent: reviewerParent, broadcast: true }),
    row({ ...lead, via: 'handoff', whom: null, text: '@docs readme: describe the greet() tests in README.md', at: min(6), member: 'docs' }),
  );
  const docsParent = seq;
  channel.room.push(
    row({ who: 'docs', role: 'docs', agent: 'claude', provider: 'anthropic', model: 'sonnet', member: 'docs', whom: 'lead', text: 'README has no testing section yet. Add one under "Development"?', at: min(5), parent: docsParent }),
    row({ who: 'docs', role: 'docs', agent: 'claude', provider: 'anthropic', model: 'sonnet', member: 'docs', whom: 'you', text: '@you which heading do you want for it: "Testing" or "Tests"?', at: min(2), parent: docsParent, mentions: ['you'] }),
  );
  const impl = channel.members.find((m) => m.name === 'implementer');
  channel.members.push({ ...structuredClone(impl), name: 'docs', role: 'docs', seat_id: 's7', agent: 'claude', provider: 'anthropic',
    model: 'sonnet', state: 'asking', live: false, reported: false, title: 'readme', replies: 2, last_reply_at: Math.round(min(2) / 1000),
    needs: 'asking', workspace_state: null, trial_state: null, doing: null });
  const reviewer = channel.members.find((m) => m.name === 'reviewer');
  reviewer.replies = 5;
  reviewer.last_reply_at = Math.round(min(4) / 1000);
  // --drip N: the reviewer posts into its thread every N seconds (to see "New replies" while scrolled up).
  const drip = process.argv.indexOf('--drip');
  if (drip > 0) {
    let n = 0;
    setInterval(() => {
      room.get(CH).push(row({ ...rev, whom: 'lead', text: 'Another note from the reviewer (' + ++n + ').', at: Date.now(), parent: reviewerParent }));
      changed(CH);
    }, Number(process.argv[drip + 1] || 30) * 1000);
  }
}
`;
const gen = resolve(here, '.threads.gen.mjs');
writeFileSync(gen, source.replace(anchor, anchor + splice));
const child = spawn(process.execPath, [gen, ...process.argv.slice(2)], { stdio: 'inherit' });
const stop = () => {
  try {
    rmSync(gen, { force: true });
  } catch {}
};
child.on('exit', (code) => {
  stop();
  process.exit(code ?? 0);
});
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, () => child.kill(sig));
