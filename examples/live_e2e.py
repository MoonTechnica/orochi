#!/usr/bin/env python3
"""Probe a real Gemini/Antigravity ACP CLI. --execute sends one tiny, evaluated task.
No login automation. Missing CLI or failed discovery is reported as blocked, never passed.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--agent', choices=['gemini', 'antigravity'], required=True)
p.add_argument('--command', help='Absolute path to an installed, authenticated ACP CLI')
p.add_argument('--arg', action='append', default=None, help='Repeatable ACP argument (use --arg=--acp)')
p.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/debug/orochi')
p.add_argument('--output', type=Path, required=True)
p.add_argument('--execute', action='store_true', help='Send one real task; consumes the agent account quota')
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=True)
report = {'agent': a.agent, 'executed': False, 'status': 'blocked', 'time': int(time.time())}

def save():
    (a.output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))

name = a.command or ('gemini' if a.agent == 'gemini' else 'agy_acp_server.par')
command = shutil.which(name)
if not command:
    report['reason'] = 'CLI not installed or not on PATH; supply --command after official installation/login'
    save()
    sys.exit(2)
args = a.arg if a.arg is not None else (['--acp'] if a.agent == 'gemini' else [])
report['command'] = command
report['args'] = args
try:
    version = subprocess.run([command, '--version'], input='', capture_output=True, text=True, timeout=5)
    report['version'] = version.stdout.strip()[:200] if version.returncode == 0 else None
except (subprocess.TimeoutExpired, OSError):
    report['version'] = None
with tempfile.TemporaryDirectory(prefix='orochi-live-e2e-') as temporary:
    root = Path(temporary)
    repo = root / 'repo'
    repo.mkdir()
    config = root / 'config.toml'
    # JSON quoted strings are compatible with TOML basic strings for these values.
    q = json.dumps
    check_code = "from pathlib import Path; assert Path('result.txt').read_text() == 'OROCHI_E2E_OK\\n'"
    config.write_text(f'''[discovery]
auto_add = false
auto_install = false
[scheduler]
max_attempts = 1
discovery_timeout_secs = 45
prompt_timeout_secs = 180
permission = "allow"
[learning]
strategy = "static"
[evaluator]
auto = false
timeout_secs = 10
[[evaluator.checks]]
name = "live-result"
command = {q(sys.executable)}
args = ["-c", {q(check_code)}]
[[agents]]
id = {q(a.agent)}
provider = "google"
command = {q(command)}
args = {q(args)}
''')
    base = [str(a.binary.resolve()), '--config', str(config), '--data-dir', str(root / 'data'), '-C', str(repo)]
    try:
        result = subprocess.run(base + ['agents', '--discover'], capture_output=True, text=True, timeout=60)
        (a.output / 'discovery.json').write_text(result.stdout)
        discovery = json.loads(result.stdout) if result.returncode == 0 else {}
        found = [item for item in discovery.get('agents', []) if item.get('agent') == a.agent]
        if not found:
            report['reason'] = 'ACP discovery failed; check official CLI login and transport arguments'
            report['failures'] = discovery.get('failures', [])
        else:
            report['models'] = [m['model'] for m in found[0]['capabilities']['models']]
            report['status'] = 'discovery_passed'
            if a.execute:
                start = time.monotonic()
                result = subprocess.run(base + ['--agent', a.agent, 'Create result.txt containing exactly OROCHI_E2E_OK followed by one newline. Do not modify any other files.'], capture_output=True, text=True, timeout=240)
                report['executed'] = True
                report['duration_seconds'] = round(time.monotonic() - start, 2)
                telemetry = subprocess.run(base + ['runs', '--limit', '5'], capture_output=True, text=True, timeout=15)
                runs = json.loads(telemetry.stdout) if telemetry.returncode == 0 else []
                (a.output / 'runs.json').write_text(json.dumps(runs, indent=2) + '\n')
                verified = result.returncode == 0 and (repo / 'result.txt').is_file() and (repo / 'result.txt').read_text() == 'OROCHI_E2E_OK\n' and any(r['outcome'] == 'success' for r in runs)
                report['status'] = 'passed' if verified else 'failed'
                report['exit_code'] = result.returncode
    except (subprocess.TimeoutExpired, ValueError, OSError) as error:
        report['reason'] = type(error).__name__
save()
sys.exit(0 if report['status'] in ('passed', 'discovery_passed') else 2)
