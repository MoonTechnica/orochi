"""When the sandbox VM powers itself off (`src/sandbox/idle.py`)."""
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


class IdleTest(unittest.TestCase):
    def test_a_command_running_inside_a_sandbox_is_use(self):
        running = {"running": [{"status": "Running", "description": "Executing command", "class": "websocket"}]}
        self.assertTrue(idle.running_exec(running))
        other = {"running": [{"status": "Running", "description": "Creating instance", "class": "task"}]}
        self.assertFalse(idle.running_exec(other))
        self.assertFalse(idle.running_exec(None))

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
        self.assertEqual(idle.decide(1000, 0, 30, True), "active")
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
