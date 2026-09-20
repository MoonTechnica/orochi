#!/usr/bin/env python3
"""Compare Orochi with each agent used alone on the same tasks, with real ACP agents.

Every trial gets a fresh repository and session, and a fresh data directory unless its arm
asks for a shared one (`memory: shared`), which is how a user's own Orochi runs: what it learns
about routing and about what asking costs carries from one task to the next. A trial is scored only by the
case's hidden check, run after the arm finished; the visible tests in the repository are what
the agents (and Orochi's evaluator) can see. An arm is charged every token its runs recorded —
execution, retries, classification and routing advice — as reported by each agent, never an
estimate. A trial whose agent could not be discovered or never ran is `blocked`, never a pass.

--execute consumes provider quota. --validate contacts nothing: it checks that each case fails
as given and passes with its reference solution.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

PURPOSE_EXECUTION = 'execution'


def command(argv, timeout, cwd=None, env=None):
    p = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         text=True, start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out, err
    except subprocess.TimeoutExpired:
        return None, '', 'timed out'
    finally:
        # Includes any surviving descendants after a timeout or a normal parent exit.
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.wait()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def materialize(root, files):
    """Writes each file; `null` content deletes it (a reference that removes a file)."""
    for name, content in files.items():
        path = Path(name)
        assert not path.is_absolute() and '..' not in path.parts, name
        target = root / path
        if content is None:
            if target.exists():
                target.unlink()
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)


def hidden(case, root, timeout, env):
    """The case's hidden check, run in the repository after the arm finished."""
    with tempfile.TemporaryDirectory(prefix='orochi-check-') as temporary:
        check = Path(temporary) / 'verify.py'
        check.write_text(case['check'])
        code, _, err = command([sys.executable, str(check)], timeout, cwd=root, env=env)
        return code == 0, err[-2000:]


def visible(case, root, timeout, env):
    spec = case['visible']
    code, _, err = command([spec['command'], *spec.get('args', [])], timeout, cwd=root, env=env)
    return code == 0, err[-2000:]


def environment(prepend):
    env = dict(os.environ)
    if prepend:
        env['PATH'] = os.pathsep.join(prepend + [env.get('PATH', '')])
    return env


def validate(args):
    suite = json.loads(args.suite.read_text())
    args.output.mkdir(parents=True, exist_ok=False)
    env = environment(args.agent_path_prepend)
    report = dict(cases=[])
    with tempfile.TemporaryDirectory(prefix='orochi-validate-') as temporary:
        for case in suite['cases']:
            root = Path(temporary) / case['id']
            root.mkdir()
            materialize(root, case['files'])
            failing, _ = hidden(case, root, args.check_timeout, env)
            materialize(root, case['reference'])
            passing, error = hidden(case, root, args.check_timeout, env)
            shown, shown_error = visible(case, root, args.check_timeout, env)
            entry = dict(id=case['id'], starts_failing=not failing, reference_passes=passing,
                         visible_passes_with_reference=shown)
            if not passing:
                entry['error'] = error
            if not shown:
                entry['visible_error'] = shown_error
            report['cases'].append(entry)
            print(json.dumps(entry), flush=True)
    save(args.output / 'validation.json', report)
    ok = all(c['starts_failing'] and c['reference_passes'] and c['visible_passes_with_reference']
             for c in report['cases'])
    return 0 if ok else 1


def toml(value):
    return json.dumps(value)


def config(arm, case, args, env_prepend):
    spec = case['visible']
    lines = [
        '[discovery]', 'auto_add = false',
        '[scheduler]', f"max_attempts = {int(arm.get('attempts', 1))}",
        'discovery_timeout_secs = 120', f'prompt_timeout_secs = {args.timeout}',
        'permission = "allow"',
        '[classifier]', f"enabled = {'true' if arm.get('classifier') else 'false'}",
        # A string names the agent that classifies; true leaves the choice to Orochi.
        *([f"agent = {toml(arm['classifier'])}"] if isinstance(arm.get('classifier'), str) else []),
        '[mailbox]', 'enabled = false',
        '[evaluator]', 'auto = false', f'timeout_secs = {args.check_timeout}',
        '[[evaluator.checks]]', 'name = "visible-tests"',
        f"command = {toml(spec['command'])}", f"args = {toml(spec.get('args', []))}",
    ]
    for agent in arm['agents']:
        lines += ['[[agents]]', f"id = {toml(agent['id'])}", f"provider = {toml(agent['provider'])}",
                  f"command = {toml(agent['command'])}", f"args = {toml(agent.get('args', []))}"]
        extra = dict(agent.get('env', {}))
        if env_prepend:
            extra['PATH'] = os.pathsep.join(env_prepend + [os.environ.get('PATH', '')])
        if extra:
            lines.append('[agents.env]')
            lines += [f'{key} = {toml(value)}' for key, value in extra.items()]
    return '\n'.join(lines) + '\n'


def tokens(runs):
    """Every recorded token, by purpose. Usage the agent did not report stays unknown."""
    by_purpose, parts = {}, dict(input=0, output=0, reasoning=0, cached=0)
    unknown = 0
    for run in runs:
        usage = run.get('usage') or {}
        total = usage.get('total_tokens')
        if total is None and usage.get('input_tokens') is not None and usage.get('output_tokens') is not None:
            total = usage['input_tokens'] + usage['output_tokens']
        if total is None:
            unknown += 1
            continue
        by_purpose[run['purpose']] = by_purpose.get(run['purpose'], 0) + total
        for key, field in (('input', 'input_tokens'), ('output', 'output_tokens'),
                           ('reasoning', 'reasoning_tokens'), ('cached', 'cached_tokens')):
            parts[key] += usage.get(field) or 0
    return dict(total=sum(by_purpose.values()), by_purpose=by_purpose, unreported_runs=unknown, **parts)


def trial(case, arm, base, args, binary, data):
    root = base / 'repo'
    root.mkdir(parents=True)
    materialize(root, case['files'])
    git = lambda *argv: command(['git', *argv], 30, cwd=root)
    git('init', '-q')
    git('-c', 'user.name=bench', '-c', 'user.email=bench@example.invalid', 'add', '-A')
    git('-c', 'user.name=bench', '-c', 'user.email=bench@example.invalid', 'commit', '-qm', 'start')
    if not data.exists():
        data.mkdir(parents=True)
        if args.adapter_cache:
            (data / 'adapters').symlink_to(args.adapter_cache.resolve(), target_is_directory=True)
    path = base / 'config.toml'
    path.write_text(config(arm, case, args, args.agent_path_prepend))
    env = environment(args.agent_path_prepend)
    cli = [binary, '--config', str(path), '--data-dir', str(data), '-C', str(root)]
    pin = arm.get('pin') or {}
    override = []
    for flag in ('agent', 'model', 'reasoning'):
        if pin.get(flag):
            override += [f'--{flag}', pin[flag]]
    record = dict(case=case['id'], arm=arm['id'], status='blocked')
    started = time.time()
    code, _, err = command(cli + override + [case['task']], args.timeout * max(1, int(arm.get('attempts', 1))) + 300,
                           env=env)
    record['wall_s'] = round(time.time() - started, 1)
    record['exit_code'] = code
    record['stderr_tail'] = err[-3000:]
    _, out, _ = command(cli + ['runs', '--limit', '500'], 30, env=env)
    try:
        runs = json.loads(out)
    except json.JSONDecodeError:
        runs = []
    executions = [r for r in runs if r['purpose'] == PURPOSE_EXECUTION]
    record['attempts'] = len(executions)
    record['routes'] = [dict(agent=r['candidate']['agent'], model=r['candidate']['model'],
                             reasoning=r['candidate'].get('reasoning_level'), outcome=r['outcome'],
                             error_kind=r.get('error_kind'))
                        for r in sorted(executions, key=lambda r: r['started_at'])]
    record['advice'] = [dict(purpose=r['purpose'], agent=r['candidate']['agent'],
                             model=r['candidate']['model'],
                             tokens=(r.get('usage') or {}).get('total_tokens'))
                        for r in sorted(runs, key=lambda r: r['started_at'])
                        if r['purpose'] != PURPOSE_EXECUTION]
    record['tokens'] = tokens(runs)
    if not executions:
        record['reason'] = 'no execution was recorded (discovery, eligibility or quota)'
        return record
    record['hidden_pass'], record['hidden_error'] = hidden(case, root, args.check_timeout, env)
    record['status'] = 'measured'
    return record


def summarize(suite, trials):
    arms = {}
    for arm in suite['arms']:
        mine = [t for t in trials if t['arm'] == arm['id']]
        measured = [t for t in mine if t['status'] == 'measured']
        arms[arm['id']] = dict(
            measured=len(measured), blocked=len(mine) - len(measured),
            passed=sum(1 for t in measured if t.get('hidden_pass')),
            tokens=sum(t['tokens']['total'] for t in measured),
            unreported_runs=sum(t['tokens']['unreported_runs'] for t in measured),
            wall_s=round(sum(t['wall_s'] for t in measured), 1))
    return arms


def markdown(suite, trials, arms):
    lines = [f"# {suite['name']}", '', '| Arm | Passed | Measured | Blocked | Tokens | Wall (s) |',
             '|---|---|---|---|---|---|']
    for arm, s in arms.items():
        lines.append(f"| {arm} | {s['passed']} | {s['measured']} | {s['blocked']} | {s['tokens']:,} | {s['wall_s']} |")
    lines += ['', '| Case | ' + ' | '.join(arms) + ' |', '|---|' + '---|' * len(arms)]
    for case in suite['cases']:
        cells = []
        for arm in arms:
            t = next((t for t in trials if t['case'] == case['id'] and t['arm'] == arm), None)
            if t is None:
                cells.append('—')
            elif t['status'] != 'measured':
                cells.append('blocked')
            else:
                route = ', '.join(f"{r['model']}" for r in t['routes'])
                cells.append(f"{'✓' if t['hidden_pass'] else '✗'} {t['tokens']['total']:,} ({route})")
        lines.append(f"| {case['id']} | " + ' | '.join(cells) + ' |')
    return '\n'.join(lines) + '\n'


def execute(args):
    suite = json.loads(args.suite.read_text())
    assert suite['schema_version'] == 2
    arms = [a for a in suite['arms'] if not args.arms or a['id'] in args.arms]
    cases = [c for c in suite['cases'] if not args.cases or c['id'] in args.cases]
    assert arms and cases, 'nothing selected'
    args.output.mkdir(parents=True, exist_ok=False)
    binary = str(args.binary.resolve())
    results = dict(schema_version=1, suite=suite['name'], started_at=int(time.time()), trials=[])
    with tempfile.TemporaryDirectory(prefix='orochi-compare-') as temporary:
        base = Path(temporary)
        for index, case in enumerate(cases):
            # Rotate the order so no arm always runs first (provider load, time of day).
            shift = index % len(arms)
            for number, arm in enumerate(arms[shift:] + arms[:shift]):
                print(f"{case['id']} / {arm['id']}", flush=True)
                # An arm with `memory: shared` keeps one data directory across the suite, as a
                # user's Orochi does: what it learns about cost and routing carries over.
                data = (base / f"shared-{arm['id']}" if arm.get('memory') == 'shared'
                        else base / f'{index}-{number}') / 'data'
                record = trial(case, arm, base / f'{index}-{number}', args, binary, data)
                results['trials'].append(record)
                results['arms'] = summarize(dict(suite, arms=arms), results['trials'])
                save(args.output / 'results.json', results)
                print(json.dumps({k: record.get(k) for k in ('status', 'hidden_pass', 'attempts', 'wall_s')}
                                 | dict(tokens=record['tokens']['total'])), flush=True)
    selected = dict(suite, arms=arms, cases=cases)
    (args.output / 'summary.md').write_text(markdown(selected, results['trials'], results['arms']))
    print(json.dumps(results['arms']))
    return 0


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--suite', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/release/orochi')
    p.add_argument('--arms', type=lambda v: v.split(','), default=None)
    p.add_argument('--cases', type=lambda v: v.split(','), default=None)
    p.add_argument('--adapter-cache', type=Path)
    p.add_argument('--timeout', type=int, default=900, help='seconds per agent prompt')
    p.add_argument('--check-timeout', type=int, default=300)
    # Absolute directories placed before PATH for Orochi, the agents and the checks.
    p.add_argument('--agent-path-prepend', action='append', default=[], type=lambda v: str(Path(v).expanduser().resolve()))
    mode = p.add_mutually_exclusive_group(required=True)
    mode.add_argument('--execute', action='store_true', help='run the arms; consumes provider quota')
    mode.add_argument('--validate', action='store_true', help='check the suite; contacts no agent')
    parsed = p.parse_args()
    raise SystemExit(validate(parsed) if parsed.validate else execute(parsed))
