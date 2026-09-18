"""Minimal ANSI screen with scrollback, enough to see what a chat session draws."""
import re, unicodedata

def width(ch):
    if unicodedata.combining(ch):
        return 0
    return 2 if unicodedata.east_asian_width(ch) in "WF" else 1

class Screen:
    def __init__(self, rows=24, cols=80):
        self.rows, self.cols = rows, cols
        self.buf = [[" "] * cols for _ in range(rows)]
        self.scrollback = []
        self.top, self.bot = 1, rows
        self.r = self.c = 1
        self.saved = (1, 1)

    def line(self, i):
        return "".join(self.buf[i]).rstrip()

    def text(self):
        return self.scrollback + [self.line(i) for i in range(self.rows)]

    def scroll_up(self):
        first = self.buf.pop(self.top - 1)
        self.scrollback.append("".join(first).rstrip())
        self.buf.insert(self.bot - 1, [" "] * self.cols)

    def put(self, ch):
        w = width(ch)
        if w == 0:
            return
        if self.c > self.cols:
            self.c = 1
            self.newline()
        self.buf[self.r - 1][self.c - 1] = ch
        for k in range(1, w):
            if self.c - 1 + k < self.cols:
                self.buf[self.r - 1][self.c - 1 + k] = ""
        self.c += w

    def newline(self):
        if self.r == self.bot:
            self.scroll_up()
        elif self.r < self.rows:
            self.r += 1

    def feed(self, data):
        i = 0
        while i < len(data):
            ch = data[i]
            if ch == "\x1b":
                m = re.match(r"\x1b\[([0-9;?]*)([@-~])", data[i:])
                if m:
                    self.csi(m.group(1), m.group(2))
                    i += m.end()
                    continue
                m = re.match(r"\x1b\][^\x07\x1b]*(\x07|\x1b\\)", data[i:])
                if m:
                    i += m.end()
                    continue
                m = re.match(r"\x1b[()][A-Za-z0-9]", data[i:])
                if m:
                    i += m.end()
                    continue
                if data[i:i + 2] in ("\x1b7", "\x1b8"):
                    if data[i + 1] == "7":
                        self.saved = (self.r, self.c)
                    else:
                        self.r, self.c = self.saved
                    i += 2
                    continue
                i += 1
                continue
            if ch == "\n":
                self.newline()
            elif ch == "\r":
                self.c = 1
            elif ch == "\b":
                self.c = max(1, self.c - 1)
            elif ch == "\t":
                self.c = min(self.cols, self.c + 8 - (self.c - 1) % 8)
            elif ch >= " ":
                self.put(ch)
            i += 1

    def csi(self, params, final):
        args = [int(p) for p in params.split(";") if p.isdigit()]
        n = args[0] if args else 0
        if final == "H" or final == "f":
            self.r = min(max(1, args[0] if args else 1), self.rows)
            self.c = min(max(1, args[1] if len(args) > 1 else 1), self.cols)
        elif final == "A":
            self.r = max(1, self.r - max(1, n))
        elif final == "B":
            self.r = min(self.rows, self.r + max(1, n))
        elif final == "C":
            self.c = min(self.cols, self.c + max(1, n))
        elif final == "D":
            self.c = max(1, self.c - max(1, n))
        elif final == "G":
            self.c = min(max(1, n or 1), self.cols)
        elif final == "J":
            if n == 2:
                self.buf = [[" "] * self.cols for _ in range(self.rows)]
            elif n == 0:
                self.buf[self.r - 1][self.c - 1:] = [" "] * (self.cols - self.c + 1)
                for row in range(self.r, self.rows):
                    self.buf[row] = [" "] * self.cols
        elif final == "K":
            if n == 0:
                self.buf[self.r - 1][self.c - 1:] = [" "] * (self.cols - self.c + 1)
            elif n == 1:
                self.buf[self.r - 1][:self.c] = [" "] * self.c
            else:
                self.buf[self.r - 1] = [" "] * self.cols
        elif final == "r":
            self.top = args[0] if args else 1
            self.bot = args[1] if len(args) > 1 else self.rows
            self.r, self.c = 1, 1
        elif final == "s":
            self.saved = (self.r, self.c)
        elif final == "u":
            self.r, self.c = self.saved
        elif final == "S":
            for _ in range(max(1, n)):
                self.scroll_up()
