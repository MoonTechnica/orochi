"""Exercise the installed group-access wrapper without installing or using Incus."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


class AccessWrapperTest(unittest.TestCase):
    def test_arguments_survive_the_group_shell_without_execution(self):
        source = (Path(__file__).resolve().parents[1] / "src/sandbox/linux-host.sh").read_text()
        wrapper = source.split("<<'WRAPPER'\n", 1)[1].split("\nWRAPPER", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "wrapper.py").write_text(wrapper)
            for name, body in {
                "sg": "import os,sys\nassert sys.argv[1:3] == ['incus-admin', '-c']\nos.execv('/bin/sh', ['/bin/sh', '-c', sys.argv[3]])\n",
                "incus": "import json,sys\nprint(json.dumps(sys.argv[1:]))\n",
            }.items():
                path = root / name
                path.write_text("#!" + sys.executable + "\n" + body)
                path.chmod(0o755)
            sentinel = root / "executed"
            args = ["raw.idmap", "uid 501 501\ngid 20 20", "with 'quotes'", "", "日本語",
                    f"$(touch {sentinel})", "; echo injected"]
            env = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"])
            output = subprocess.check_output([sys.executable, str(root / "wrapper.py"), *args], env=env, text=True)
            self.assertEqual(json.loads(output), args)
            self.assertFalse(sentinel.exists())


if __name__ == "__main__":
    unittest.main()
