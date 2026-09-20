//! The terminal for the interactive chat: raw-mode key decoding, a transcript area that keeps
//! the shell's scrollback, and an input line with a status row pinned to the bottom.
use std::io::Write;
use tokio::sync::mpsc;
use unicode_width::UnicodeWidthChar;

#[derive(Debug, PartialEq, Clone)]
pub enum Key {
    Char(char),
    Paste(String),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
    ShiftTab,
    Escape,
    Interrupt,
    /// Ctrl-D typed at a terminal: deletes forward, or asks to leave on an empty line.
    CtrlD,
    /// A turn a client queued, read from the store instead of a keyboard. It carries the row
    /// it came from, so the turn is claimed rather than written again.
    Queued {
        turn: String,
        text: String,
    },
    /// Input ended (a closed pipe or terminal).
    Eof,
    KillLine,
    KillWord,
    Clear,
}

/// Reads stdin on a thread and decodes keys, so the async loop never blocks on input.
pub struct Keyboard {
    keys: mpsc::UnboundedReceiver<Key>,
}
impl Keyboard {
    pub fn start(raw: bool) -> Self {
        let (sender, keys) = mpsc::unbounded_channel();
        // Detached: a blocked read must not delay process exit.
        std::thread::Builder::new()
            .name("orochi-input".into())
            .spawn(move || {
                if raw {
                    read_keys(&sender);
                } else {
                    read_lines(&sender);
                }
            })
            .ok();
        Self { keys }
    }
    pub async fn next(&mut self) -> Option<Key> {
        self.keys.recv().await
    }
    /// A keyboard someone else types at: `orochi host` feeds it from the store. Dropping the
    /// sender ends the session, which is how a host leaves when it has been idle long enough.
    pub fn channel() -> (mpsc::UnboundedSender<Key>, Self) {
        let (sender, keys) = mpsc::unbounded_channel();
        (sender, Self { keys })
    }
}

/// Without a terminal each stdin line is one message.
fn read_lines(keys: &mpsc::UnboundedSender<Key>) {
    loop {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => {
                let _ = keys.send(Key::Eof);
                return;
            }
            Ok(_) => {
                let line = line.trim_end_matches(['\n', '\r']).to_owned();
                if keys.send(Key::Paste(line)).is_err() || keys.send(Key::Enter).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(unix)]
fn byte() -> Option<u8> {
    let mut byte = 0u8;
    (unsafe { libc::read(libc::STDIN_FILENO, (&raw mut byte).cast(), 1) } == 1).then_some(byte)
}
#[cfg(unix)]
fn readable(timeout_ms: i32) -> bool {
    let mut fd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut fd, 1, timeout_ms) > 0 }
}
#[cfg(not(unix))]
fn byte() -> Option<u8> {
    None
}
#[cfg(not(unix))]
fn readable(_: i32) -> bool {
    false
}

fn read_keys(keys: &mpsc::UnboundedSender<Key>) {
    let mut pending = Vec::new();
    loop {
        let Some(first) = byte() else {
            let _ = keys.send(Key::Eof);
            return;
        };
        let key = match first {
            0x1b => match escape() {
                Some(key) => key,
                None => continue,
            },
            b'\r' | b'\n' => Key::Enter,
            0x7f | 0x08 => Key::Backspace,
            b'\t' => Key::Tab,
            0x01 => Key::Home,
            0x03 => Key::Interrupt,
            0x04 => Key::CtrlD,
            0x05 => Key::End,
            0x0b => Key::KillLine,
            0x0c => Key::Clear,
            0x15 => Key::KillWord,
            0x17 => Key::KillWord,
            0x00..0x20 => continue,
            byte => {
                // Collect a UTF-8 sequence before reporting a character.
                pending.push(byte);
                match std::str::from_utf8(&pending) {
                    Ok(text) => {
                        let key = text.chars().next().map(Key::Char);
                        pending.clear();
                        match key {
                            Some(key) => key,
                            None => continue,
                        }
                    }
                    Err(_) if pending.len() < 4 => continue,
                    Err(_) => {
                        pending.clear();
                        continue;
                    }
                }
            }
        };
        if keys.send(key).is_err() {
            return;
        }
    }
}

/// Decodes what follows an Esc byte: a lone Esc, an arrow, Shift-Tab or a bracketed paste.
fn escape() -> Option<Key> {
    if !readable(40) {
        return Some(Key::Escape);
    }
    let mut parameters = Vec::new();
    let final_byte = match byte()? {
        b'[' => loop {
            let byte = byte()?;
            if (0x40..=0x7e).contains(&byte) {
                break byte;
            }
            parameters.push(byte);
        },
        b'O' => byte()?,
        _ => return None,
    };
    let parameters = String::from_utf8_lossy(&parameters).into_owned();
    Some(match (final_byte, parameters.as_str()) {
        (b'A', _) => Key::Up,
        (b'B', _) => Key::Down,
        (b'C', _) => Key::Right,
        (b'D', _) => Key::Left,
        (b'H', _) | (b'~', "1" | "7") => Key::Home,
        (b'F', _) | (b'~', "4" | "8") => Key::End,
        (b'~', "3") => Key::Delete,
        (b'Z', _) => Key::ShiftTab,
        (b'~', "200") => Key::Paste(paste()),
        _ => return None,
    })
}

/// Everything up to the bracketed-paste end marker, so pasted newlines stay in the message.
fn paste() -> String {
    let mut text = Vec::new();
    while let Some(byte) = byte() {
        text.push(byte);
        if text.ends_with(b"\x1b[201~") {
            text.truncate(text.len() - 6);
            break;
        }
        if text.len() > 1_000_000 {
            break;
        }
    }
    String::from_utf8_lossy(&text).replace('\r', "\n")
}

pub fn width_of(text: &str) -> usize {
    let mut width = 0;
    let mut escape = false;
    for c in text.chars() {
        match (escape, c) {
            (false, '\u{1b}') => escape = true,
            (true, 'm') => escape = false,
            (true, _) => {}
            (false, c) => width += c.width().unwrap_or(0),
        }
    }
    width
}

pub fn flush() {
    let _ = std::io::stdout().flush();
}

use crate::types::Attachment;

/// The visible terminal: a transcript area that scrolls (and reaches the shell's scrollback)
/// above the input line, its attachments and a status row, all pinned to the bottom.
pub struct Term {
    pub tty: bool,
    pub color: bool,
    pub prompt: Prompt,
    pub status: String,
    /// Rows that take over the input area while a question is open.
    pub overlay: Option<Vec<String>>,
    /// Command candidates for what is being typed, drawn above the input line.
    pub suggestions: Vec<String>,
    /// Which of `suggestions` Tab and Enter would take.
    pub selected: usize,
    columns: usize,
    rows: usize,
    pinned: usize,
    /// Where the next transcript character goes (1-based row and column).
    row: usize,
    column: usize,
    #[cfg(unix)]
    saved: Option<libc::termios>,
}

const MAX_INPUT_ROWS: usize = 8;

impl Term {
    pub fn start(tty: bool, color: bool, marker: String) -> Self {
        let mut term = Self {
            tty,
            color,
            prompt: Prompt::new(marker),
            status: String::new(),
            overlay: None,
            suggestions: vec![],
            selected: 0,
            columns: 80,
            rows: 24,
            pinned: 2,
            row: 1,
            column: 1,
            #[cfg(unix)]
            saved: None,
        };
        if !tty {
            return term;
        }
        term.measure();
        #[cfg(unix)]
        {
            let mut saved: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } == 0 {
                let mut raw = saved;
                // Keep output processing (OPOST) so "\n" still returns to column 1.
                raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
                raw.c_iflag &= !(libc::IXON | libc::ICRNL);
                raw.c_cc[libc::VMIN] = 1;
                raw.c_cc[libc::VTIME] = 0;
                if unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) } == 0 {
                    term.saved = Some(saved);
                }
            }
        }
        // Bracketed paste, a cleared screen, and a scroll region above the pinned rows.
        print!("\x1b[?2004h\x1b[2J\x1b[H");
        term.set_region();
        flush();
        term
    }
    fn measure(&mut self) {
        #[cfg(unix)]
        {
            let mut size: libc::winsize = unsafe { std::mem::zeroed() };
            if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
                && size.ws_col > 0
                && size.ws_row > 4
            {
                self.columns = size.ws_col.into();
                self.rows = size.ws_row.into();
            }
        }
    }
    fn bottom(&self) -> usize {
        self.rows.saturating_sub(self.pinned).max(1)
    }
    fn set_region(&mut self) {
        print!("\x1b[1;{}r", self.bottom());
        self.row = self.row.min(self.bottom());
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    /// Re-reads the size after SIGWINCH and redraws the pinned rows.
    pub fn resized(&mut self) {
        if !self.tty {
            return;
        }
        self.measure();
        self.set_region();
        self.render();
    }

    /// Adds transcript text above the pinned rows and leaves the cursor in the input.
    pub fn write(&mut self, text: &str) {
        if !self.tty {
            print!("{text}");
            flush();
            return;
        }
        print!("\x1b[{};{}H{text}", self.row, self.column);
        self.advance(text);
        self.render();
    }
    /// Transcript text that goes to stderr when output is redirected (everything but replies).
    pub fn note(&mut self, text: &str) {
        if self.tty {
            self.write(text);
        } else {
            eprint!("{text}");
        }
    }
    /// Moves the write position back over rows that are about to be drawn again.
    pub fn step_back(&mut self, rows: usize) {
        if !self.tty {
            return;
        }
        self.row = self.row.saturating_sub(rows).max(1);
        self.column = 1;
        let mut out = String::new();
        for offset in 0..rows {
            out.push_str(&format!("\x1b[{};1H\x1b[K", self.row + offset));
        }
        print!("{out}");
    }
    /// Clears the current transcript row (a streamed line about to be redrawn styled).
    pub fn rewind(&mut self) {
        if self.tty {
            self.column = 1;
            print!("\x1b[{};1H\x1b[K", self.row);
        }
    }
    fn advance(&mut self, text: &str) {
        let (row, column) =
            cursor_after((self.row, self.column), self.columns, self.bottom(), text);
        self.row = row;
        self.column = column;
    }

    /// Resizes the pinned area to `pinned` rows and returns (first pinned row, first row to
    /// clear). A shrinking input frees rows that still hold its leftovers: the transcript flows
    /// there next, so they are cleared before anything is drawn.
    fn repin(&mut self, pinned: usize) -> (usize, usize) {
        let previous = std::mem::replace(&mut self.pinned, pinned);
        if previous != pinned {
            // A growing input eats into the transcript: scroll it up so the last lines stay
            // visible and the next one flows on, rather than being written under the input.
            // It scrolls inside the old region, which still holds those lines; the new, shorter
            // one would leave the rows about to be covered where they are, to be drawn over.
            let bottom = self.bottom();
            if self.row > bottom {
                let old = self.rows.saturating_sub(previous).max(1);
                print!("\x1b[{old};1H{}", "\n".repeat(self.row - bottom));
                self.row = bottom;
            }
            self.set_region();
        }
        (
            self.rows.saturating_sub(pinned) + 1,
            self.rows.saturating_sub(previous.max(pinned)) + 1,
        )
    }

    pub fn render(&mut self) {
        if !self.tty {
            return;
        }
        // A question takes over the input area, exactly where the answer is given.
        if let Some(overlay) = self.overlay.clone() {
            let rows: Vec<String> = overlay.into_iter().take(self.rows / 2).collect();
            let (first, freed) = self.repin(rows.len() + 1);
            let mut out = String::from("\x1b[?25l");
            for row in freed..first {
                out.push_str(&format!("\x1b[{row};1H\x1b[K"));
            }
            for (index, row) in rows.iter().enumerate() {
                out.push_str(&format!("\x1b[{};1H\x1b[K{row}", first + index));
            }
            out.push_str(&format!("\x1b[{};1H\x1b[K{}", self.rows, self.status));
            print!("{out}");
            flush();
            return;
        }
        let (mut rows, caret) = self.prompt.layout(self.columns.saturating_sub(1));
        rows.truncate(MAX_INPUT_ROWS);
        let caret = (caret.0.min(rows.len().saturating_sub(1)), caret.1);
        let attachments: Vec<String> = self
            .prompt
            .attachments
            .iter()
            .map(|a| {
                let text = format!("  ⎿ {} {} ({})", a.tag, a.name, size(a.bytes));
                if self.color {
                    format!("\x1b[2m{text}\x1b[0m")
                } else {
                    text
                }
            })
            .collect();
        // Candidates sit above the line being typed, so the caret stays where the text is.
        let suggestions: Vec<String> = self
            .suggestions
            .iter()
            .take(self.rows.saturating_sub(MAX_INPUT_ROWS + 2).max(1))
            .cloned()
            .collect();
        let (first, freed) = self.repin(suggestions.len() + rows.len() + attachments.len() + 1);
        let mut out = String::new();
        for row in freed..first {
            out.push_str(&format!("\x1b[{row};1H\x1b[K"));
        }
        for (index, row) in suggestions
            .iter()
            .chain(&rows)
            .chain(&attachments)
            .enumerate()
        {
            out.push_str(&format!("\x1b[{};1H\x1b[K{row}", first + index));
        }
        out.push_str(&format!("\x1b[{};1H\x1b[K{}", self.rows, self.status));
        out.push_str(&format!(
            "\x1b[{};{}H\x1b[?25h",
            first + suggestions.len() + caret.0,
            caret.1 + 1
        ));
        print!("{out}");
        flush();
    }

    /// Restores the terminal: full scroll region, normal input, cursor below the transcript.
    pub fn restore(&mut self) {
        if !self.tty {
            return;
        }
        let first = self.rows.saturating_sub(self.pinned) + 1;
        let mut out = String::from("\x1b[r\x1b[?2004l\x1b[?25h");
        for row in first..=self.rows {
            out.push_str(&format!("\x1b[{row};1H\x1b[K"));
        }
        out.push_str(&format!("\x1b[{};1H\n", self.row));
        print!("{out}");
        flush();
        #[cfg(unix)]
        if let Some(saved) = self.saved.take() {
            unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &saved) };
        }
    }
}
impl Drop for Term {
    fn drop(&mut self) {
        self.restore();
    }
}

pub fn size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// The message being typed: text, cursor, attachments and this session's history.
#[derive(Default)]
pub struct Prompt {
    pub buffer: String,
    pub cursor: usize,
    pub marker: String,
    pub attachments: Vec<Attachment>,
    history: Vec<String>,
    index: Option<usize>,
    draft: String,
}
impl Prompt {
    pub fn new(marker: String) -> Self {
        Self {
            marker,
            ..Default::default()
        }
    }
    pub fn attach(&mut self, attachment: Attachment) {
        self.insert(&attachment.tag.clone());
        self.attachments.push(attachment);
    }
    pub fn insert(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }
    pub fn backspace(&mut self) {
        if let Some((index, _)) = self.buffer[..self.cursor].char_indices().next_back() {
            self.buffer.remove(index);
            self.cursor = index;
        }
    }
    pub fn delete(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.remove(self.cursor);
        }
    }
    pub fn left(&mut self) {
        if let Some((index, _)) = self.buffer[..self.cursor].char_indices().next_back() {
            self.cursor = index;
        }
    }
    pub fn right(&mut self) {
        if let Some(c) = self.buffer[self.cursor..].chars().next() {
            self.cursor += c.len_utf8();
        }
    }
    pub fn home(&mut self) {
        self.cursor = 0;
    }
    pub fn end(&mut self) {
        self.cursor = self.buffer.len();
    }
    pub fn kill_line(&mut self) {
        self.buffer.truncate(self.cursor);
    }
    pub fn kill_word(&mut self) {
        let head = self.buffer[..self.cursor].trim_end_matches(char::is_whitespace);
        let start = head
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map_or(0, |(index, c)| index + c.len_utf8());
        self.buffer.replace_range(start..self.cursor, "");
        self.cursor = start;
    }
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.attachments.clear();
        self.cursor = 0;
        self.index = None;
    }
    pub fn text(&self) -> &str {
        &self.buffer
    }
    /// Clears a draft without sending it, keeping it in history so Up brings it back.
    pub fn shelve(&mut self) {
        if !self.buffer.trim().is_empty() && self.history.last() != Some(&self.buffer) {
            self.history.push(self.buffer.clone());
        }
        self.clear();
    }
    /// Puts messages taken back from the queue ahead of whatever is being typed.
    pub fn restore(&mut self, text: &str, attachments: Vec<Attachment>) {
        let typed = std::mem::take(&mut self.buffer);
        self.buffer = if typed.is_empty() {
            text.to_owned()
        } else {
            format!("{text}\n{typed}")
        };
        self.cursor = self.buffer.len();
        self.index = None;
        let mut restored = attachments;
        restored.append(&mut self.attachments);
        self.attachments = restored;
    }
    pub fn take(&mut self) -> (String, Vec<Attachment>) {
        let text = std::mem::take(&mut self.buffer);
        let attachments = std::mem::take(&mut self.attachments);
        self.cursor = 0;
        self.index = None;
        if !text.trim().is_empty() && self.history.last() != Some(&text) {
            self.history.push(text.clone());
        }
        (text, attachments)
    }
    pub fn recall(&mut self, back: bool) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.index, back) {
            (None, true) => {
                self.draft = self.buffer.clone();
                Some(self.history.len() - 1)
            }
            (Some(0), true) => Some(0),
            (Some(index), true) => Some(index - 1),
            (Some(index), false) if index + 1 < self.history.len() => Some(index + 1),
            (Some(_), false) => None,
            (None, false) => return,
        };
        self.index = next;
        self.buffer = match next {
            Some(index) => self.history[index].clone(),
            None => std::mem::take(&mut self.draft),
        };
        self.cursor = self.buffer.len();
    }

    /// The input rows as drawn, plus the caret's (row, column).
    pub fn layout(&self, columns: usize) -> (Vec<String>, (usize, usize)) {
        let columns = columns.max(8);
        let mut rows = vec![String::new()];
        let mut widths = vec![width_of(&self.marker)];
        rows[0].push_str(&self.marker);
        let mut caret = (0, widths[0]);
        for (index, c) in self.buffer.char_indices() {
            if index == self.cursor {
                caret = (rows.len() - 1, *widths.last().expect("row width"));
            }
            let width = c.width().unwrap_or(0);
            if c == '\n' || widths.last().expect("row width") + width > columns {
                rows.push("  ".into());
                widths.push(2);
            }
            if c != '\n' {
                rows.last_mut().expect("row").push(c);
                *widths.last_mut().expect("row width") += width;
            }
        }
        if self.cursor >= self.buffer.len() {
            caret = (rows.len() - 1, *widths.last().expect("row width"));
        }
        (rows, caret)
    }
}

/// Where the cursor lands after printing `text` from `start`. Colour escapes move nothing, so
/// they must not count towards the column: a half-written row would otherwise be redrawn over
/// itself, one gap per escape sequence.
pub fn cursor_after(
    start: (usize, usize),
    columns: usize,
    bottom: usize,
    text: &str,
) -> (usize, usize) {
    let (mut row, mut column) = start;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                // CSI and OSC sequences print nothing; skip to the end of the sequence.
                let osc = chars.peek() == Some(&']');
                if matches!(chars.peek(), Some('[' | ']')) {
                    chars.next();
                }
                for c in chars.by_ref() {
                    if (osc && matches!(c, '\u{7}' | '\u{1b}')) || (!osc && c.is_ascii_alphabetic())
                    {
                        break;
                    }
                }
            }
            '\n' => {
                column = 1;
                row = (row + 1).min(bottom);
            }
            '\r' => column = 1,
            _ => {
                column += c.width().unwrap_or(0);
                if column > columns {
                    column = 1;
                    row = (row + 1).min(bottom);
                }
            }
        }
    }
    (row, column)
}
