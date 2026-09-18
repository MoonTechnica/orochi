"""What the chat actually draws, read back through a terminal model.

The pinned input and the transcript share one screen, so redraw bugs only show as overwritten
or leftover rows — never as a wrong string in a log. This drives the real binary in a pty with
the mock agent and reads the resulting screen.
"""
import fcntl
import importlib.util
import os
import pty
import re
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest
from pathlib import Path

TESTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("screen", TESTS / "screen.py")
screen_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(screen_module)

ROWS, COLS = 24, 80
BINARY = os.environ.get("OROCHI_BIN", str(TESTS.parent / "target/debug/orochi"))


def config(
    dir: Path, lines: int, delay: float = 0.01, finish: float = 0.0, seats: int = 0
) -> Path:
    path = dir / "config.toml"
    path.write_text(f"""
[discovery]
auto_add = false
[evaluator]
auto = false
[mailbox]
enabled = {"true" if seats else "false"}
[scheduler]
discovery_timeout_secs = 20
prompt_timeout_secs = 40
[[agents]]
id = "test"
provider = "openai"
command = "python3"
args = ["{TESTS / 'fixtures/mock_acp.py'}"]
[agents.env]
MOCK_BEHAVIOR = "{"seats" if seats else "success"}"
MOCK_MODELS = "sol-test,astra-test"
MOCK_SEATS = "{seats}"
MOCK_LINES = "{lines}"
MOCK_LINE_DELAY = "{delay}"
MOCK_FINISH_DELAY = "{finish}"
""")
    return path


def chat(messages, lines, wait=True, delay=0.01, finish=0.0, seats=0):
    """Sends each message and returns every row the session drew, oldest first. With
    `wait=False` the next message is typed while the agent is still streaming, so it queues."""
    dir = Path(tempfile.mkdtemp())
    (dir / "repo").mkdir()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    process = subprocess.Popen(
        [BINARY, "--config", str(config(dir, lines, delay, finish, seats)), "--data-dir", str(dir / "data"),
         "-C", str(dir / "repo"), "chat"],
        stdin=slave, stdout=slave, stderr=slave, close_fds=True,
        env=dict(os.environ, TERM="xterm-256color", COLUMNS=str(COLS), LINES=str(ROWS)))
    os.close(slave)
    screen = screen_module.Screen(ROWS, COLS)

    def pump(seconds, until=None):
        end = time.time() + seconds
        while time.time() < end:
            if until and until(screen):
                return
            readable, _, _ = select.select([master], [], [], 0.2)
            if not readable:
                continue
            try:
                data = os.read(master, 65536)
            except OSError:
                return
            if not data:
                return
            screen.feed(data.decode("utf-8", "replace"))

    try:
        done = lambda s, n: sum("Fixture completed." in row for row in s.text()) >= n
        pump(30, until=lambda s: "What do you want to build?" in "\n".join(s.text()))
        # `wait=False` types the next message while a row is half written, which is when a
        # queued line can land on top of the agent's own text.
        open_row = lambda s: any(
            re.search(r"line \d+:$", row) or row.endswith("Fixture completed.") for row in s.text()
        )
        for index, message in enumerate(messages, start=1):
            # Typed, not pasted: the input area grows row by row, exactly as a person sees it.
            for start in range(0, len(message), 8):
                os.write(master, message[start:start + 8].encode())
                pump(0.05)
            # Wait until the whole message is on screen: a long one wraps the input over
            # several rows, and those rows are freed again the moment it is sent.
            pump(10, until=lambda s: any(message[-8:] in row for row in s.text()))
            os.write(master, b"\n")
            if wait:
                pump(60, until=lambda s: done(s, index))
            elif index < len(messages):
                pump(60, until=open_row)
        pump(60, until=lambda s: done(s, len(messages)))
        os.write(master, b"\x04")
        pump(5)
    finally:
        process.terminate()
        process.wait(timeout=10)
        os.close(master)
    return screen.text()


def typing(chunks, lines=1, during_turn=False):
    """Types each chunk into a live session and returns the screen after each one, so a test
    can see the pinned area grow and shrink rather than only its final state."""
    dir = Path(tempfile.mkdtemp())
    (dir / "repo").mkdir()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    process = subprocess.Popen(
        [BINARY, "--config", str(config(dir, lines, 0.01, 0.0, 0)), "--data-dir", str(dir / "data"),
         "-C", str(dir / "repo"), "chat"],
        stdin=slave, stdout=slave, stderr=slave, close_fds=True,
        env=dict(os.environ, TERM="xterm-256color", COLUMNS=str(COLS), LINES=str(ROWS)))
    os.close(slave)
    screen = screen_module.Screen(ROWS, COLS)

    def pump(seconds, until=None):
        end = time.time() + seconds
        while time.time() < end:
            if until and until(screen):
                return
            readable, _, _ = select.select([master], [], [], 0.2)
            if not readable:
                continue
            try:
                data = os.read(master, 65536)
            except OSError:
                return
            if not data:
                return
            screen.feed(data.decode("utf-8", "replace"))

    shots = []
    try:
        pump(30, until=lambda s: "What do you want to build?" in "\n".join(s.text()))
        if during_turn:
            # Candidates have to survive the agent streaming into the transcript above them.
            os.write(master, b"keep talking\n")
            pump(30, until=lambda s: any("the fixture keeps talking" in r for r in s.text()))
        for chunk in chunks:
            os.write(master, chunk.encode())
            pump(1.5)
            shots.append(list(screen.text()))
        os.write(master, b"\x04")
        pump(5)
    finally:
        process.terminate()
        process.wait(timeout=10)
        os.close(master)
    return shots


class ChatTerminal(unittest.TestCase):
    def test_command_candidates_appear_above_the_input_and_free_their_rows(self):
        # "/n" matches, "/nzz" matches nothing, and backspacing returns to the match.
        shown, gone, back = typing(["/n", "zz", "\x7f\x7f"])

        def rows(screen):
            return [row for row in screen if row.strip()]

        listed = [row for row in rows(shown) if "/new" in row]
        self.assertTrue(listed, "\n".join(rows(shown)))
        # The candidate sits above the line being typed, which still shows what was typed.
        input_row = max(i for i, row in enumerate(shown) if row.strip().endswith("/n"))
        self.assertLess(
            max(i for i, row in enumerate(shown) if "/new" in row), input_row,
            "\n".join(rows(shown)),
        )
        # Nothing matches, so every candidate row is given back and cleared: a freed row must
        # not keep the text it had.
        self.assertFalse([row for row in gone if "/new" in row], "\n".join(rows(gone)))
        self.assertTrue([row for row in back if "/new" in row], "\n".join(rows(back)))

    def test_command_candidates_survive_an_agent_streaming_above_them(self):
        # Typed while a turn is running, which is its own key path and its own repainting.
        (shown,) = typing(["/n"], lines=30, during_turn=True)
        self.assertTrue(
            [row for row in shown if "/new" in row],
            "\n".join(row for row in shown if row.strip()),
        )

    def test_messages_from_other_agents_slot_between_rows_of_the_reply(self):
        rows = chat(["3人のエージェントでディスカッションして"], lines=8, delay=0.4, seats=2)
        body = [row for row in rows if "the fixture keeps talking" in row]
        self.assertEqual(len(body), 8, "\n".join(rows))
        # The reply is one block: a message arriving mid-answer does not start it again.
        self.assertEqual(sum(row.startswith("⏺ ") for row in body), 1, "\n".join(rows))
        mail = [index for index, row in enumerate(rows) if row.startswith("✉ ")]
        first, last = rows.index(body[0]), rows.index(body[-1])
        self.assertTrue(any(first < index < last for index in mail), "\n".join(rows))


    def test_a_message_typed_mid_turn_never_lands_on_what_the_agent_is_writing(self):
        rows = chat(
            ["最初の依頼です。", "実行中に入力した二つ目の依頼です。"],
            lines=10,
            wait=False,
            delay=0.3,
            finish=2.0,
        )
        streamed = [row for row in rows if "the fixture keeps talking" in row]
        for row in streamed:
            self.assertRegex(row, r"^\s*(⏺ )?line \d+: the fixture keeps talking$")
        self.assertTrue(any("queued (1)" in row for row in rows), "\n".join(rows))
        # The queued line must not eat the row the agent was writing.
        self.assertEqual(sum("Fixture completed." in row for row in rows), 2, "\n".join(rows))
        self.assertEqual(len(streamed), 20, "\n".join(rows))

    def test_every_turn_flows_on_without_overwriting_the_last_one(self):
        # Long enough to wrap: the input grows to several rows and shrinks back on Enter.
        # Long enough to wrap the input over several rows, so sending it frees more rows than
        # the echo covers: whatever is left there is what the next lines are written on top of.
        messages = ["一回目です。" + "長めの依頼を書いて折り返させます。" * 6,
                    "二回目です。" + "こちらも長めに書いて折り返させます。" * 6]
        rows = chat(messages, lines=12)
        streamed = [row for row in rows if "the fixture keeps talking" in row]
        self.assertEqual(len(streamed), 12 * len(messages), "\n".join(rows))
        # Nothing else may share a streamed row: leftovers of an earlier draw would land here.
        for row in streamed:
            self.assertRegex(row, r"^\s*(⏺ )?line \d+: the fixture keeps talking$")
        self.assertEqual(sum(row.startswith("> ") for row in rows), len(messages), "\n".join(rows))
        self.assertEqual(
            sum("Fixture completed." in row for row in rows), len(messages), "\n".join(rows)
        )
        # The input area shrinks back on Enter: none of what was typed may survive anywhere
        # but in its own echo or in a queued line.
        for row in rows:
            if "折り返させます" in row:
                self.assertTrue(
                    row.startswith("> ") or "queued" in row, f"leftover input in {row!r}"
                )
        for index in range(1, 13):
            self.assertEqual(
                sum(row.endswith(f"line {index}: the fixture keeps talking") for row in rows),
                len(messages),
                "\n".join(rows),
            )


if __name__ == "__main__":
    unittest.main()
