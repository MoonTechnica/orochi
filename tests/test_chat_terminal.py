"""What the chat actually draws, read back through a terminal model.

The pinned input and the transcript share one screen, so redraw bugs only show as overwritten
or leftover rows — never as a wrong string in a log. This drives the real binary in a pty with
the mock agent and reads the resulting screen.
"""
import fcntl
import importlib.util
import json
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
# Ceilings, not pacing: every wait returns the moment its condition holds, so a generous one
# costs nothing on a fast machine and is the difference between a real failure and a loaded
# runner on a slow one. A three-seat turn starts three agent sessions and 60s was not enough
# for it on macOS CI (2026-09-24), which failed in a different test each time it ran.
START, TURN = 60, 180


def config(
    dir: Path, lines: int, delay: float = 0.01, finish: float = 0.0, seats: int = 0
) -> Path:
    path = dir / "config.toml"
    path.write_text(f"""
[discovery]
auto_add = false
[evaluator]
auto = false
[classifier]
enabled = false
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
MOCK_MID_REPLY = "{"1" if seats else ""}"
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
        pump(START, until=lambda s: "What do you want to build?" in "\n".join(s.text()))
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
            pump(START, until=lambda s: any(message[-8:] in row for row in s.text()))
            os.write(master, b"\n")
            if wait:
                pump(TURN, until=lambda s: done(s, index))
            elif index < len(messages):
                pump(TURN, until=open_row)
        pump(TURN, until=lambda s: done(s, len(messages)))
        if not done(screen, len(messages)):
            # Said plainly, because the assertion that follows would otherwise read as a
            # drawing bug: a turn that never finished leaves a screen with nothing on it.
            raise AssertionError(
                f"only {sum('Fixture completed.' in r for r in screen.text())} of "
                f"{len(messages)} turns finished within {TURN}s:\n" + "\n".join(screen.text())
            )
        os.write(master, b"\x04\x04")
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
        pump(START, until=lambda s: "What do you want to build?" in "\n".join(s.text()))
        if during_turn:
            # Candidates have to survive the agent streaming into the transcript above them.
            os.write(master, b"keep talking\n")
            pump(START, until=lambda s: any("the fixture keeps talking" in r for r in s.text()))
        for chunk in chunks:
            os.write(master, chunk.encode())
            pump(1.5)
            shots.append(list(screen.text()))
        os.write(master, b"\x04\x04")
        pump(5)
    finally:
        process.terminate()
        process.wait(timeout=10)
        os.close(master)
    return shots


def divided(env="", stop_at=None, answer=b"\n", follow=None):
    """A design step that divides the work, the question answered with `answer` (Enter takes
    the team), and whatever runs next. `env` adds fixture settings; with `stop_at`, Ctrl-C is
    pressed once that text is on screen; `follow` is sent as the next message. Returns every
    row drawn, the repository, the data directory and the prompts the agents received."""
    dir = Path(tempfile.mkdtemp())
    repo = dir / "repo"
    repo.mkdir()
    meeting = dir / "rendezvous"
    meeting.mkdir()
    parts = [
        {"id": "alpha", "brief": "build alpha", "paths": ["alpha"]},
        {"id": "beta", "brief": "build beta", "paths": ["beta"]},
        {"id": "gamma", "brief": "build gamma", "paths": ["gamma"], "after": ["alpha", "beta"]},
    ]
    design = "Build alpha and beta side by side, then gamma.\n```json\n" + json.dumps({"parts": parts}) + "\n```\n"
    path = dir / "config.toml"
    path.write_text(f"""
[discovery]
auto_add = false
[evaluator]
auto = false
[[evaluator.checks]]
name = "integrated"
command = "python3"
args = ["-c", "from pathlib import Path; assert Path('completed.txt').read_text() == 'integrated'"]
[classifier]
enabled = false
[mailbox]
enabled = false
[scheduler]
discovery_timeout_secs = 20
prompt_timeout_secs = 40
[[agents]]
id = "test"
provider = "openai"
command = "python3"
args = ["{TESTS / 'fixtures/mock_acp.py'}"]
[agents.env]
MOCK_BEHAVIOR = "session_collaboration"
MOCK_MODELS = "sol-test,astra-test"
MOCK_DESIGN_REPLY = {json.dumps(design)}
MOCK_PARTS = "1"
MOCK_PART_NEEDS = "gamma:alpha/beta"
MOCK_EXPECT_PARTS = "alpha,beta,gamma"
MOCK_RENDEZVOUS = "{meeting}"
MOCK_LOG = "{dir / 'agent.jsonl'}"
{env or 'MOCK_RENDEZVOUS_PARTS = "alpha,beta"'}
""")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    process = subprocess.Popen(
        [BINARY, "--config", str(path), "--data-dir", str(dir / "data"), "-C", str(repo), "chat"],
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

    shown = lambda text: lambda s: any(text in row for row in s.text())
    try:
        pump(START, until=shown("What do you want to build?"))
        os.write(master, b"rewrite the entire architecture from scratch")
        pump(START, until=shown("from scratch"))
        os.write(master, b"\n")
        pump(TURN, until=shown("This divides into parts"))
        os.write(master, answer)
        completed = lambda n: lambda s: sum("Fixture completed." in row for row in s.text()) >= n
        if answer == b"\n":
            if stop_at:
                pump(TURN, until=shown(stop_at))
                os.write(master, b"\x03")
            pump(120, until=lambda s: shown("team finished")(s) or shown("team stopped")(s))
        elif answer == b"\x1b":
            pump(TURN, until=shown("keeping the design"))
        else:
            # One agent carries on: implementation, then its review.
            pump(TURN, until=completed(2))
        if follow:
            done = sum("Fixture completed." in row for row in screen.text())
            for start in range(0, len(follow), 8):
                os.write(master, follow[start:start + 8].encode())
                pump(0.05)
            os.write(master, b"\n")
            pump(TURN, until=completed(done + 1))
        os.write(master, b"\x04\x04")
        pump(5)
    finally:
        process.terminate()
        process.wait(timeout=10)
        os.close(master)
    prompts = [
        request["params"]["prompt"][0]["text"]
        for request in map(json.loads, (dir / "agent.jsonl").read_text().splitlines())
        if request.get("method") == "session/prompt"
    ]
    return screen.text(), repo, dir / "data", prompts


class Console:
    """One live session in a pty, driven a key at a time."""

    def __init__(self, behavior="success", env=None, lines=0, delay=0.01, classifier=False, args=(),
                 files=None):
        self.dir = Path(tempfile.mkdtemp())
        self.repo = self.dir / "repo"
        self.repo.mkdir()
        # What the repository holds before the session starts, such as its own .mcp.json.
        for name, text in (files or {}).items():
            (self.repo / name).write_text(text)
        self.log = self.dir / "agent.jsonl"
        extra = "\n".join(f"{key} = {json.dumps(value)}" for key, value in (env or {}).items())
        path = self.dir / "config.toml"
        path.write_text(f"""
[discovery]
auto_add = false
[evaluator]
auto = false
[classifier]
enabled = {"true" if classifier else "false"}
[mailbox]
enabled = false
[scheduler]
discovery_timeout_secs = 20
prompt_timeout_secs = 40
[[agents]]
id = "test"
provider = "openai"
command = "python3"
args = ["{TESTS / 'fixtures/mock_acp.py'}"]
[agents.env]
MOCK_BEHAVIOR = "{behavior}"
MOCK_MODELS = "sol-test,astra-test"
MOCK_LINES = "{lines}"
MOCK_LINE_DELAY = "{delay}"
MOCK_LOG = "{self.log}"
{extra}
""")
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.process = subprocess.Popen(
            [BINARY, "--config", str(path), "--data-dir", str(self.dir / "data"),
             "-C", str(self.repo), *args, "chat"],
            stdin=slave, stdout=slave, stderr=slave, close_fds=True,
            env=dict(os.environ, TERM="xterm-256color", COLUMNS=str(COLS), LINES=str(ROWS)))
        os.close(slave)
        self.screen = screen_module.Screen(ROWS, COLS)
        self.wait("What do you want to build?")

    def pump(self, seconds, until=None):
        end = time.time() + seconds
        while time.time() < end:
            if until and until(self.screen):
                return
            readable, _, _ = select.select([self.master], [], [], 0.05)
            if not readable:
                if self.process.poll() is not None:
                    return
                continue
            try:
                data = os.read(self.master, 65536)
            except OSError:
                return
            if not data:
                return
            self.screen.feed(data.decode("utf-8", "replace"))

    def shown(self, text):
        return any(text in row for row in self.screen.text())

    def wait(self, text, seconds=TURN):
        self.pump(seconds, until=lambda s: self.shown(text))
        assert self.shown(text), f"never showed {text!r}:\n" + "\n".join(self.screen.text())

    def type(self, text):
        for start in range(0, len(text), 8):
            os.write(self.master, text[start:start + 8].encode())
            self.pump(0.05)
        self.pump(0.3)

    def key(self, data, settle=0.4):
        os.write(self.master, data)
        self.pump(settle)

    def esc(self, settle=0.4):
        # A lone Esc: nothing may follow within the decoder's 40 ms, or it reads a sequence.
        self.key(b"\x1b", settle)

    def input(self):
        """The pinned input, first row to last: it starts at the last row that opens with >."""
        rows = self.screen.text()
        starts = [index for index, row in enumerate(rows) if row.startswith(">")]
        return rows[starts[-1]:] if starts else []

    def alive(self):
        return self.process.poll() is None

    def exited(self, seconds=10):
        end = time.time() + seconds
        while time.time() < end and self.alive():
            self.pump(0.2)
        return not self.alive()

    def requests(self, method):
        if not self.log.exists():
            return []
        return [r for r in map(json.loads, self.log.read_text().splitlines()) if r.get("method") == method]

    def prompts(self):
        return [r["params"]["prompt"][0]["text"] for r in self.requests("session/prompt")]

    def rows(self):
        return "\n".join(self.screen.text())

    def close(self):
        if self.alive():
            self.process.terminate()
            self.process.wait(timeout=10)
        os.close(self.master)


class ScreenModel(unittest.TestCase):
    """The screen these tests read the console back through has to survive a pty read that
    ends mid-escape. It did not: the ESC was dropped and the rest of the sequence drawn as
    text, so a correct redraw read back as a stale row and the suite failed at whatever rate
    the reads happened to split (macOS CI, 2026-09-24)."""

    def test_a_sequence_split_across_reads_draws_the_same_as_an_unsplit_one(self):
        whole = "ab\x1b[2;1Hcd\x1b[Kef\x1b[1;1Hgh"
        one = screen_module.Screen(rows=3, cols=10)
        one.feed(whole)
        for at in range(1, len(whole)):
            split = screen_module.Screen(rows=3, cols=10)
            split.feed(whole[:at])
            split.feed(whole[at:])
            self.assertEqual(split.text(), one.text(), f"split after {whole[:at]!r}")

    def test_an_escape_the_screen_does_not_know_is_still_skipped(self):
        # Only a tail that could still grow into a sequence is held over. An escape this
        # screen has no rule for is passed over exactly as it always was — the ESC alone —
        # and is not buffered waiting for a completion that will never come.
        one = screen_module.Screen(rows=2, cols=10)
        one.feed("a\x1bZb")
        self.assertEqual(one.text()[0], "aZb")
        self.assertEqual(one.pending, "")


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

    def test_a_design_that_divides_the_work_is_offered_and_run_side_by_side(self):
        rows, repo, _, _ = divided()
        text = "\n".join(rows)
        # Asked once, after the design and with the order it would run in.
        self.assertTrue(any("alpha ‖ beta → gamma" in row for row in rows), text)
        self.assertTrue(any("team finished · result merged" in row for row in rows), text)
        # The division is for Orochi; the design itself stays in the transcript.
        self.assertTrue(any("Build alpha and beta side by side" in row for row in rows), text)
        self.assertFalse(any('"parts"' in row or "```" in row for row in rows), text)
        for part in ("alpha", "beta", "gamma"):
            self.assertEqual((repo / part / "done.txt").read_text(), part)
        self.assertEqual((repo / "completed.txt").read_text(), "integrated")

    def test_interrupting_a_running_team_stops_it_and_leaves_it_resumable(self):
        rows, repo, data, _ = divided(
            env='MOCK_FAIL_ROLE = "implementer"\nMOCK_FAIL_PART = "alpha"\nMOCK_FAILURE_KIND = "hang"',
            stop_at="Stage 0, alpha",
        )
        text = "\n".join(rows)
        self.assertTrue(any("team stopped" in row for row in rows), text)
        self.assertTrue(any("collaborate-resume" in row for row in rows), text)
        (report,) = (data / "collaborations").glob("*/report.json")
        self.assertEqual(json.loads(report.read_text())["status"], "cancelled")
        self.assertFalse((repo / "alpha").exists())
    def test_esc_on_the_team_question_keeps_the_design_and_runs_nothing(self):
        # As declining a plan in Claude Code: nothing is built, and the next message carries on
        # from the design.
        rows, repo, data, prompts = divided(answer=b"\x1b", follow="go on")
        text = "\n".join(rows)
        self.assertFalse((data / "collaborations").exists(), text)
        self.assertFalse(any("following the plan above" in p for p in prompts), prompts)
        self.assertTrue(any("keeping the design" in row for row in rows), text)
        self.assertIn("Build alpha and beta side by side", prompts[-1])
        self.assertTrue(prompts[-1].rstrip().endswith("go on"), prompts[-1])

    def test_choosing_one_agent_carries_on_in_the_working_tree(self):
        rows, repo, data, prompts = divided(answer=b"2")
        self.assertFalse((data / "collaborations").exists(), "\n".join(rows))
        self.assertTrue(any("following the plan above" in p for p in prompts), prompts)


class ConsoleKeys(unittest.TestCase):
    """Esc, Ctrl-C, Ctrl-D and queued messages behave as in Claude Code
    (code.claude.com/docs/en/interactive-mode, /permissions)."""

    def test_esc_at_an_idle_prompt_never_exits_and_twice_clears_the_draft(self):
        c = Console()
        try:
            c.type("draft text")
            c.esc()
            self.assertTrue(c.alive())
            self.assertIn("draft text", c.input()[0], c.rows())
            c.esc(settle=0.15)
            c.esc()
            self.assertNotIn("draft text", " ".join(c.input()), c.rows())
            # The cleared draft went to history.
            c.key(b"\x1b[A")
            self.assertIn("draft text", c.input()[0], c.rows())
            for _ in range(4):
                c.esc(settle=0.15)
            c.pump(0.5)
            self.assertTrue(c.alive(), c.rows())
        finally:
            c.close()

    def test_ctrl_c_clears_the_input_first_and_exits_on_the_second_press(self):
        c = Console()
        try:
            c.type("half a thought")
            c.key(b"\x03")
            self.assertNotIn("half a thought", " ".join(c.input()), c.rows())
            self.assertTrue(c.shown("Press Ctrl-C again to exit"), c.rows())
            self.assertTrue(c.alive())
            c.key(b"\x03")
            self.assertTrue(c.exited(), c.rows())
        finally:
            c.close()

    def test_ctrl_d_deletes_forward_and_asks_before_exiting(self):
        c = Console()
        try:
            c.type("ab")
            c.key(b"\x01")
            c.key(b"\x04")
            self.assertEqual(c.input()[0], "> b", c.rows())
            c.key(b"\x05")
            c.key(b"\x7f")
            c.key(b"\x04")
            self.assertTrue(c.shown("Press Ctrl-D again to exit"), c.rows())
            self.assertTrue(c.alive())
            # Too late to count as the second press: it asks again.
            time.sleep(1.0)
            c.key(b"\x04", settle=0.1)
            self.assertTrue(c.alive(), c.rows())
            c.key(b"\x04", settle=0.1)
            self.assertTrue(c.exited(), c.rows())
        finally:
            c.close()

    def test_ctrl_c_during_a_turn_stops_it_and_keeps_what_was_typed(self):
        c = Console(lines=60, delay=0.15)
        try:
            c.type("start working")
            c.key(b"\n")
            c.wait("line 2:")
            c.type("half typed")
            c.key(b"\x03")
            c.wait("Interrupted")
            self.assertIn("half typed", c.input()[0], c.rows())
            self.assertFalse(c.shown("line 60:"), c.rows())
            self.assertTrue(c.alive())
        finally:
            c.close()

    def test_esc_stops_the_turn_and_sends_what_was_queued_next(self):
        c = Console(lines=60, delay=0.15)
        try:
            c.type("first")
            c.key(b"\n")
            c.wait("line 2:")
            c.type("second")
            c.key(b"\n")
            c.wait("queued (1)")
            c.esc()
            c.wait("Interrupted")
            c.pump(START, until=lambda s: len(c.prompts()) >= 2)
            prompts = c.prompts()
            self.assertEqual(len(prompts), 2, prompts)
            self.assertTrue(prompts[1].rstrip().endswith("second"), prompts[1])
        finally:
            c.close()

    def test_up_takes_back_what_was_queued(self):
        c = Console(lines=60, delay=0.15)
        try:
            c.type("first")
            c.key(b"\n")
            c.wait("line 2:")
            for index, text in enumerate(["second", "third"], start=1):
                c.type(text)
                c.key(b"\n")
                c.wait(f"queued ({index})")
            c.key(b"\x1b[A")
            pinned = c.input()
            self.assertIn("second", pinned[0], c.rows())
            self.assertTrue(any("third" in row for row in pinned), c.rows())
            c.esc()
            c.wait("Interrupted")
            c.pump(2)
            # Nothing is left in the queue to send on its own.
            self.assertEqual(len(c.prompts()), 1, c.prompts())
            c.key(b"\n")
            c.pump(START, until=lambda s: len(c.prompts()) >= 2)
            self.assertIn("second\nthird", c.prompts()[1])
        finally:
            c.close()

    def test_esc_on_a_permission_prompt_declines_and_stops_the_turn(self):
        # The console answers permission requests itself unless asked to stop and ask.
        c = Console(behavior="permission_persist", args=("--permission", "ask"))
        try:
            c.type("edit the notes")
            c.key(b"\n")
            c.wait("Permission required")
            # The choices read as Claude Code's, with No last and on Esc.
            self.assertTrue(c.shown("1. Yes"), c.rows())
            self.assertTrue(c.shown("2. Yes, and don't ask again this session"), c.rows())
            self.assertTrue(c.shown("3. No, and say what to do instead (esc)"), c.rows())
            c.esc()
            c.wait("Stopped because the request was denied")
            # The panel grew over the transcript and shrank again; what was above it stays.
            self.assertTrue(c.shown("> edit the notes"), c.rows())
            self.assertTrue(c.shown("test · sol-test"), c.rows())
            # Told only "no", the fixture would try another way after 1.5 s; a stopped turn
            # never gets there.
            c.pump(2.5)
            self.assertFalse((c.repo / "alternative.txt").exists(), c.rows())
        finally:
            c.close()

    def test_esc_while_the_request_is_classified_starts_no_agent(self):
        c = Console(classifier=True, env={
            "MOCK_CLASSIFICATION": '{"task_type":"implementation","complexity":"normal","ambiguity":0.1}',
            "MOCK_CLASSIFY_DELAY": "3",
        })
        try:
            c.type("add a health endpoint")
            c.key(b"\n")
            c.pump(START, until=lambda s: len(c.prompts()) >= 1)
            c.esc()
            c.wait("Interrupted")
            c.pump(4)
            prompts = c.prompts()
            self.assertEqual(len(prompts), 1, prompts)
            self.assertIn("You are a task classifier", prompts[0])
            self.assertTrue(c.alive())
        finally:
            c.close()


if __name__ == "__main__":
    unittest.main()


class RepositoryMcpServers(unittest.TestCase):
    """A repository's own .mcp.json servers are asked about as Claude Code 2.1.278 asks: before
    they are used, with its wording and options, "Continue without" focused by default, Esc as
    "no", and the answer remembered for the repository."""

    ONE = json.dumps({"mcpServers": {"notes": {"command": "python3", "args": ["-c", "pass"]}}})
    TWO = json.dumps({"mcpServers": {"alpha": {"command": "python3"}, "beta": {"command": "python3"}}})

    def given(self, c):
        """The MCP servers the agent's session was opened with."""
        c.type("hello")
        c.key(b"\n")
        c.wait("Fixture completed.")
        return [s["name"] for s in c.requests("session/new")[-1]["params"].get("mcpServers", [])]

    def test_a_repository_server_is_asked_about_first_and_esc_refuses_it_for_good(self):
        c = Console(files={".mcp.json": self.ONE})
        try:
            c.wait("New MCP server found in this project: notes")
            # Each option is waited for, not merely looked for: they are drawn after the
            # heading, so a read that lands between the two saw a half-drawn dialog and
            # failed on a console that was about to be right (macOS CI, 2026-09-24).
            for option in ("Use this MCP server", "Use this and all future MCP servers in this project",
                           "Continue without using this MCP server"):
                c.wait(option)
            self.assertTrue(any("❯" in row and "Continue without using this MCP server" in row
                                for row in c.screen.text()), "no is the default:\n" + c.rows())
            c.esc()
            c.wait("not using notes")
            self.assertEqual(self.given(c), [], c.rows())
            choices = json.loads((c.dir / "data/mcp/choices.json").read_text())
            self.assertEqual([list(v["disabled"]) for v in choices.values()], [["notes"]])
        finally:
            c.close()

    def test_an_approved_repository_server_reaches_the_agent(self):
        c = Console(files={".mcp.json": self.ONE})
        try:
            c.wait("New MCP server found in this project: notes")
            c.key(b"1")
            c.wait("using notes from this repository's .mcp.json")
            self.assertEqual(self.given(c), ["notes"], c.rows())
        finally:
            c.close()

    def test_several_new_servers_are_one_checklist_every_one_ticked_to_begin_with(self):
        c = Console(files={".mcp.json": self.TWO})
        try:
            c.wait("2 new MCP servers found in this project")
            # Waited for, for the same reason: the checklist is drawn after its heading.
            c.wait("Select any you wish to enable.")
            c.wait("[✔] alpha")
            c.wait("[✔] beta")
            c.key(b" ")
            c.wait("[ ] alpha")
            c.key(b"\n")
            c.wait("using beta from this repository's .mcp.json")
            self.assertEqual(self.given(c), ["beta"], c.rows())
        finally:
            c.close()
