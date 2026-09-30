"""When the sandbox VM powers itself off (`src/sandbox/idle.py`): only once no agent's work is
alive in any sandbox and nothing is connected from the Mac."""
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "src" / "sandbox"))
import idle  # noqa: E402

SS = """0 0 127.0.0.1:1355 127.0.0.1:50212
0 0 10.203.0.1:53 10.203.0.7:40000
0 0 127.0.0.1:54323 127.0.0.1:50300
"""


def fake_proc(processes):
    """A /proc with (pid, name, state, environ) entries."""
    root = tempfile.mkdtemp()
    for pid, name, state, environ in processes:
        d = os.path.join(root, str(pid))
        os.mkdir(d)
        Path(d, "environ").write_bytes(b"\0".join(f"{k}={v}".encode() for k, v in environ.items()) + b"\0")
        Path(d, "stat").write_bytes(f"{pid} ({name}) {state} 1 1 1".encode())
        Path(d, "comm").write_bytes(f"{name}\n".encode())
    os.mkdir(os.path.join(root, "self-not-a-pid"))
    return root


class IdleTest(unittest.TestCase):
    def test_an_agent_and_everything_it_started_are_work_wherever_they_run(self):
        proc = fake_proc([
            (10, "claude-agent-a", "S", {"OROCHI_SANDBOX": "demo", "HOME": "/home/dev"}),
            # A build the agent left running after its turn, detached into its own session.
            (11, "node", "R", {"OROCHI_SANDBOX": "demo", "NODE_ENV": "production"}),
            (20, "codex-acp", "S", {"OROCHI_SANDBOX": "web"}),
            # Supabase idling and the VM's own services are not an agent's work.
            (30, "postgres", "S", {"PGDATA": "/var/lib/postgresql"}),
            (31, "sshd", "S", {}),
            # A process that has exited is not work either.
            (40, "sleep", "Z", {"OROCHI_SANDBOX": "demo"}),
        ])
        work = idle.agent_work(proc)
        self.assertEqual(sorted(p for p, _, _ in work), [10, 11, 20])
        self.assertEqual(idle.summary(work), {"demo": ["claude-agent-a", "node"], "web": ["codex-acp"]})

    def test_an_incus_operation_in_progress_is_use(self):
        running = {"running": [{"status": "Running", "description": "Publishing image"},
                               {"status": "Running", "description": "Executing command"}]}
        self.assertEqual(idle.incus_busy(running), ["Executing command", "Publishing image"])
        self.assertEqual(idle.incus_busy({"running": []}), [])
        self.assertEqual(idle.incus_busy(None), [])

    def test_no_agent_work_is_no_work(self):
        self.assertEqual(idle.agent_work(fake_proc([(30, "postgres", "S", {"X": "1"})])), [])

    def test_a_connection_through_the_gateway_or_a_focused_port_is_use(self):
        self.assertTrue(idle.connected(SS, {1355}))
        self.assertTrue(idle.connected(SS, {54323}))
        self.assertFalse(idle.connected(SS, {3000}), "a sandbox's own DNS traffic is not use")
        self.assertFalse(idle.connected("", {1355}))

    def test_focused_ports_are_read_from_proxy_devices(self):
        instances = [{"devices": {
            "sbx-port-54323": {"type": "proxy", "listen": "tcp:127.0.0.1:54323"},
            "work": {"type": "disk", "source": "/x"},
        }}]
        self.assertEqual(idle.focused_ports(instances), {54323})

    def test_the_vm_stops_only_after_the_whole_idle_time_with_nothing_in_use(self):
        self.assertEqual(idle.decide(10**9, 0, 30, True), "active", "use resets the clock, however long")
        self.assertEqual(idle.decide(29 * 60, 0, 30, False), "wait")
        self.assertEqual(idle.decide(30 * 60, 0, 30, False), "stop")
        self.assertEqual(idle.decide(10**9, 0, 0, False), "wait", "0 keeps it running")

    def test_settings_come_from_what_orochi_wrote_and_default_to_thirty_minutes(self):
        with tempfile.NamedTemporaryFile("w", suffix=".conf", delete=False) as f:
            f.write("SBX_VM_IDLE_MINUTES=5\nSBX_GATEWAY_PORT=2000\n")
        self.assertEqual(idle.settings(f.name), (5, 2000))
        self.assertEqual(idle.settings("/nonexistent"), (30, 1355))


if __name__ == "__main__":
    unittest.main()
