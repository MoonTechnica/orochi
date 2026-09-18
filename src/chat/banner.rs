//! The eight-headed serpent approved for the interactive welcome screen.
//! Artwork uses single-column Unicode braille cells; braces mark red eye cells.

const LARGE: &str = include_str!("banner/large.txt");
const MEDIUM: &str = include_str!("banner/medium.txt");
const SMALL: &str = include_str!("banner/small.txt");
const WORDMARK: &str = "O R O C H I";
const BODY: &str = "\x1b[38;2;194;180;205m";
const EYES: &str = "\x1b[38;2;255;70;110m";
const RESET: &str = "\x1b[0m";

pub(super) fn render(columns: usize, max_lines: usize, color: bool) -> String {
    if columns < WORDMARK.len() + 4 || max_lines < 2 {
        return String::new();
    }
    let frame = columns.min(80);
    let art = [(LARGE, 64), (MEDIUM, 48), (SMALL, 34)]
        .into_iter()
        .find(|(art, width)| *width + 4 <= columns && art.lines().count() + 2 <= max_lines);
    let mut out = String::new();
    if let Some((art, width)) = art {
        let indent = " ".repeat((frame - width) / 2);
        for line in art.lines() {
            out.push_str(&indent);
            if color {
                out.push_str(BODY);
            }
            for c in line.chars() {
                match c {
                    '{' if color => out.push_str(EYES),
                    '}' if color => out.push_str(BODY),
                    '{' | '}' => {}
                    _ => out.push(c),
                }
            }
            if color {
                out.push_str(RESET);
            }
            out.push('\n');
        }
    }
    out.push_str(&" ".repeat((frame - WORDMARK.len()) / 2));
    if color {
        out.push_str("\x1b[1m");
    }
    out.push_str(WORDMARK);
    if color {
        out.push_str(RESET);
    }
    out.push_str("\n\n");
    out
}
