//! Monospace text views with lightweight syntax highlighting (JSON / XML / HTML).

use gtk::prelude::*;
use once_cell::sync::Lazy;
use regex::Regex;

/// Skip highlighting above this size to keep the UI responsive.
const MAX_HIGHLIGHT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Plain,
    Json,
    Xml,
}

pub fn lang_for_content_type(ct: &str) -> Lang {
    if ct.contains("json") {
        Lang::Json
    } else if ct.contains("xml") || ct.contains("html") {
        Lang::Xml
    } else {
        Lang::Plain
    }
}

pub fn new_view(editable: bool) -> (gtk::ScrolledWindow, gtk::TextView) {
    let view = gtk::TextView::builder()
        .monospace(true)
        .editable(editable)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .build();
    view.add_css_class("code-view");
    install_tags(&view.buffer());
    let scroll = gtk::ScrolledWindow::builder().child(&view).vexpand(true).hexpand(true).build();
    (scroll, view)
}

fn palette(dark: bool) -> [(&'static str, &'static str); 6] {
    if dark {
        [("key", "#c68ff5"), ("string", "#8ff0a4"), ("number", "#ffbe6f"), ("keyword", "#78aeed"), ("punct", "#9a9996"), ("tag", "#78aeed")]
    } else {
        [("key", "#8b24c7"), ("string", "#1a7f37"), ("number", "#c64600"), ("keyword", "#1c5fb8"), ("punct", "#77767b"), ("tag", "#1c5fb8")]
    }
}

fn install_tags(buffer: &gtk::TextBuffer) {
    let style = adw::StyleManager::default();
    let table = buffer.tag_table();
    for (name, color) in palette(style.is_dark()) {
        let tag = gtk::TextTag::builder().name(name).foreground(color).build();
        if name == "keyword" {
            tag.set_weight(600);
        }
        table.add(&tag);
    }
    table.add(&gtk::TextTag::builder().name("search").background("rgba(246, 211, 45, 0.55)").build());

    // follow light/dark switches
    let weak = buffer.downgrade();
    style.connect_dark_notify(move |sm| {
        if let Some(buf) = weak.upgrade() {
            for (name, color) in palette(sm.is_dark()) {
                if let Some(tag) = buf.tag_table().lookup(name) {
                    tag.set_foreground(Some(color));
                }
            }
        }
    });
}

static JSON_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?P<str>"(?:[^"\\]|\\.)*")(?P<colon>\s*:)?|(?P<num>-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|(?P<kw>\btrue\b|\bfalse\b|\bnull\b)|(?P<punct>[{}\[\],])"#).unwrap()
});

static XML_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?P<tag></?[\w:.-]+|/?>)|(?P<attr>[\w:.-]+)=(?P<str>"[^"]*"|'[^']*')|(?P<comment><!--[\s\S]*?-->)"#).unwrap()
});

/// Converts a byte offset in `text` to a char offset for GtkTextIter.
struct OffsetMap<'a> {
    text: &'a str,
    last_byte: usize,
    last_char: i32,
}

impl<'a> OffsetMap<'a> {
    fn chars(&mut self, byte: usize) -> i32 {
        // regex matches are produced in order, so we can walk forward incrementally
        if byte < self.last_byte {
            self.last_byte = 0;
            self.last_char = 0;
        }
        self.last_char += self.text[self.last_byte..byte].chars().count() as i32;
        self.last_byte = byte;
        self.last_char
    }
}

pub fn highlight(buffer: &gtk::TextBuffer, lang: Lang) {
    let (start, end) = buffer.bounds();
    for name in ["key", "string", "number", "keyword", "punct", "tag"] {
        buffer.remove_tag_by_name(name, &start, &end);
    }
    let text = buffer.text(&start, &end, false);
    if text.len() > MAX_HIGHLIGHT_BYTES || lang == Lang::Plain {
        return;
    }
    let mut map = OffsetMap { text: &text, last_byte: 0, last_char: 0 };
    let apply = |name: &str, range: std::ops::Range<usize>, map: &mut OffsetMap| {
        let s = map.chars(range.start);
        let e = map.chars(range.end);
        buffer.apply_tag_by_name(name, &buffer.iter_at_offset(s), &buffer.iter_at_offset(e));
    };
    match lang {
        Lang::Json => {
            for caps in JSON_RE.captures_iter(&text) {
                if let Some(m) = caps.name("str") {
                    let tag = if caps.name("colon").is_some() { "key" } else { "string" };
                    apply(tag, m.range(), &mut map);
                } else if let Some(m) = caps.name("num") {
                    apply("number", m.range(), &mut map);
                } else if let Some(m) = caps.name("kw") {
                    apply("keyword", m.range(), &mut map);
                } else if let Some(m) = caps.name("punct") {
                    apply("punct", m.range(), &mut map);
                }
            }
        }
        Lang::Xml => {
            for caps in XML_RE.captures_iter(&text) {
                if let Some(m) = caps.name("tag") {
                    apply("tag", m.range(), &mut map);
                } else if let Some(m) = caps.name("comment") {
                    apply("punct", m.range(), &mut map);
                } else {
                    if let Some(m) = caps.name("attr") {
                        apply("key", m.range(), &mut map);
                    }
                    if let Some(m) = caps.name("str") {
                        apply("string", m.range(), &mut map);
                    }
                }
            }
        }
        Lang::Plain => {}
    }
}

/// Re-highlights a buffer shortly after the user stops typing.
pub fn auto_highlight(buffer: &gtk::TextBuffer, lang: impl Fn() -> Lang + 'static) {
    let pending: std::rc::Rc<std::cell::Cell<Option<gtk::glib::SourceId>>> = Default::default();
    buffer.connect_changed(move |buf| {
        if let Some(id) = pending.take() {
            id.remove();
        }
        let buf = buf.clone();
        let lang = lang();
        let pending2 = pending.clone();
        pending.set(Some(gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            pending2.set(None);
            highlight(&buf, lang);
        })));
    });
}

/// Pretty-prints JSON, returning the input unchanged if it doesn't parse.
pub fn pretty_json(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&v).ok()
}

/// Minimal XML indenter for display purposes.
pub fn pretty_xml(text: &str) -> String {
    static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(<[^>]+>)").unwrap());
    let mut out = String::new();
    let mut depth: usize = 0;
    for piece in TAG.split(text).zip(TAG.find_iter(text).map(Some).chain(std::iter::once(None))) {
        let (between, tag) = piece;
        let between = between.trim();
        if !between.is_empty() {
            out += &"  ".repeat(depth);
            out += between;
            out.push('\n');
        }
        if let Some(tag) = tag {
            let t = tag.as_str();
            let closing = t.starts_with("</");
            let self_closing = t.ends_with("/>") || t.starts_with("<?") || t.starts_with("<!");
            if closing {
                depth = depth.saturating_sub(1);
            }
            out += &"  ".repeat(depth);
            out += t;
            out.push('\n');
            if !closing && !self_closing {
                depth += 1;
            }
        }
    }
    out.trim_end().to_string()
}

/// Selects and scrolls to the next occurrence of `needle` after the cursor.
/// Returns false when nothing was found.
pub fn find_next(view: &gtk::TextView, needle: &str, backwards: bool) -> bool {
    let buffer = view.buffer();
    let (start, end) = buffer.bounds();
    buffer.remove_tag_by_name("search", &start, &end);
    if needle.is_empty() {
        return false;
    }
    let flags = gtk::TextSearchFlags::CASE_INSENSITIVE | gtk::TextSearchFlags::TEXT_ONLY;
    let cursor = buffer.iter_at_mark(&buffer.get_insert());
    let found = if backwards {
        let (sel_start, _) = buffer.selection_bounds().unwrap_or((cursor, cursor));
        sel_start.backward_search(needle, flags, None).or_else(|| end.backward_search(needle, flags, None))
    } else {
        let (_, sel_end) = buffer.selection_bounds().unwrap_or((cursor, cursor));
        sel_end.forward_search(needle, flags, None).or_else(|| start.forward_search(needle, flags, None))
    };
    match found {
        Some((mut s, e)) => {
            buffer.apply_tag_by_name("search", &s, &e);
            buffer.select_range(&s, &e);
            view.scroll_to_iter(&mut s, 0.1, false, 0.0, 0.3);
            true
        }
        None => false,
    }
}
