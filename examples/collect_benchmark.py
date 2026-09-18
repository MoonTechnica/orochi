#!/usr/bin/env python3
"""Collect real paired ACP trials from a JSON suite. Every arm gets fresh files, DB and session.
Explicit --execute consumes provider quota. Interrupted/incomplete cases never enter dataset.json.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time


def command(argv, timeout, cwd=None):
    p = subprocess.Popen(argv, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         text=True, start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out, err
    finally:
        # Includes any surviving descendants after timeout or normal parent exit.
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.wait()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def collect(args):
    suite = json.loads(args.suite.read_text())
    assert suite['schema_version'] == 1 and len(suite['arms']) >= 2
    if args.adapter_cache:
        assert args.adapter_cache.is_dir(), 'adapter cache must exist'
    assert len({a['id'] for a in suite['arms']}) == len(suite['arms'])
    assert len({c['id'] for c in suite['cases']}) == len(suite['cases'])
    args.output.mkdir(parents=True, exist_ok=False)
    dataset = dict(schema_version=1, provenance='Real ACP paired trials: ' + suite['name'],
                   synthetic=False, cases=[])
    evidence = dict(started_at=int(time.time()), trials=[], blocked_cases=[])
    q = json.dumps
    binary = str(args.binary.resolve())
    with tempfile.TemporaryDirectory(prefix='orochi-benchmark-') as temporary:
        base = Path(temporary)
        for index, case in enumerate(suite['cases']):
            measured = []
            # Rotate execution order to reduce provider/time-order bias.
            arms = suite['arms'][index % len(suite['arms']):] + suite['arms'][:index % len(suite['arms'])]
            for number, arm in enumerate(arms):
                trial = base / f'{index}-{number}'
                root = trial / 'repo'
                root.mkdir(parents=True)
                for name, content in case['files'].items():
                    path = Path(name)
                    assert not path.is_absolute() and '..' not in path.parts
                    target = root / path
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(content)
                # Evaluator is outside the agent's working directory and is run by Orochi.
                check = trial / 'verify.py'
                check.write_text(case['check'])
                path_env = ('[agents.env]\nPATH = ' + q(os.pathsep.join(args.agent_path_prepend + [os.environ.get('PATH', '')])) + '\n'
                            if args.agent_path_prepend else '')
                config = trial / 'config.toml'
                config.write_text(f'''[discovery]
auto_add = false
[scheduler]
max_attempts = 1
discovery_timeout_secs = 60
prompt_timeout_secs = {args.timeout}
permission = "allow"
[learning]
strategy = "static"
[evaluator]
auto = false
timeout_secs = {args.check_timeout}
[[evaluator.checks]]
name = "paired-task-check"
command = {q(os.sys.executable)}
args = [{q(str(check))}]
[[agents]]
id = {q(arm['agent'])}
provider = {q(arm['provider'])}
command = {q(arm['command'])}
args = {q(arm.get('args', []))}
''' + path_env)
                # Adapter cache may be shared; each trial's telemetry is independent.
                data = trial / 'data'
                data.mkdir()
                if args.adapter_cache:
                    (data / 'adapters').symlink_to(args.adapter_cache.resolve(), target_is_directory=True)
                cli = [binary, '--config', str(config), '--data-dir', str(data), '-C', str(root)]
                override = ['--agent', arm['agent'], '--model', arm['model']]
                if arm.get('reasoning'):
                    override += ['--reasoning', arm['reasoning']]
                record = dict(case=case['id'], arm=arm['id'], status='blocked')
                print(f"{case['id']} / {arm['id']}", flush=True)
                try:
                    code, out, err = command(cli + override + ['--dry-run', '--json', case['task']], 150)
                    if code:
                        raise ValueError('Discovery/eligibility failed: ' + err[-1200:])
                    route = json.loads(out)
                    baseline = route['candidates'][0]
                    code, out, err = command(cli + override + [case['task']], args.timeout + 150)
                    _, out, _ = command(cli + ['runs', '--limit', '10'], 15)
                    runs = [r for r in json.loads(out) if r['purpose'] == 'execution']
                    record.update(exit_code=code, runs=runs, execution_stderr=err[-4000:])
                    record['artifacts'] = {name: (root / name).read_text()[:20000] for name in case['files'] if (root / name).is_file()}
                    if len(runs) != 1:
                        raise ValueError('Expected exactly one execution')
                    run = runs[0]
                    tokens = run['usage'].get('total_tokens')
                    if tokens is None:
                        # Same conservative semantics as Usage::total; never substitute estimates.
                        u = run['usage']
                        if u.get('input_tokens') is not None and u.get('output_tokens') is not None:
                            tokens = u['input_tokens'] + u['output_tokens']
                    if tokens is None or run['outcome'] not in ('success', 'failure') or run.get('error_kind'):
                        raise ValueError('Missing comparable usage/evaluation, or execution error')
                    if any(run['candidate'][k] != baseline[k] for k in ('agent','model','reasoning_level','mode')):
                        raise ValueError('Execution configuration changed from frozen baseline')
                    measured.append(dict(candidate=baseline, success=run['outcome'] == 'success',
                                         tokens=tokens, duration_ms=run['duration_ms']))
                    record['status'] = 'measured'
                except (ValueError, KeyError, subprocess.TimeoutExpired, OSError) as error:
                    record['reason'] = str(error)
                evidence['trials'].append(record)
                save(args.output / 'evidence.json', evidence)
            if len(measured) == len(suite['arms']):
                dataset['cases'].append(dict(id=case['id'], descriptor=route['descriptor'], arms=measured))
            else:
                evidence['blocked_cases'].append(case['id'])
            save(args.output / 'dataset.json', dataset)
            save(args.output / 'evidence.json', evidence)
    print(json.dumps(dict(complete_cases=len(dataset['cases']), blocked_cases=evidence['blocked_cases'])))
    return int(bool(evidence['blocked_cases']))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--suite', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/debug/orochi')
    p.add_argument('--adapter-cache', type=Path)
    p.add_argument('--timeout', type=int, default=180)
    p.add_argument('--check-timeout', type=int, default=30)
    # Absolute directories placed before PATH for the agent and its tools (e.g. a rustup cargo).
    p.add_argument('--agent-path-prepend', action='append', default=[], type=lambda v: str(Path(v).resolve()))
    p.add_argument('--execute', action='store_true', required=True)
    raise SystemExit(collect(p.parse_args()))
