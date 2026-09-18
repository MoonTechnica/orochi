"""Run a no-account ACP demo in a temporary workspace; no external API calls."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

project = Path(__file__).resolve().parents[1]
binary = project / "target/debug/orochi"
if not binary.exists():
    raise SystemExit("Run cargo build --locked first.")

with tempfile.TemporaryDirectory(prefix="orochi-demo-") as temp:
    work = Path(temp)
    repo = work / "repo"
    repo.mkdir()
    config = work / "config.toml"
    # JSON string escaping is also valid for these TOML basic strings.
    config.write_text("\n".join([
        "[discovery]", "auto_add = false",
        "[[agents]]", 'id = "demo"', 'provider = "openai"',
        "command = " + json.dumps(sys.executable),
        "args = [" + json.dumps(str(project / "tests/fixtures/mock_acp.py")) + "]",
        "[scheduler]", 'permission = "deny"',
        "[evaluator]", "auto = false",
        "[[evaluator.checks]]", 'name = "tests"', "command = " + json.dumps(sys.executable),
        'args = ["-c", "from pathlib import Path; assert Path(\'completed.txt\').exists()"]',
    ]))
    base = [str(binary), "--config", str(config), "--data-dir", str(work / "data"), "--cwd", str(repo)]
    for args in [
        ["--dry-run", "--json", "Implement a small endpoint"],
        ["Implement a small endpoint"],
        ["status"],
        ["sessions"],
    ]:
        print("\n$ orochi " + " ".join(args), flush=True)
        subprocess.run(base + args, check=True)
