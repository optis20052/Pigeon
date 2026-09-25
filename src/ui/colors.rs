//! Label colors for collections and folders (JetBrains-style "file colors").

/// (key, label, color) - GNOME palette tones that read well on light and dark backgrounds.
pub const COLORS: [(&str, &str, &str); 9] = [
    ("blue", "Blue", "#3584e4"),
    ("teal", "Teal", "#2190a4"),
    ("green", "Green", "#2ec27e"),
    ("yellow", "Yellow", "#e5a50a"),
    ("orange", "Orange", "#ff7800"),
    ("red", "Red", "#e01b24"),
    ("pink", "Pink", "#d56199"),
    ("purple", "Purple", "#9141ac"),
    ("slate", "Slate", "#6f8396"),
];

pub fn is_valid(key: &str) -> bool {
    COLORS.iter().any(|(k, _, _)| *k == key)
}

pub fn tint_class(key: &str) -> String {
    format!("tint-{key}")
}

pub fn all_tint_classes() -> impl Iterator<Item = String> {
    COLORS.iter().map(|(k, _, _)| tint_class(k))
}

/// Stylesheet for icons, row tints, tab tints and swatches.
pub fn css() -> String {
    let mut out = String::new();
    for (key, _, color) in COLORS {
        out += &format!(
            "image.tree-icon.color-{key} {{ color: {color}; }}\n\
             row.tint-{key} {{ background-color: alpha({color}, 0.10); }}\n\
             row.tint-{key}:hover {{ background-color: alpha({color}, 0.17); }}\n\
             row.tint-{key}:selected {{ background-color: alpha({color}, 0.26); }}\n\
             .tab-chip.tint-{key} {{ background-color: alpha({color}, 0.10); box-shadow: inset 0 -2px alpha({color}, 0.55); }}\n\
             .tab-chip.tint-{key}:hover {{ background-color: alpha({color}, 0.17); }}\n\
             .tab-chip.tint-{key}.selected {{ background-color: alpha({color}, 0.24); box-shadow: inset 0 -2px {color}; }}\n\
             button.color-swatch.swatch-{key} {{ background: {color}; }}\n"
        );
    }
    out
}
