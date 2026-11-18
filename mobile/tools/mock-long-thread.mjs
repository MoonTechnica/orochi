#!/usr/bin/env node
// The mock desktop with a long conversation: the recorded team channel followed by TURNS more turns
// (default 100: your message, the lead's reply and a member's post each, about 300 posts), to see
// how a long thread scrolls. It edits nothing: it reads tools/mock-server.mjs as it is, splices the
// extra rows in right after the channel fixture is loaded, and runs that copy (tools/.long-thread.gen.mjs,
// deleted on exit). Every other argument goes to the mock server (--port, --code, ...).
//   node tools/mock-long-thread.mjs [--turns 100] [--port 7421]
import { spawn } from 'node:child_process';
import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const i = args.indexOf('--turns');
const turns = i >= 0 ? Number(args.splice(i, 2)[1]) || 100 : 100;

const anchor = "const channel = load('channel');";
const source = readFileSync(resolve(here, 'mock-server.mjs'), 'utf8');
if (!source.includes(anchor)) {
  console.error(`mock-server.mjs no longer has: ${anchor}`);
  process.exit(1);
}
const splice = `
{
  const TURNS = ${turns};
  const base = 1790000400000;
  const said = channel.room;
  const items = channel.thread.items;
  let seq = 1000;
  const lines = ['Use the existing helper.', 'Tests are green now.', 'Looks fine to me.', 'Which branch?', 'Done, see the diff.'];
  for (let n = 0; n < TURNS; n++) {
    const at = base + n * 60000;
    const turn = 'long' + n;
    items.push({ seq: ++seq, turn, turn_ordinal: 100 + n, lane: null, role: null, agent: null, model: null,
      kind: 'user_message', status: null, text: 'Question ' + (n + 1) + ': ' + lines[n % lines.length], data: null,
      truncated: false, patches: 0, at, failed: false, turn_origin: null, provider: null, member: null });
    items.push({ seq: ++seq, turn, turn_ordinal: 100 + n, lane: 0, role: 'lead', agent: 'claude', model: 'opus',
      kind: 'agent_message', status: 'completed', text: 'Answer ' + (n + 1) + ': ' + lines[(n + 2) % lines.length],
      data: null, truncated: false, patches: 0, at: at + 20000, failed: false, turn_origin: null, provider: 'anthropic', member: null });
    said.push({ seq: ++seq, kind: 'message', who: n % 2 ? 'reviewer' : 'implementer', whom: 'all', via: 'mailbox',
      text: 'Note ' + (n + 1) + ': ' + lines[(n + 4) % lines.length], at: at + 40000,
      role: n % 2 ? 'reviewer' : 'implementer', model: n % 2 ? 'gpt-5.6' : 'qwen3.8-27b',
      agent: n % 2 ? 'codex' : 'opencode', provider: n % 2 ? 'openai' : 'opencode', parent: null,
      member: n % 2 ? 'reviewer' : 'implementer', mentions: [], broadcast: true });
  }
}
`;
const gen = resolve(here, '.long-thread.gen.mjs');
writeFileSync(gen, source.replace(anchor, anchor + splice));
const child = spawn(process.execPath, [gen, ...args], { stdio: 'inherit' });
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
