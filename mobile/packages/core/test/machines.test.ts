import { describe, expect, it } from 'vitest';
import {
  active,
  emptyStore,
  findFor,
  fromStorage,
  markRepair,
  migrateLegacy,
  noticesFor,
  remove,
  rename,
  setActive,
  titleIn,
  toStorage,
  upsertPaired,
  withMachine,
  type MachineStore,
} from '../src';

let n = 0;
const key = () => `k${++n}`;
const home = {
  host: '100.64.0.1',
  port: 47821,
  token: 'T1',
  name: 'home',
  machineId: 'aaaaaaaaaaaaaaaa',
};
const work = {
  host: '100.64.0.2',
  port: 47821,
  token: 'T2',
  name: 'work',
  machineId: 'bbbbbbbbbbbbbbbb',
};

const two = (): MachineStore => {
  let s = upsertPaired(emptyStore(), home, 1, key).store;
  s = upsertPaired(s, work, 2, key).store;
  return s;
};

describe('machine store', () => {
  it('the first machine becomes active, a second one is added without switching', () => {
    const s = two();
    expect(s.machines).toHaveLength(2);
    expect(active(s)?.name).toBe('home');
  });
  it('pairing a known machine id updates host and token instead of duplicating', () => {
    const s = two();
    const r = upsertPaired(s, { ...home, host: '100.64.0.9', token: 'NEW' }, 9, key);
    expect(r.outcome).toBe('updated');
    expect(r.store.machines).toHaveLength(2);
    expect(r.machine).toMatchObject({ host: '100.64.0.9', token: 'NEW', needsRepair: false });
  });
  it('without an id, the same host:port is the same machine, and an id is learned', () => {
    const legacy = upsertPaired(emptyStore(), { ...home, machineId: undefined }, 1, key).store;
    const r = upsertPaired(legacy, home, 2, key);
    expect(r.outcome).toBe('updated');
    expect(r.store.machines).toHaveLength(1);
    expect(r.machine.machineId).toBe(home.machineId);
    const bare = upsertPaired(r.store, { ...home, machineId: undefined, token: 'X' }, 3, key);
    expect(bare.outcome).toBe('updated');
    expect(bare.machine.machineId).toBe(home.machineId);
  });
  it('another id at the same address is another machine', () => {
    const s = two();
    const r = upsertPaired(s, { ...home, machineId: 'cccccccccccccccc' }, 5, key);
    expect(r.outcome).toBe('added');
    expect(
      findFor(r.store, { host: home.host, port: home.port, machineId: home.machineId })?.token,
    ).toBe('T1');
  });
  it('switching sets the active machine and its last use; unknown keys change nothing', () => {
    const s = two();
    const other = s.machines[1].key;
    const next = setActive(s, other, 50);
    expect(active(next)?.name).toBe('work');
    expect(active(next)?.lastUsedAt).toBe(50);
    expect(setActive(s, 'nope', 60)).toBe(s);
  });
  it('removing the active machine falls back to the most recently used', () => {
    let s = two();
    s = setActive(setActive(s, s.machines[1].key, 10), s.machines[0].key, 20);
    expect(active(remove(s, s.machines[0].key))?.name).toBe('work');
    expect(remove(remove(s, s.machines[0].key), s.machines[1].key).activeKey).toBeNull();
  });
  it('a revoked token marks only that machine; pairing again clears it', () => {
    const base = two();
    const marked = markRepair(base, base.machines[1].key);
    expect(marked.machines.map((m) => !!m.needsRepair)).toEqual([false, true]);
    const again = upsertPaired(marked, { ...work, token: 'Z' }, 99, key);
    expect(again.machine.needsRepair).toBe(false);
  });
  it('a local label wins and same-named machines are told apart by address', () => {
    let s = two();
    s = rename(s, s.machines[0].key, '  Home Mac ');
    expect(titleIn(s, s.machines[0])).toBe('Home Mac');
    s = upsertPaired(
      s,
      { ...work, name: 'home', machineId: 'dddddddddddddddd', host: '100.64.0.3' },
      3,
      key,
    ).store;
    s = rename(s, s.machines[0].key, '');
    expect(titleIn(s, s.machines[0])).toBe('home (100.64.0.1:47821)');
    // both of them are told apart, consistently, wherever the name is shown
    expect(titleIn(s, s.machines[2])).toBe('home (100.64.0.3:47821)');
    expect(new Set(s.machines.map((m) => titleIn(s, m))).size).toBe(s.machines.length);
  });
});

describe('pair again', () => {
  it('replaces the chosen machine even when the desktop now answers with another id', () => {
    const s = two();
    const key0 = s.machines[0].key;
    const r = upsertPaired(
      markRepair(s, key0),
      { ...home, machineId: 'eeeeeeeeeeeeeeee', token: 'N' },
      9,
      key,
      key0,
    );
    expect(r.outcome).toBe('updated');
    expect(r.store.machines).toHaveLength(2);
    expect(r.machine).toMatchObject({
      key: key0,
      machineId: 'eeeeeeeeeeeeeeee',
      token: 'N',
      needsRepair: false,
    });
  });
});

describe('machine storage', () => {
  it('keeps tokens out of the metadata record and round-trips', () => {
    const s = two();
    const { meta, tokens } = toStorage(s);
    expect(meta).not.toContain('T1');
    expect(Object.values(tokens).sort()).toEqual(['T1', 'T2']);
    expect(fromStorage(meta, (slot) => tokens[slot] ?? null)).toEqual(s);
  });
  it('drops a machine whose token is gone and survives a damaged record', () => {
    const s = two();
    const { meta, tokens } = toStorage(s);
    const only = fromStorage(meta, (slot) =>
      slot.endsWith(s.machines[0].key) ? tokens[slot] : null,
    );
    expect(only.machines).toHaveLength(1);
    expect(only.activeKey).toBe(s.machines[0].key);
    expect(fromStorage('{nope', () => null)).toEqual(emptyStore());
  });
  it('migrates the single-machine record of earlier versions without a re-pair', () => {
    const m = migrateLegacy(
      JSON.stringify({ host: 'h', port: 1, token: 'old', name: 'Mac' }),
      5,
      key,
    );
    expect(m?.machines[0]).toMatchObject({ host: 'h', token: 'old', name: 'Mac' });
    expect(active(m!)?.token).toBe('old');
    expect(migrateLegacy('{"host":"h"}', 5, key)).toBeNull();
    expect(migrateLegacy(null, 5, key)).toBeNull();
  });
});

describe('scoping', () => {
  it('names the machine in a notice only when more than one is paired', () => {
    const [notice] = noticesFor(
      { prompts: [], mentions: [], status: {} },
      [],
      [{ id: 'p1', thread_id: 't', title: 'Run?', thread_title: 'Fix', project: 'x' } as any],
      [],
    );
    expect(withMachine(notice, 'work', 1).title).toBe('Fix');
    expect(withMachine(notice, 'work', 2).title).toBe('work · Fix');
  });
});
