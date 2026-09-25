//! Small helpers shared by the UI.

use crate::model::KeyValue;
use gtk::prelude::*;

/// Parses the query string of a URL into key/value pairs without decoding,
/// so `{{variables}}` stay intact.
pub fn parse_query(url: &str) -> Vec<KeyValue> {
    let Some((_, query)) = url.split_once('?') else { return vec![] };
    let query = query.split('#').next().unwrap_or("");
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            KeyValue::new(k, v)
        })
        .collect()
}

/// Replaces the query string of `url` with the enabled `params`.
pub fn with_query(url: &str, params: &[KeyValue]) -> String {
    let (base, rest) = url.split_once('?').map(|(b, r)| (b, Some(r))).unwrap_or((url, None));
    let fragment = rest.and_then(|r| r.split_once('#').map(|(_, f)| f.to_string())).or_else(|| {
        if rest.is_none() { base.split_once('#').map(|(_, f)| f.to_string()) } else { None }
    });
    let base = base.split('#').next().unwrap_or(base);
    let query: Vec<String> = params
        .iter()
        .filter(|p| p.enabled && !(p.key.is_empty() && p.value.is_empty()))
        .map(|p| if p.value.is_empty() && !p.key.is_empty() { p.key.clone() } else { format!("{}={}", p.key, p.value) })
        .collect();
    let mut out = base.to_string();
    if !query.is_empty() {
        out.push('?');
        out.push_str(&query.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(&f);
    }
    out
}

pub fn dropdown(items: &[&str]) -> gtk::DropDown {
    let dd = gtk::DropDown::from_strings(items);
    dd.set_valign(gtk::Align::Center);
    dd
}

pub fn text_of(buffer: &gtk::TextBuffer) -> String {
    let (s, e) = buffer.bounds();
    buffer.text(&s, &e, false).to_string()
}

pub fn labeled_row(label: &str, child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let l = gtk::Label::new(Some(label));
    l.set_xalign(0.0);
    l.set_width_chars(14);
    l.add_css_class("dim-label");
    b.append(&l);
    child.set_hexpand(true);
    b.append(child);
    b
}

pub fn copy_to_clipboard(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.clipboard().set_text(text);
}

pub fn status_css(status: u16) -> &'static str {
    match status {
        200..=299 => "status-ok",
        300..=399 => "status-redirect",
        400..=499 => "status-client-error",
        _ => "status-server-error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_roundtrip() {
        let p = parse_query("https://x/y?a=1&b={{v}}#frag");
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].value, "{{v}}");
        assert_eq!(with_query("https://x/y?old=1#frag", &p), "https://x/y?a=1&b={{v}}#frag");
        assert_eq!(with_query("https://x/y?a=1", &[]), "https://x/y");
    }
}
