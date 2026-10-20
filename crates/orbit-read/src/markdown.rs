//! Markdown, laid out in a GTK text view: headings, emphasis, code,
//! quotes, lists and task lists, rules, tables (a grid in the text),
//! local images, links and footnotes. Parsed by pulldown-cmark
//! (CommonMark, with GitHub's tables, strikethrough and task lists).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use gtk::{gdk, pango, prelude::*};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Widest an image is shown, in pixels.
const IMAGE_WIDTH: i32 = 700;

/// What the laid-out text links to.
#[derive(Default)]
pub struct Rendered {
    /// Each link's tag, and where it leads.
    pub links: HashMap<gtk::TextTag, String>,
    /// Headings by their anchor (`#instalação`), for links inside the page.
    pub anchors: HashMap<String, gtk::TextMark>,
}

pub fn render(view: &gtk::TextView, source: &str, base: &Path) -> Rendered {
    let buffer = view.buffer();
    buffer.set_text("");
    let mut out = Writer::new(view, base);
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    for event in Parser::new_ext(source, options) {
        out.event(event);
    }
    out.rendered
}

struct Writer<'a> {
    view: &'a gtk::TextView,
    buffer: gtk::TextBuffer,
    base: &'a Path,
    /// Tags in force, innermost last.
    active: Vec<gtk::TextTag>,
    /// The lists open: `Some(n)` numbered from n, `None` bulleted.
    lists: Vec<Option<u64>>,
    in_code: bool,
    /// The heading being written: its start and its text.
    heading: Option<(gtk::TextMark, String)>,
    /// A table being collected: its rows of cells, and whether the first
    /// is the header.
    table: Option<Vec<Vec<String>>>,
    cell: Option<String>,
    /// An image being collected: its address and its alternative text.
    image: Option<(String, String)>,
    rendered: Rendered,
}

impl<'a> Writer<'a> {
    fn new(view: &'a gtk::TextView, base: &'a Path) -> Self {
        let buffer = view.buffer();
        create_tags(&buffer);
        Self {
            view,
            buffer,
            base,
            active: Vec::new(),
            lists: Vec::new(),
            in_code: false,
            heading: None,
            table: None,
            cell: None,
            image: None,
            rendered: Rendered::default(),
        }
    }

    fn tag(&self, name: &str) -> gtk::TextTag {
        self.buffer
            .tag_table()
            .lookup(name)
            .unwrap_or_else(|| panic!("tag {name}"))
    }

    fn push(&mut self, name: &str) {
        let tag = self.tag(name);
        self.active.push(tag);
    }

    fn pop(&mut self) {
        self.active.pop();
    }

    fn write(&mut self, text: &str) {
        if let Some(cell) = &mut self.cell {
            cell.push_str(text);
            return;
        }
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(text);
            return;
        }
        if let Some((_, heading)) = &mut self.heading {
            heading.push_str(text);
        }
        let mut end = self.buffer.end_iter();
        let tags: Vec<&gtk::TextTag> = self.active.iter().collect();
        self.buffer.insert_with_tags(&mut end, text, &tags);
    }

    /// Ends the line, unless it is ended already.
    fn newline(&mut self) {
        let end = self.buffer.end_iter();
        if end.offset() > 0 {
            let mut before = end;
            before.backward_char();
            if before.char() != '\n' {
                self.write_plain("\n");
            }
        }
    }

    /// Text outside every tag but the block ones (lists, quotes).
    fn write_plain(&mut self, text: &str) {
        let mut end = self.buffer.end_iter();
        let tags: Vec<&gtk::TextTag> = self
            .active
            .iter()
            .filter(|t| {
                t.name()
                    .is_some_and(|n| n.starts_with("list") || n == "quote")
            })
            .collect();
        self.buffer.insert_with_tags(&mut end, text, &tags);
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.write(&text),
            Event::Code(code) => {
                self.push("code");
                self.write(&code);
                self.pop();
            }
            Event::SoftBreak => self.write(if self.in_code { "\n" } else { " " }),
            Event::HardBreak => self.write("\n"),
            Event::Rule => {
                self.newline();
                self.push("rule");
                self.write(&"─".repeat(30));
                self.pop();
                self.write_plain("\n");
            }
            Event::TaskListMarker(done) => self.write(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(label) => {
                self.push("note");
                self.write(&format!("[{label}]"));
                self.pop();
            }
            Event::InlineHtml(html) if html.trim().eq_ignore_ascii_case("<br>") || html.trim() == "<br/>" => {
                self.write("\n");
            }
            // Raw HTML is left out: a reader is no browser.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.newline();
                self.push("p");
            }
            Tag::Heading { level, .. } => {
                self.newline();
                let mark = self
                    .buffer
                    .create_mark(None, &self.buffer.end_iter(), true);
                self.heading = Some((mark, String::new()));
                self.push(match level {
                    HeadingLevel::H1 => "h1",
                    HeadingLevel::H2 => "h2",
                    HeadingLevel::H3 => "h3",
                    HeadingLevel::H4 => "h4",
                    HeadingLevel::H5 => "h5",
                    HeadingLevel::H6 => "h6",
                });
            }
            Tag::BlockQuote(_) => {
                self.newline();
                self.push("quote");
            }
            Tag::CodeBlock(kind) => {
                self.newline();
                self.in_code = true;
                self.push("pre");
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty()
                {
                    self.push("lang");
                    self.write(&format!("{lang}\n"));
                    self.pop();
                }
            }
            Tag::List(start) => {
                self.newline();
                self.lists.push(start);
            }
            Tag::Item => {
                self.newline();
                let depth = self.lists.len().clamp(1, 6);
                self.push(&format!("list{depth}"));
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => ["• ", "◦ ", "▪ "][(depth - 1) % 3].to_owned(),
                };
                self.write(&marker);
            }
            Tag::Emphasis => self.push("em"),
            Tag::Strong => self.push("strong"),
            Tag::Strikethrough => self.push("strike"),
            Tag::Link { dest_url, .. } => {
                let tag = gtk::TextTag::builder()
                    .underline(pango::Underline::Single)
                    .foreground_rgba(&orbit_app::theme::accent())
                    .build();
                self.buffer.tag_table().add(&tag);
                self.rendered.links.insert(tag.clone(), dest_url.to_string());
                self.active.push(tag);
            }
            Tag::Image { dest_url, .. } => {
                self.image = Some((dest_url.to_string(), String::new()));
            }
            Tag::Table(_) => {
                self.newline();
                self.table = Some(Vec::new());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.push(Vec::new());
                }
            }
            Tag::TableCell => self.cell = Some(String::new()),
            Tag::FootnoteDefinition(label) => {
                self.newline();
                self.push("note");
                self.write(&format!("[{label}] "));
                self.pop();
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.write("\n");
                self.pop();
            }
            TagEnd::Heading(_) => {
                self.write("\n");
                self.pop();
                if let Some((mark, text)) = self.heading.take() {
                    self.rendered.anchors.insert(slug(&text), mark);
                }
            }
            TagEnd::BlockQuote(_) => {
                self.pop();
                self.newline();
            }
            TagEnd::CodeBlock => {
                self.newline();
                self.pop();
                self.in_code = false;
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.newline();
            }
            TagEnd::Item => {
                self.newline();
                self.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => self.pop(),
            TagEnd::Image => {
                if let Some((url, alt)) = self.image.take() {
                    self.image(&url, &alt);
                }
            }
            TagEnd::TableCell => {
                if let (Some(cell), Some(table)) = (self.cell.take(), &mut self.table)
                    && let Some(row) = table.last_mut()
                {
                    row.push(cell.trim().to_owned());
                }
            }
            TagEnd::Table => {
                if let Some(rows) = self.table.take() {
                    self.table_grid(&rows);
                }
            }
            _ => {}
        }
    }

    /// A local image, shown; anything else, its text as a link.
    fn image(&mut self, url: &str, alt: &str) {
        let path = local_path(self.base, url);
        let texture = path.as_ref().and_then(|p| gdk::Texture::from_filename(p).ok());
        let Some(texture) = texture else {
            let shown = if alt.is_empty() { url } else { alt };
            let tag = gtk::TextTag::builder()
                .underline(pango::Underline::Single)
                .foreground_rgba(&orbit_app::theme::accent())
                .build();
            self.buffer.tag_table().add(&tag);
            self.rendered.links.insert(tag.clone(), url.to_owned());
            self.active.push(tag);
            self.write(&format!("🖼 {shown}"));
            self.pop();
            return;
        };
        let (w, h) = (texture.width().max(1), texture.height().max(1));
        let width = w.min(IMAGE_WIDTH);
        let picture = gtk::Picture::builder()
            .paintable(&texture)
            .can_shrink(true)
            .width_request(width)
            .height_request(h * width / w)
            .alternative_text(alt)
            .tooltip_text(alt)
            .build();
        self.anchor(&picture);
    }

    fn table_grid(&mut self, rows: &[Vec<String>]) {
        let grid = gtk::Grid::builder()
            .column_spacing(0)
            .row_spacing(0)
            .css_classes(["md-table"])
            .build();
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let mut classes = vec!["cell"];
                if r == 0 {
                    classes.push("header");
                }
                let label = gtk::Label::builder()
                    .label(cell.as_str())
                    .xalign(0.0)
                    .wrap(true)
                    // A wrapping label asks for its narrowest; a cell asks
                    // for its text's width, up to a limit.
                    .width_chars(cell.chars().count().min(40) as i32)
                    .max_width_chars(40)
                    .selectable(true)
                    .css_classes(classes)
                    .build();
                grid.attach(&label, c as i32, r as i32, 1, 1);
            }
        }
        self.anchor(&grid);
    }

    /// Puts `widget` in the text, on a line of its own.
    fn anchor(&mut self, widget: &impl IsA<gtk::Widget>) {
        self.newline();
        let mut end = self.buffer.end_iter();
        let anchor = self.buffer.create_child_anchor(&mut end);
        self.view.add_child_at_anchor(widget, &anchor);
        self.write_plain("\n");
    }
}

/// Moves the block tags' margins along with the view's, `margin` pixels:
/// a tag's margin is counted from the view's edge, not from the text's.
pub fn indent(buffer: &gtk::TextBuffer, margin: i32) {
    let table = buffer.tag_table();
    let set = |name: &str, left: i32, right: Option<i32>| {
        if let Some(tag) = table.lookup(name) {
            tag.set_left_margin(margin + left);
            if let Some(right) = right {
                tag.set_right_margin(margin + right);
            }
        }
    };
    set("pre", 20, Some(20));
    set("quote", 30, None);
    for depth in 1..=6 {
        set(&format!("list{depth}"), 25 * depth, None);
    }
}

fn create_tags(buffer: &gtk::TextBuffer) {
    let table = buffer.tag_table();
    if table.lookup("p").is_some() {
        return;
    }
    let grey = |alpha: f32| gdk::RGBA::new(0.5, 0.5, 0.5, alpha);
    let mono = "CaskaydiaCove Nerd Font, monospace";
    let tags = [
        gtk::TextTag::builder().name("p").pixels_below_lines(10).build(),
        gtk::TextTag::builder().name("strong").weight(700).build(),
        gtk::TextTag::builder().name("em").style(pango::Style::Italic).build(),
        gtk::TextTag::builder().name("strike").strikethrough(true).build(),
        gtk::TextTag::builder()
            .name("code")
            .family(mono)
            .background_rgba(&grey(0.2))
            .build(),
        gtk::TextTag::builder()
            .name("pre")
            .family(mono)
            .paragraph_background_rgba(&grey(0.12))
            .left_margin(20)
            .right_margin(20)
            .pixels_above_lines(2)
            .wrap_mode(gtk::WrapMode::Char)
            .build(),
        gtk::TextTag::builder()
            .name("lang")
            .foreground_rgba(&grey(1.0))
            .scale(0.85)
            .build(),
        gtk::TextTag::builder()
            .name("quote")
            .left_margin(30)
            .style(pango::Style::Italic)
            .foreground_rgba(&grey(1.0))
            .build(),
        gtk::TextTag::builder()
            .name("rule")
            .justification(gtk::Justification::Center)
            .foreground_rgba(&grey(0.6))
            .build(),
        gtk::TextTag::builder()
            .name("note")
            .scale(0.8)
            .rise(4 * pango::SCALE)
            .foreground_rgba(&grey(1.0))
            .build(),
    ];
    for tag in tags {
        table.add(&tag);
    }
    for (level, scale) in [2.0, 1.6, 1.35, 1.2, 1.1, 1.0].into_iter().enumerate() {
        table.add(
            &gtk::TextTag::builder()
                .name(format!("h{}", level + 1))
                .scale(scale)
                .weight(700)
                .pixels_above_lines(15)
                .pixels_below_lines(5)
                .build(),
        );
    }
    for depth in 1..=6 {
        table.add(
            &gtk::TextTag::builder()
                .name(format!("list{depth}"))
                .left_margin(25 * depth)
                .indent(-15)
                .pixels_below_lines(3)
                .build(),
        );
    }
}

/// `url` as a file next to the document, when it is one.
fn local_path(base: &Path, url: &str) -> Option<PathBuf> {
    if url.contains("://") {
        return url
            .strip_prefix("file://")
            .map(PathBuf::from)
            .filter(|p| p.is_file());
    }
    let url = url.split(['#', '?']).next().unwrap_or(url);
    let path = base.join(percent_decode(url));
    path.is_file().then_some(path)
}

/// `%20` → space, and the like.
pub fn percent_decode(text: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16);
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A heading's anchor as GitHub makes it: lower case, spaces to hyphens,
/// punctuation dropped.
pub fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_like_github() {
        assert_eq!(slug("Instalação rápida"), "instalação-rápida");
        assert_eq!(slug("What's new? (v2)"), "whats-new-v2");
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("my%20image.png"), "my image.png");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
