//! Project icons: a curated set of themed symbolic icons, or a custom image.

use crate::model::ProjectIcon;
use gtk::prelude::*;

pub const DEFAULT_ICON: &str = "folder-open-symbolic";

/// Candidate icons; only those present in the current icon theme are offered.
const CANDIDATES: &[&str] = &[
    "folder-open-symbolic",
    "user-home-symbolic",
    "network-server-symbolic",
    "globe-symbolic",
    "web-browser-symbolic",
    "computer-symbolic",
    "phone-symbolic",
    "drive-harddisk-symbolic",
    "security-high-symbolic",
    "dialog-password-symbolic",
    "system-users-symbolic",
    "avatar-default-symbolic",
    "emoji-people-symbolic",
    "mail-unread-symbolic",
    "x-office-document-symbolic",
    "applications-engineering-symbolic",
    "applications-science-symbolic",
    "applications-utilities-symbolic",
    "emblem-system-symbolic",
    "power-profile-performance-symbolic",
    "applications-graphics-symbolic",
    "applications-multimedia-symbolic",
    "camera-photo-symbolic",
    "applications-games-symbolic",
    "input-gaming-symbolic",
    "starred-symbolic",
    "heart-filled-symbolic",
    "weather-clear-symbolic",
    "weather-few-clouds-symbolic",
    "emoji-nature-symbolic",
    "emoji-food-symbolic",
    "emoji-activities-symbolic",
    "emoji-objects-symbolic",
    "emoji-flags-symbolic",
];

pub fn available() -> Vec<&'static str> {
    let Some(display) = gtk::gdk::Display::default() else { return CANDIDATES.to_vec() };
    let theme = gtk::IconTheme::for_display(&display);
    CANDIDATES.iter().copied().filter(|n| theme.has_icon(n)).collect()
}

/// Shows `icon` (or the default) in `image` at `size` pixels.
pub fn apply(image: &gtk::Image, icon: Option<&ProjectIcon>, size: i32) {
    for key in super::colors::COLORS.iter().map(|(k, _, _)| k) {
        image.remove_css_class(&format!("color-{key}"));
    }
    image.add_css_class("tree-icon");
    image.set_pixel_size(size);
    match icon {
        Some(ProjectIcon::Image { path }) => match gtk::gdk::Texture::from_filename(crate::storage::resolve_project_image(path)) {
            Ok(texture) => image.set_paintable(Some(&texture)),
            Err(_) => image.set_icon_name(Some(DEFAULT_ICON)),
        },
        Some(ProjectIcon::Symbolic { name, color }) => {
            image.set_icon_name(Some(name));
            if let Some(c) = color {
                image.add_css_class(&format!("color-{c}"));
            }
        }
        None => image.set_icon_name(Some(DEFAULT_ICON)),
    }
}

pub fn image(icon: Option<&ProjectIcon>, size: i32) -> gtk::Image {
    let img = gtk::Image::new();
    apply(&img, icon, size);
    img
}
