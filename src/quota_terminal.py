"""Read-only native CLI quota panels over a PTY. No prompts, credentials or raw screens are saved.
Python 3 + POSIX only. Unknown UI layouts/authentication states fail closed.
"""
import codecs
import datetime
import fcntl
import json
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
import unicodedata
from zoneinfo import ZoneInfo


class Screen:
    def __init__(self, width=160, height=80):
        self.width, self.height = width, height
        self.rows = [[' '] * width for _ in range(height)]
        self.x = self.y = 0
        self.pending = ''
        self.saved = (0, 0)

    def feed(self, text):
        text = self.pending + text
        self.pending = ''
        i = 0
        while i < len(text):
            c = text[i]
            if c == '\x1b':
                if i + 1 >= len(text):
                    self.pending = text[i:]; break
                if text[i+1] == '[':
                    m = re.match(r'\x1b\[([0-?]*)([ -/]*)([@-~])', text[i:])
                    if not m:
                        self.pending = text[i:]; break
                    raw, _, action = m.groups()
                    values = [int(v) if v.isdigit() else 0 for v in raw.lstrip('?<>').split(';')]
                    n = values[0] or 1
                    if not raw.startswith(('?', '>', '<')):
                        if action == 'A': self.y = max(0, self.y-n)
                        elif action == 'B': self.y = min(self.height-1, self.y+n)
                        elif action == 'C': self.x = min(self.width-1, self.x+n)
                        elif action == 'D': self.x = max(0, self.x-n)
                        elif action == 'G': self.x = min(self.width-1, n-1)
                        elif action in ('H','f'):
                            self.y = min(self.height-1, n-1)
                            self.x = min(self.width-1, (values[1] if len(values)>1 and values[1] else 1)-1)
                        elif action == 'K':
                            start = 0 if values[0] in (1,2) else self.x
                            end = self.x+1 if values[0] == 1 else self.width
                            self.rows[self.y][start:end] = [' '] * (end-start)
                        elif action == 'J':
                            if values[0] == 2: self.rows = [[' '] * self.width for _ in range(self.height)]
                            elif values[0] == 0:
                                self.rows[self.y][self.x:] = [' '] * (self.width-self.x)
                                for row in range(self.y+1, self.height): self.rows[row] = [' '] * self.width
                    i += len(m.group()); continue
                if text[i+1] == ']':
                    m = re.search(r'\x07|\x1b\\', text[i+2:])
                    if not m: self.pending = text[i:]; break
                    i += 2 + m.end(); continue
                if text[i+1] in '()':
                    if i+2 >= len(text): self.pending = text[i:]; break
                    i += 3; continue
                if text[i+1] == '7': self.saved = (self.x,self.y)
                elif text[i+1] == '8': self.x,self.y = self.saved
                i += 2; continue
            if c == '\r': self.x = 0
            elif c == '\n':
                self.y += 1
                if self.y >= self.height:
                    self.rows.pop(0); self.rows.append([' '] * self.width); self.y = self.height-1
            elif c == '\b': self.x = max(0,self.x-1)
            elif c == '\t': self.x = min(self.width-1, (self.x//8+1)*8)
            elif c >= ' ' and not unicodedata.combining(c):
                if self.x >= self.width:
                    self.x = 0; self.y = min(self.height-1, self.y+1)
                self.rows[self.y][self.x] = c
                self.x += 2 if unicodedata.east_asian_width(c) in ('W','F') else 1
            i += 1
        if len(self.pending) > 8192: raise ValueError('unsupported terminal escape')

    def text(self):
        return '\n'.join(''.join(row).rstrip() for row in self.rows)


def reset_at(text, now):
    # Claude displays a timezone explicitly. Never infer it from the machine locale.
    m = re.search(r'Resets\s+(?:(\w{3})\s+(\d{1,2})\s+at\s+)?(\d{1,2})(?::(\d{2}))?(am|pm)\s+\(([^)]+)\)', text)
    if not m: return None
    month, day, hour, minute, ampm, zone = m.groups()
    try:
        current = datetime.datetime.fromtimestamp(now, ZoneInfo(zone))
        hour = int(hour) % 12 + (12 if ampm == 'pm' else 0)
        value = current.replace(hour=hour, minute=int(minute or 0), second=0, microsecond=0)
        if month: value = value.replace(month=time.strptime(month, '%b').tm_mon, day=int(day))
        if value.timestamp() <= now:
            value = value.replace(year=value.year+1) if month else value + datetime.timedelta(days=1)
        return int(value.timestamp())
    except (ValueError, KeyError): return None


def parse_screen(kind, text, now):
    windows = []
    if kind == 'claude_usage':
        for label, bucket in [('Current session','five_hour'), ('Current week (all models)','seven_day')]:
            m = re.search(re.escape(label) + r'\s*\n([^\n]+)\n([^\n]*)', text)
            if not m: continue
            percent = re.search(r'(?<![\d.])(\d+(?:\.\d+)?)%\s+used', m[1])
            if percent and 0 <= float(percent[1]) <= 100:
                windows.append(dict(bucket=bucket, remaining=1-float(percent[1])/100,
                    reset_at=reset_at(m[2], now), model=None, affects_routing=True))
    elif kind == 'antigravity_usage':
        # Accept explicit machine model IDs plus an explicit direction only. Friendly
        # model names, bare percentages and context-window statistics are ambiguous.
        if re.search(r'Model Quotas|Model Usage|Quota & Credits', text):
            for line in text.splitlines():
                m = re.search(r'\b((?:gemini|claude|gpt)-[\w.\[\]-]+)\b.*?(\d+(?:\.\d+)?)%\s+(remaining|left|used)\b', line)
                if m and 0 <= float(m[2]) <= 100:
                    remaining = float(m[2])/100
                    if m[3] == 'used': remaining = 1-remaining
                    windows.append(dict(bucket=m[1], remaining=remaining, reset_at=None,
                        model=m[1], affects_routing=True))
    # Repeated rows are not valid distinct windows.
    return list({(w['bucket'],w['model']):w for w in windows}.values())


def probe(kind, executable, args, timeout):
    if kind == 'antigravity_usage':
        # Unlike interactive startup, this command reports missing auth without
        # initiating a browser login. A read-only refresh must not start OAuth.
        ready = subprocess.run([executable] + args + ['models'], stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=min(5,timeout))
        if ready.returncode:
            return []
    watch_read, watch_write = os.pipe()
    pid, fd = pty.fork()
    if pid == 0:
        os.close(watch_write)
        # pty.fork creates a new process group. A small watchdog in that group
        # kills it even if Rust cancels and SIGKILLs this parent before finally.
        session_pid = os.getpid()
        watcher = os.fork()
        if watcher == 0:
            while os.read(watch_read, 1):
                pass
            try: os.killpg(os.getpgrp(), signal.SIGKILL)
            except PermissionError: os.kill(session_pid, signal.SIGKILL)
            os._exit(0)
        os.close(watch_read)
        env = dict(os.environ, TERM='xterm-256color', NO_COLOR='1')
        os.execvpe(executable, [executable] + args, env)
    os.close(watch_read)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 80,160,0,0))
    screen = Screen()
    decoder = codecs.getincrementaldecoder('utf-8')('replace')
    start = time.monotonic()
    sent = None
    trust_answered = False
    trust_key_at = 0.0
    trust_attempts = 0
    result = []
    received = 0
    try:
        while time.monotonic()-start < timeout:
            if select.select([fd],[],[],0.1)[0]:
                try: chunk = os.read(fd,65536)
                except OSError: break
                if not chunk: break
                received += len(chunk)
                if received > 1048576: raise ValueError('quota output exceeds 1 MiB')
                screen.feed(decoder.decode(chunk))
            text = screen.text()
            # Only the empty, temporary probe workspace can reach this branch.
            if kind == 'claude_usage' and 'Yes, I trust this folder' in text and 'No, exit' in text:
                elapsed = time.monotonic()
                if elapsed-start > 2 and elapsed-trust_key_at > 1 and not trust_answered:
                    if re.search(r'❯\s+Yes, I trust this folder', text):
                        os.write(fd,b'\r'); trust_answered=True
                    elif trust_attempts < 2:
                        os.write(fd,b'\x1b[B'); trust_attempts += 1
                    trust_key_at = elapsed
                continue
            if sent is None and time.monotonic()-start > 2 and 'Yes, I trust this folder' not in text and ('❯' in text or '> ' in text):
                os.write(fd,b'/usage\r'); sent=time.monotonic()
            if sent is not None:
                result = parse_screen(kind, text, int(time.time()))
                if result and time.monotonic()-sent > 4 and 'Refreshing' not in text:
                    return result
        return []
    finally:
        os.close(watch_write)
        # Closing our watchdog pipe terminates the PTY group without depending on
        # the parent's permission to signal a separate terminal session.
        os.waitpid(pid,0)
        os.close(fd)



if __name__ == '__main__':
    kind, timeout, executable, *args = sys.argv[1:]
    try:
        windows = probe(kind, executable, args, float(timeout))
        print(json.dumps({'schema_version':1,'windows':windows}))
    except Exception:
        # CLI screens may contain account information; never echo them in failures.
        print('Native quota probe failed; check CLI login and supported panel layout.', file=sys.stderr)
        raise SystemExit(1)
