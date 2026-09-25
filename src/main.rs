//! Pigeon - a native GTK4 / libadwaita API client.

mod assertions;
mod codegen;
mod http;
mod model;
mod openapi;
mod interchange;
mod storage;
mod ui;
mod vars;

use adw::prelude::*;

pub const APP_ID: &str = "dev.pigeon.Pigeon";

fn main() -> gtk::glib::ExitCode {
    // `pigeon --export-icon <file.svg> [variant]`: used by the packaging scripts
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--export-icon") {
        let Some(path) = args.get(i + 1) else {
            eprintln!("usage: pigeon --export-icon <file.svg> [variant]");
            return gtk::glib::ExitCode::FAILURE;
        };
        let key = args.get(i + 2).map(String::as_str).unwrap_or(ui::app_icon::DEFAULT);
        return match std::fs::write(path, ui::app_icon::variant(key).svg()) {
            Ok(_) => gtk::glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("pigeon: {path}: {e}");
                gtk::glib::ExitCode::FAILURE
            }
        };
    }
    // start the network runtime early so the first request is snappy
    once_cell::sync::Lazy::force(&http::RUNTIME);

    let app = adw::Application::builder().application_id(APP_ID).build();
    // debug snapshot runs must not hand off to an already-running instance
    #[cfg(debug_assertions)]
    if std::env::var("PIGEON_SNAPSHOT").is_ok() {
        app.set_flags(gtk::gio::ApplicationFlags::NON_UNIQUE);
    }

    app.connect_startup(|_| {
        // the app icon is generated from code (see ui::app_icon), so it works even uninstalled
        ui::app_icon::apply(&storage::load_settings().app_icon);
        gtk::Window::set_default_icon_name(APP_ID);
        // hand cursor over everything clickable
        ui::cursors::install();
        let provider = gtk::CssProvider::new();
        provider.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("no display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        let colors = gtk::CssProvider::new();
        colors.load_from_string(&ui::colors::css());
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("no display"),
            &colors,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });

    app.connect_activate(|app| {
        if let Some(win) = app.active_window() {
            win.present();
            return;
        }
        let window = ui::window::App::new(app);
        window.present();
        // keep the App alive for the lifetime of its window
        let holder = std::cell::RefCell::new(Some(window.clone()));
        window.window.connect_destroy(move |_| {
            holder.borrow_mut().take();
        });
        #[cfg(debug_assertions)]
        debug_snapshot(&window);
    });

    for (action, accels) in [
        ("win.send", &["<Control>Return", "<Control>KP_Enter"][..]),
        ("win.save", &["<Control>s"]),
        ("win.new-tab", &["<Control>t", "<Control>n"]),
        ("win.close-tab", &["<Control>w"]),
        ("win.focus-url", &["<Control>l"]),
        ("win.import", &["<Control>o"]),
        ("win.environments", &["<Control>e"]),
        ("win.code", &["<Control><Shift>c"]),
        ("win.toggle-sidebar", &["F9"]),
        ("win.preferences", &["<Control>comma"]),
    ] {
        app.set_accels_for_action(action, accels);
    }

    app.run()
}

/// Debug aid: `PIGEON_SNAPSHOT=out.png` renders the window to a PNG after a delay and quits.
/// `PIGEON_SNAPSHOT_SEND=1` also sends the current request first.
#[cfg(debug_assertions)]
fn debug_snapshot(app: &std::rc::Rc<ui::window::App>) {
    let Ok(path) = std::env::var("PIGEON_SNAPSHOT") else { return };
    if let Ok(send_at) = std::env::var("PIGEON_SNAPSHOT_SEND") {
        let a = app.clone();
        let ms = send_at.parse().unwrap_or(800);
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
            if let Some(tab) = a.current_tab() {
                a.send(&tab);
            }
        });
    }
    if let Ok(url) = std::env::var("PIGEON_SNAPSHOT_IMPORT") {
        app.import_from_url(url);
    }
    if let Ok(action) = std::env::var("PIGEON_SNAPSHOT_ACTION") {
        let win = app.window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            let (name, target) = action.split_once(':').map(|(a, t)| (a.to_string(), Some(t.to_variant()))).unwrap_or((action.clone(), None));
            let _ = gtk::prelude::WidgetExt::activate_action(&win, &name, target.as_ref());
        });
    }
    if let Ok(index) = std::env::var("PIGEON_SNAPSHOT_POPUP") {
        let (a, out) = (app.clone(), format!("{path}.popup.png"));
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            let Some(pop) = a.debug_popup_row(index.parse().unwrap_or(0)) else { return };
            let win = a.window.clone();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(700), move || {
                let paintable = gtk::WidgetPaintable::new(Some(&pop));
                let snapshot = gtk::Snapshot::new();
                let scale: f32 = std::env::var("PIGEON_SNAPSHOT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                snapshot.scale(scale, scale);
                paintable.snapshot(&snapshot, pop.width() as f64, pop.height() as f64);
                if let Some(b) = pop.parent().and_then(|anchor| anchor.compute_bounds(&win)) {
                    eprintln!("POPUP_ANCHOR {} {} {} {} POPUP_SIZE {} {}", b.x(), b.y(), b.width(), b.height(), pop.width(), pop.height());
                }
                if let (Some(node), Some(renderer)) = (snapshot.to_node(), pop.native().and_then(|n| n.renderer())) {
                    let _ = renderer.render_texture(&node, None).save_to_png(&out);
                }
                pop.popdown();
            });
        });
    }
    if std::env::var("PIGEON_MEASURE").is_ok() {
        let win = app.window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            if let Some(content) = win.content() {
                let (min_w, _, _, _) = content.measure(gtk::Orientation::Horizontal, -1);
                let (min_h, _, _, _) = content.measure(gtk::Orientation::Vertical, -1);
                let (min_h_at_w, _, _, _) = content.measure(gtk::Orientation::Vertical, min_w.max(360));
                eprintln!("MEASURE content min {min_w}x{min_h} (height at min width: {min_h_at_w})");
            }
        });
    }
    if let Ok(views) = std::env::var("PIGEON_SNAPSHOT_VIEWS") {
        // "0:body,1:tests": show page `name` in the n-th mapped view stack (tree order)
        let win = app.window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1800), move || {
            fn stacks(w: &gtk::Widget, out: &mut Vec<adw::ViewStack>) {
                if let Some(s) = w.downcast_ref::<adw::ViewStack>() {
                    if s.is_mapped() {
                        out.push(s.clone());
                    }
                }
                let mut c = w.first_child();
                while let Some(ch) = c {
                    stacks(&ch, out);
                    c = ch.next_sibling();
                }
            }
            let mut all = vec![];
            stacks(win.upcast_ref(), &mut all);
            for (i, s) in all.iter().enumerate() {
                eprintln!("VIEWSTACK {i}: visible={:?}", s.visible_child_name());
            }
            for spec in views.split(',') {
                if let Some((i, name)) = spec.split_once(':') {
                    if let Some(s) = i.parse::<usize>().ok().and_then(|i| all.get(i)) {
                        s.set_visible_child_name(name);
                    }
                }
            }
        });
    }
    if let Ok(label) = std::env::var("PIGEON_SNAPSHOT_CLICK") {
        // clicks the first visible button whose label matches (e.g. "Run" in the runner)
        let win = app.window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
            fn find(w: &gtk::Widget, label: &str) -> Option<gtk::Button> {
                if let Some(b) = w.downcast_ref::<gtk::Button>() {
                    if b.label().as_deref() == Some(label) && b.is_mapped() {
                        return Some(b.clone());
                    }
                }
                let mut c = w.first_child();
                while let Some(ch) = c {
                    if let Some(b) = find(&ch, label) {
                        return Some(b);
                    }
                    c = ch.next_sibling();
                }
                None
            }
            if let Some(b) = find(win.upcast_ref(), &label) {
                b.emit_clicked();
            }
        });
    }
    if std::env::var("PIGEON_SNAPSHOT_HOVER").is_ok() {
        // mark the first cell of every icon grid (and the button in it) as hovered
        let win = app.window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(2200), move || {
            fn walk(w: &gtk::Widget) {
                if w.has_css_class("app-icon-grid") || w.has_css_class("icon-grid") {
                    if let Some(cell) = w.first_child() {
                        cell.set_state_flags(gtk::StateFlags::PRELIGHT, false);
                        if let Some(b) = cell.first_child() {
                            b.set_state_flags(gtk::StateFlags::PRELIGHT, false);
                        }
                    }
                }
                let mut c = w.first_child();
                while let Some(ch) = c {
                    walk(&ch);
                    c = ch.next_sibling();
                }
            }
            walk(win.upcast_ref());
        });
    }
    if std::env::var("PIGEON_DEBUG_CURSORS").is_ok() {
        let win = app.window.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || {
            let mut stats: std::collections::BTreeMap<String, (u32, u32)> = Default::default();
            fn walk(w: &gtk::Widget, stats: &mut std::collections::BTreeMap<String, (u32, u32)>) {
                if w.is_mapped() {
                    let e = stats.entry(w.type_().name().to_string()).or_default();
                    e.0 += 1;
                    if w.cursor().and_then(|c| c.name()).as_deref() == Some("pointer") {
                        e.1 += 1;
                    }
                }
                let mut c = w.first_child();
                while let Some(ch) = c {
                    walk(&ch, stats);
                    c = ch.next_sibling();
                }
            }
            walk(win.upcast_ref(), &mut stats);
            for (ty, (total, hand)) in stats {
                if hand > 0 || ty.contains("Button") || ty.contains("Row") || ty.contains("Entry") || ty.contains("DropDown") || ty == "GtkText" {
                    eprintln!("CURSORS {ty:<28} mapped={total:<4} hand={hand}");
                }
            }
        });
    }
    let win = app.window.clone();
    let delay = std::env::var("PIGEON_SNAPSHOT_DELAY").ok().and_then(|d| d.parse().ok()).unwrap_or(3000);
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        let paintable = gtk::WidgetPaintable::new(Some(&win));
        let (w, h) = (win.width(), win.height());
        let snapshot = gtk::Snapshot::new();
        let scale: f32 = std::env::var("PIGEON_SNAPSHOT_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
        snapshot.scale(scale, scale);
        paintable.snapshot(&snapshot, w as f64, h as f64);
        if let Some(node) = snapshot.to_node() {
            if let Some(renderer) = win.native().and_then(|n| n.renderer()) {
                let tex = renderer.render_texture(&node, None);
                let _ = tex.save_to_png(&path);
            }
        }
        win.close();
    });
}
