"""The sandboxes' HTTP gateway (`src/sandbox/gateway.py`), run as the VM runs it, on loopback."""
import http.server
import os
import socket
import subprocess
import sys
import threading
import time
import unittest
from pathlib import Path

GATEWAY = Path(__file__).resolve().parent.parent / "src" / "sandbox" / "gateway.py"
sys.dont_write_bytecode = True
sys.path.insert(0, str(GATEWAY.parent))
import gateway  # noqa: E402


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Echo(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.headers.get("Upgrade") == "websocket":
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.end_headers()
            self.wfile.flush()
            data = self.rfile.read(5)
            self.wfile.write(b"echo:" + data)
            self.wfile.flush()
            return
        body = f"path={self.path} host={self.headers['Host']}".encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def ask(port, host, request=None):
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        s.sendall(request or f"GET /x?y=1 HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n".encode())
        chunks = []
        while data := s.recv(65536):
            chunks.append(data)
        return b"".join(chunks).decode(errors="replace")


class GatewayTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.service = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Echo)
        cls.service_port = cls.service.server_address[1]
        threading.Thread(target=cls.service.serve_forever, daemon=True).start()
        cls.port = free_port()
        env = dict(os.environ, SBX_GATEWAY_PORT=str(cls.port), SBX_GATEWAY_UPSTREAM="127.0.0.1")
        cls.process = subprocess.Popen([sys.executable, str(GATEWAY)], env=env)
        for _ in range(100):
            try:
                socket.create_connection(("127.0.0.1", cls.port), timeout=0.1).close()
                break
            except OSError:
                time.sleep(0.05)

    @classmethod
    def tearDownClass(cls):
        cls.process.kill()
        cls.process.wait()
        cls.service.shutdown()

    def test_a_port_and_project_named_in_the_host_reach_that_service_unchanged(self):
        host = f"{self.service_port}-demo.localhost:{self.port}"
        reply = ask(self.port, host)
        self.assertIn("200 OK", reply)
        self.assertIn(f"path=/x?y=1 host={host}", reply, "the request arrives as it was sent")

    def test_a_host_that_names_no_sandbox_is_refused_with_how_to_name_one(self):
        reply = ask(self.port, "example.com")
        self.assertIn("400", reply)
        self.assertIn(f"http://<port>-<project>.localhost:{self.port}/", reply)

    def test_nothing_listening_is_reported_rather_than_left_hanging(self):
        reply = ask(self.port, f"{free_port()}-demo.localhost:{self.port}")
        self.assertIn("502", reply)
        self.assertIn("orochi sandbox focus", reply)

    def test_an_upgraded_connection_passes_bytes_both_ways(self):
        host = f"{self.service_port}-demo.localhost:{self.port}"
        with socket.create_connection(("127.0.0.1", self.port), timeout=5) as s:
            s.sendall(f"GET /ws HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n".encode())
            head = b""
            while b"\r\n\r\n" not in head:
                head += s.recv(1024)
            self.assertIn(b"101", head)
            s.sendall(b"hello")
            reply = head.split(b"\r\n\r\n", 1)[1]
            while len(reply) < len(b"echo:hello"):
                reply += s.recv(1024)
            self.assertEqual(reply, b"echo:hello")

    def test_names_are_read_as_incus_and_dns_allow_them(self):
        self.assertEqual(gateway.route("54323-demo.localhost:1355"), ("demo", 54323))
        self.assertEqual(gateway.route("3000-My-App.LOCALHOST"), ("my-app", 3000))
        for host in ["demo.localhost:1355", "0-demo.localhost", "70000-demo.localhost",
                     "3000-demo-.localhost", "3000-9demo.localhost", "3000-demo.localhost.evil.com",
                     "3000-demo.sbx"]:
            self.assertIsNone(gateway.route(host), host)


if __name__ == "__main__":
    unittest.main()
