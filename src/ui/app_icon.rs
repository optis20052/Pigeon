//! App icon variants. All variants share one geometry; only the colors differ, so each
//! variant is a palette and the SVG is generated here (the single source of truth for the
//! in-app picker, the installed icon and the per-user override).

use std::path::PathBuf;

pub struct Variant {
    pub key: &'static str,
    pub label: &'static str,
    background: &'static str,
    /// Optional outline for light backgrounds.
    border: Option<&'static str>,
    body: &'static str,
    body_shade: &'static str,
    neck: &'static str,
    beak: &'static str,
    wing: &'static str,
    wing_shade: &'static str,
}

pub const DEFAULT: &str = "dusk";

pub const VARIANTS: [Variant; 5] = [
    Variant { key: "dusk", label: "Dusk", background: "#2A1F4D", border: None, body: "#B9ADEB", body_shade: "#8F80D6", neck: "#FFFFFF", beak: "#FFC23D", wing: "#FFFFFF", wing_shade: "#DCD5F7" },
    Variant { key: "cobalt", label: "Cobalt", background: "#2F5BEA", border: None, body: "#AFC1FF", body_shade: "#8098F0", neck: "#FFFFFF", beak: "#FFC23D", wing: "#FFFFFF", wing_shade: "#DCE4FF" },
    Variant { key: "mint", label: "Mint", background: "#159A78", border: None, body: "#A6E3D0", body_shade: "#7CCDB4", neck: "#FFFFFF", beak: "#FFD84A", wing: "#FFFFFF", wing_shade: "#D6F3EA" },
    Variant { key: "sunrise", label: "Sunrise", background: "#FFB930", border: None, body: "#4A5896", body_shade: "#34407A", neck: "#1B2447", beak: "#FFFFFF", wing: "#1B2447", wing_shade: "#2D3A6E" },
    Variant { key: "ivory", label: "Ivory", background: "#EEF1F8", border: Some("#D5DAE8"), body: "#8C9AE6", body_shade: "#6B7BD6", neck: "#3B4BA8", beak: "#F5B82E", wing: "#3B4BA8", wing_shade: "#5566C4" },
];

pub fn variant(key: &str) -> &'static Variant {
    VARIANTS.iter().find(|v| v.key == key).unwrap_or(&VARIANTS[0])
}

impl Variant {
    /// The icon as SVG (512×512 canvas).
    pub fn svg(&self) -> String {
        let border = self.border.map(|c| format!(r#" stroke="{c}" stroke-width="4""#)).unwrap_or_default();
        let poly = |points: &str, fill: &str| format!(r#"<polygon points="{points}" fill="{fill}"/>"#);
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512"><rect x="32" y="32" width="448" height="448" rx="100" fill="{bg}"{border}/>{body}{shade}{neck}{beak}{wing}{wing_shade}</svg>"#,
            bg = self.background,
            body = poly("110,326 300,356 392,168", self.body),
            shade = poly("300,356 392,168 350,300", self.body_shade),
            neck = poly("344,214 392,168 366,232", self.neck),
            beak = poly("388,164 432,176 386,188", self.beak),
            wing = poly("156,322 350,216 206,104", self.wing),
            wing_shade = poly("156,322 206,104 190,258", self.wing_shade),
        )
    }

    pub fn texture(&self) -> Option<gtk::gdk::Texture> {
        gtk::gdk::Texture::from_bytes(&gtk::glib::Bytes::from_owned(self.svg().into_bytes())).ok()
    }
}

/// Private icon-theme dir that Pigeon itself looks in first, so in-app icons follow the choice.
fn private_theme_dir() -> PathBuf {
    crate::storage::data_dir().join("icon-theme")
}

/// Per-user override picked up by GNOME Shell (dock, app grid), which checks
/// `~/.local/share/icons` before the system-wide icon installed by the package.
fn user_icon_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("icons/hicolor/scalable/apps").join(format!("{}.svg", crate::APP_ID)))
}

fn write_if_changed(path: &std::path::Path, contents: &str) -> std::io::Result<bool> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(contents) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents)?;
    Ok(true)
}

/// Makes `key` the app icon: in-app right away, and for the desktop via the per-user override.
pub fn apply(key: &str) {
    let svg = variant(key).svg();
    let private = private_theme_dir().join("hicolor/scalable/apps").join(format!("{}.svg", crate::APP_ID));
    let _ = write_if_changed(&private, &svg);

    if let Some(path) = user_icon_path() {
        if let Ok(true) = write_if_changed(&path, &svg) {
            // A stale icon cache would hide the new file: refresh it if there is one, and
            // bump the theme dir's mtime so running apps (and GNOME Shell) notice the change.
            if let Some(theme_dir) = path.ancestors().nth(3) {
                if theme_dir.join("icon-theme.cache").exists() {
                    let _ = std::process::Command::new("gtk-update-icon-cache").args(["-q", "-t", "-f"]).arg(theme_dir).status();
                }
                let _ = std::fs::File::open(theme_dir).and_then(|f| f.set_modified(std::time::SystemTime::now()));
            }
        }
    }

    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        let mut paths: Vec<PathBuf> = theme.search_path().into_iter().filter(|p| *p != private_theme_dir()).collect();
        paths.insert(0, private_theme_dir());
        let refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
        // resetting the search path also makes GTK reload icons already on screen
        theme.set_search_path(&refs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_are_distinct_and_default_exists() {
        assert_eq!(variant("nope").key, DEFAULT);
        let mut keys: Vec<_> = VARIANTS.iter().map(|v| v.key).collect();
        keys.dedup();
        assert_eq!(keys.len(), VARIANTS.len());
        assert!(VARIANTS.iter().all(|v| v.svg().starts_with("<svg") && v.svg().ends_with("</svg>")));
    }
}
