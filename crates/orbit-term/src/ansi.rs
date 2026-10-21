//! The output box's text: what a command writes, with its colours. The
//! escape sequences a terminal would act on are read here — SGR colours and
//! styles (16, 256 and true colour), carriage return (a progress bar
//! redrawing its line), backspace, erasing the line — and the rest (cursor
//! moves, titles, links) are dropped: the box is a record of the output,
//! not a screen.

use std::collections::HashMap;

use gtk::prelude::*;

/// The 16 basic colours, for the dark surface of the box.
pub const PALETTE: [&str; 16] = [
    "#3b4252", "#e06c75", "#98c379", "#e5c07b", "#61afef", "#c678dd", "#56b6c2", "#d8dee9",
    "#6b7280", "#ff8a94", "#b5e890", "#ffd787", "#8cc4ff", "#dc9bf0", "#7fd4de", "#ffffff",
];
/// Errors (stderr) without a colour of their own.
const STDERR: &str = "#e06c75";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
struct Style {
    fg: Option<Colour>,
    bg: Option<Colour>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Colour {
    Index(u8),
    Rgb(u8, u8, u8),
}

impl Colour {
    fn css(self) -> String {
        match self {
            Colour::Index(i) if i < 16 => PALETTE[i as usize].to_string(),
            Colour::Index(i) if i < 232 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                format!("#{:02x}{:02x}{:02x}", level(i / 36), level(i / 6 % 6), level(i % 6))
            }
            Colour::Index(i) => {
                let v = 8 + (i - 232) * 10;
                format!("#{v:02x}{v:02x}{v:02x}")
            }
            Colour::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

#[derive(Default)]
enum State {
    #[default]
    Text,
    Escape,
    Csi(String),
    /// Operating system commands run to BEL or ESC \.
    Osc(bool),
    /// ESC followed by an intermediate byte: one more byte ends it.
    Charset,
}

/// One stream's reading of the escape sequences (stdout and stderr each
/// have one, so a sequence cut between chunks of one never mixes with the
/// other).
#[derive(Default)]
pub struct Reader {
    state: State,
    style: Style,
    /// The end of an incomplete UTF-8 character.
    tail: Vec<u8>,
    /// A carriage return was read: the next text replaces the line.
    cr: bool,
}

/// Writes streams into a text buffer.
pub struct Writer {
    buffer: gtk::TextBuffer,
    tags: HashMap<(Style, bool), gtk::TextTag>,
}

impl Writer {
    pub fn new(buffer: &gtk::TextBuffer) -> Self {
        Self { buffer: buffer.clone(), tags: HashMap::new() }
    }

    pub fn clear(&mut self) {
        self.buffer.set_text("");
    }

    /// Adds `bytes` of a stream read by `reader`; `error` marks stderr.
    pub fn write(&mut self, reader: &mut Reader, bytes: &[u8], error: bool) {
        let mut data = std::mem::take(&mut reader.tail);
        data.extend_from_slice(bytes);
        let text = match std::str::from_utf8(&data) {
            Ok(text) => text.to_string(),
            Err(e) if e.error_len().is_none() => {
                // A character cut in two: its start waits for the rest.
                let (whole, cut) = data.split_at(e.valid_up_to());
                reader.tail = cut.to_vec();
                String::from_utf8_lossy(whole).into_owned()
            }
            Err(_) => String::from_utf8_lossy(&data).into_owned(),
        };
        let mut run = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match &mut reader.state {
                State::Text => match c {
                    '\x1b' => {
                        self.flush(reader, &mut run, error);
                        reader.state = State::Escape;
                    }
                    '\r' if chars.peek() == Some(&'\n') => {}
                    '\r' => {
                        self.flush(reader, &mut run, error);
                        reader.cr = true;
                    }
                    '\x08' => {
                        if run.pop().is_none() {
                            self.flush(reader, &mut run, error);
                            self.erase_back();
                        }
                    }
                    '\n' | '\t' => run.push(c),
                    c if c.is_control() => {}
                    c => run.push(c),
                },
                State::Escape => {
                    reader.state = match c {
                        '[' => State::Csi(String::new()),
                        ']' => State::Osc(false),
                        '(' | ')' | '*' | '+' | '#' | '%' => State::Charset,
                        _ => State::Text,
                    }
                }
                State::Charset => reader.state = State::Text,
                State::Osc(esc) => {
                    if c == '\x07' || (*esc && c == '\\') {
                        reader.state = State::Text;
                    } else {
                        *esc = c == '\x1b';
                    }
                }
                State::Csi(params) => {
                    if ('\x40'..='\x7e').contains(&c) {
                        let params = std::mem::take(params);
                        reader.state = State::Text;
                        match c {
                            'm' => sgr(&mut reader.style, &params),
                            'K' => self.erase_line(&params),
                            _ => {}
                        }
                    } else {
                        params.push(c);
                    }
                }
            }
        }
        self.flush(reader, &mut run, error);
    }

    /// Writes `run` in the current style.
    fn flush(&mut self, reader: &mut Reader, run: &mut String, error: bool) {
        if run.is_empty() {
            return;
        }
        if reader.cr {
            reader.cr = false;
            // The text after "\r" takes the place of the line's.
            let end = self.buffer.end_iter();
            let mut start = end;
            start.set_line_offset(0);
            let mut end = end;
            self.buffer.delete(&mut start, &mut end);
        }
        let tag = self.tag(reader.style, error);
        let mut end = self.buffer.end_iter();
        self.buffer.insert_with_tags(&mut end, run, &[&tag]);
        run.clear();
    }

    fn erase_back(&mut self) {
        let mut end = self.buffer.end_iter();
        if end.line_offset() == 0 {
            return;
        }
        let mut start = end;
        start.backward_char();
        self.buffer.delete(&mut start, &mut end);
    }

    /// CSI K: the line's end (0), start (1) or all of it (2); the box only
    /// writes at the end, so "to the end" has nothing to take.
    fn erase_line(&mut self, params: &str) {
        if matches!(params, "1" | "2") {
            let mut end = self.buffer.end_iter();
            let mut start = end;
            start.set_line_offset(0);
            self.buffer.delete(&mut start, &mut end);
        }
    }

    fn tag(&mut self, style: Style, error: bool) -> gtk::TextTag {
        if let Some(tag) = self.tags.get(&(style, error)) {
            return tag.clone();
        }
        let (mut fg, mut bg) = (style.fg.map(Colour::css), style.bg.map(Colour::css));
        if fg.is_none() && error {
            fg = Some(STDERR.into());
        }
        if fg.is_none() && style.dim {
            fg = Some("#8a909c".into());
        }
        if style.reverse {
            (fg, bg) = (bg.or_else(|| Some("#1b1f27".into())), fg.or_else(|| Some(PALETTE[7].into())));
        }
        let tag = gtk::TextTag::new(None);
        if let Some(fg) = fg {
            tag.set_foreground(Some(&fg));
        }
        if let Some(bg) = bg {
            tag.set_background(Some(&bg));
        }
        if style.bold {
            tag.set_weight(700);
        }
        if style.italic {
            tag.set_style(gtk::pango::Style::Italic);
        }
        if style.underline {
            tag.set_underline(gtk::pango::Underline::Single);
        }
        self.buffer.tag_table().add(&tag);
        self.tags.insert((style, error), tag.clone());
        tag
    }
}

/// Applies an SGR sequence's parameters (`1;38;5;208`) to `style`.
fn sgr(style: &mut Style, params: &str) {
    let numbers: Vec<u16> = if params.is_empty() {
        vec![0]
    } else {
        params.split([';', ':']).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let mut i = 0;
    while i < numbers.len() {
        let n = numbers[i];
        match n {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            7 => style.reverse = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            27 => style.reverse = false,
            30..=37 => style.fg = Some(Colour::Index((n - 30) as u8)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(Colour::Index((n - 40) as u8)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(Colour::Index((n - 90 + 8) as u8)),
            100..=107 => style.bg = Some(Colour::Index((n - 100 + 8) as u8)),
            38 | 48 => {
                let colour = match numbers.get(i + 1) {
                    Some(5) => {
                        i += 2;
                        numbers.get(i).map(|&c| Colour::Index(c.min(255) as u8))
                    }
                    Some(2) => {
                        i += 4;
                        let c = |k: usize| numbers.get(i - 3 + k).copied().unwrap_or(0).min(255) as u8;
                        Some(Colour::Rgb(c(1), c(2), c(3)))
                    }
                    _ => None,
                };
                if n == 38 {
                    style.fg = colour;
                } else {
                    style.bg = colour;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_and_styles() {
        let mut s = Style::default();
        sgr(&mut s, "1;31");
        assert!(s.bold && s.fg == Some(Colour::Index(1)));
        sgr(&mut s, "38;5;208;48;2;10;20;30");
        assert_eq!((s.fg, s.bg), (Some(Colour::Index(208)), Some(Colour::Rgb(10, 20, 30))));
        sgr(&mut s, "");
        assert!(s == Style::default());
        assert_eq!(Colour::Index(196).css(), "#ff0000");
        assert_eq!(Colour::Index(244).css(), "#808080");
    }

    #[test]
    fn the_buffer_gets_the_text() {
        gtk::init().ok();
        if !gtk::is_initialized() {
            return;
        }
        let buffer = gtk::TextBuffer::new(None);
        let mut w = Writer::new(&buffer);
        let mut r = Reader::default();
        w.write(&mut r, b"a\x1b[31mb\x1b[0m\n10%\r50%\r100%\n\xc3", false);
        w.write(&mut r, b"\xa9 ok", false);
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        assert_eq!(text, "ab\n100%\né ok");
    }
}
