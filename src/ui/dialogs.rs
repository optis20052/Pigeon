//! Assorted dialogs: prompts, confirmations, save target, code generation, container settings.

use super::auth_form::AuthForm;
use super::code_view;
use super::kv_editor::{KvEditor, Row};
use super::util::{dropdown, text_of};
use super::window::App;
use crate::codegen::{self, Target};
use crate::http::Prepared;
use crate::model::*;
use adw::prelude::*;
use std::rc::Rc;

/// Asks for a single line of text.
pub fn prompt(parent: &impl IsA<gtk::Widget>, heading: &str, label: &str, initial: &str, accept: &str, on_ok: impl Fn(String) + 'static) {
    let dialog = adw::AlertDialog::new(Some(heading), None);
    let entry = adw::EntryRow::builder().title(label).text(initial).activates_default(true).build();
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    list.append(&entry);
    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", accept)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    let e = entry.clone();
    dialog.connect_response(None, move |_, resp| {
        let text = e.text().trim().to_string();
        if resp == "ok" && !text.is_empty() {
            on_ok(text);
        }
    });
    dialog.present(Some(parent));
    entry.grab_focus();
}

pub fn confirm(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str, accept: &str, on_ok: impl Fn() + 'static) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", accept)]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "ok" {
            on_ok();
        }
    });
    dialog.present(Some(parent));
}

/// Asks for a name and a target collection. `None` target means "create a new collection".
pub fn save_request(app: &Rc<App>, name: &str, on_ok: impl Fn(String, Option<String>) + 'static) {
    let dialog = adw::AlertDialog::new(Some("Save Request"), None);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let entry = adw::EntryRow::builder().title("Request name").text(name).activates_default(true).build();
    list.append(&entry);

    // collections and folders, indented
    let mut labels = vec![];
    let mut ids: Vec<Option<String>> = vec![];
    fn walk(items: &[Item], depth: usize, labels: &mut Vec<String>, ids: &mut Vec<Option<String>>) {
        for i in items {
            if let Item::Folder(f) = i {
                labels.push(format!("{}{}", "    ".repeat(depth), f.name));
                ids.push(Some(f.id.clone()));
                walk(&f.items, depth + 1, labels, ids);
            }
        }
    }
    for c in &app.ws.borrow().collections {
        labels.push(c.name.clone());
        ids.push(Some(c.id.clone()));
        walk(&c.items, 1, &mut labels, &mut ids);
    }
    labels.push("+ New Collection".into());
    ids.push(None);
    let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let combo = adw::ComboRow::builder().title("Save to").model(&gtk::StringList::new(&refs)).build();
    list.append(&combo);

    dialog.set_extra_child(Some(&list));
    dialog.add_responses(&[("cancel", "Cancel"), ("ok", "Save")]);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, resp| {
        if resp == "ok" {
            let name = entry.text().trim().to_string();
            let target = ids.get(combo.selected() as usize).cloned().flatten();
            on_ok(if name.is_empty() { "Untitled Request".into() } else { name }, target);
        }
    });
    dialog.present(Some(&app.window));
}

pub fn codegen(parent: &impl IsA<gtk::Widget>, prepared: &Prepared) {
    let labels: Vec<&str> = Target::ALL.iter().map(|t| t.label()).collect();
    let target = dropdown(&labels);
    let copy = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Copy").build();
    let (scroll, view) = code_view::new_view(false);
    view.set_wrap_mode(gtk::WrapMode::None);
    scroll.add_css_class("card");

    let header = adw::HeaderBar::new();
    header.pack_start(&target);
    header.pack_end(&copy);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scroll.set_margin_start(12);
    scroll.set_margin_end(12);
    scroll.set_margin_bottom(12);
    body.append(&scroll);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&body));
    let dialog = adw::Dialog::builder().title("Generate Code").content_width(760).content_height(520).child(&tv).build();

    let p = prepared.clone();
    let v = view.clone();
    let render = move |i: u32| v.buffer().set_text(&codegen::generate(&p, Target::ALL[i as usize]));
    render(0);
    target.connect_selected_notify(move |dd| render(dd.selected()));
    let v = view.clone();
    let toast_parent = dialog.clone();
    copy.connect_clicked(move |btn| {
        super::util::copy_to_clipboard(btn, &text_of(&v.buffer()));
        btn.set_icon_name("object-select-symbolic");
        let _ = &toast_parent;
    });
    dialog.present(Some(parent));
}

/// Edits variables/auth/description of a collection, or auth/description of a folder.
pub fn edit_container(app: &Rc<App>, id: &str) {
    let (is_collection, name, auth, vars, desc) = {
        let mut ws = app.ws.borrow_mut();
        if let Some(c) = ws.collections.iter().find(|c| c.id == id) {
            (true, c.name.clone(), c.auth.clone(), c.variables.clone(), c.description.clone())
        } else {
            let mut found = None;
            for c in ws.collections.iter_mut() {
                if let Some(f) = find_folder_mut(&mut c.items, id) {
                    found = Some((false, f.name.clone(), f.auth.clone(), vec![], f.description.clone()));
                    break;
                }
            }
            match found {
                Some(f) => f,
                None => return,
            }
        }
    };

    let stack = adw::ViewStack::new();
    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();

    let variables = KvEditor::new(false, "Variable", "Value");
    variables.set_items(&vars.iter().map(Row::from).collect::<Vec<_>>());
    if is_collection {
        let b = pad(variables.widget());
        stack.add_titled_with_icon(&b, Some("vars"), "Variables", "accessories-dictionary-symbolic");
    }
    let auth_form = AuthForm::new(true);
    auth_form.load(&auth);
    auth_form.set_inherit_hint(&app.inherit_hint(app.collection_of(id).as_deref(), id));
    stack.add_titled_with_icon(&pad(auth_form.widget()), Some("auth"), "Authorization", "dialog-password-symbolic");
    let (desc_scroll, desc_view) = code_view::new_view(true);
    desc_view.set_monospace(false);
    desc_view.buffer().set_text(&desc);
    desc_scroll.add_css_class("card");
    stack.add_titled_with_icon(&pad(&desc_scroll), Some("docs"), "Docs", "text-x-generic-symbolic");

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&stack));
    let dialog = adw::Dialog::builder().title(&name).content_width(720).content_height(520).child(&tv).build();

    let app2 = app.clone();
    let id = id.to_string();
    dialog.connect_closed(move |_| {
        let auth = auth_form.collect();
        let desc = text_of(&desc_view.buffer());
        {
            let mut ws = app2.ws.borrow_mut();
            if let Some(c) = ws.collections.iter_mut().find(|c| c.id == id) {
                c.variables = variables.kvs();
                c.auth = auth;
                c.description = desc;
            } else {
                for c in ws.collections.iter_mut() {
                    if let Some(f) = find_folder_mut(&mut c.items, &id) {
                        f.auth = auth;
                        f.description = desc;
                        break;
                    }
                }
            }
        }
        app2.collections_changed();
    });
    dialog.present(Some(&app.window));
}

/// App-wide preferences.
pub fn preferences(app: &Rc<App>) {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferences");
    let page = adw::PreferencesPage::builder().title("General").icon_name("preferences-system-symbolic").build();

    page.add(&app_icon_group(app));

    let appearance = adw::PreferencesGroup::builder()
        .title("Folder Colors")
        .description("Choose how far a collection or folder color reaches in the sidebar and tabs.")
        .build();
    let labels: Vec<&str> = ColorScope::ALL.iter().map(|s| s.label()).collect();
    let scope = adw::ComboRow::builder().title("Colors apply to").model(&gtk::StringList::new(&labels)).build();
    let current = app.settings.borrow().color_scope;
    scope.set_selected(ColorScope::ALL.iter().position(|s| *s == current).unwrap_or(0) as u32);
    let explain = |s: ColorScope| match s {
        ColorScope::Cascade => "Subfolders and their requests are tinted too, unless they have their own color",
        ColorScope::Direct => "Requests directly in the folder are tinted; subfolders are not",
        ColorScope::RowOnly => "Only the colored folder's row is tinted",
    };
    scope.set_subtitle(explain(current));
    let app2 = app.clone();
    scope.connect_selected_notify(move |row| {
        let s = ColorScope::ALL[row.selected() as usize];
        row.set_subtitle(explain(s));
        app2.settings.borrow_mut().color_scope = s;
        app2.settings_changed();
    });
    appearance.add(&scope);
    page.add(&appearance);
    dialog.add(&page);
    dialog.present(Some(&app.window));
}

/// Row of app icon variants; clicking one makes it the app icon right away.
fn app_icon_group(app: &Rc<App>) -> adw::PreferencesGroup {
    use super::app_icon;
    let group = adw::PreferencesGroup::builder()
        .title("App Icon")
        .description("Shown in Pigeon, the dock and the app grid. The dock may take a moment to refresh.")
        .build();
    let row = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(3)
        .max_children_per_line(5)
        .column_spacing(8)
        .row_spacing(8)
        .build();
    row.add_css_class("app-icon-grid");
    let current = app.settings.borrow().app_icon.clone();
    let mut buttons: Vec<(&'static str, gtk::Button)> = vec![];
    for v in app_icon::VARIANTS.iter() {
        let picture = gtk::Picture::builder().can_shrink(true).content_fit(gtk::ContentFit::Contain).width_request(72).height_request(72).build();
        if let Some(t) = v.texture() {
            picture.set_paintable(Some(&t));
        }
        let label = gtk::Label::new(Some(v.label));
        label.add_css_class("caption");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.append(&picture);
        content.append(&label);
        let b = gtk::Button::builder().child(&content).build();
        b.add_css_class("flat");
        b.add_css_class("app-icon-choice");
        if v.key == current {
            b.add_css_class("current");
        }
        row.insert(&b, -1);
        if let Some(cell) = row.last_child() {
            cell.set_focusable(false);
        }
        buttons.push((v.key, b));
    }
    let buttons = Rc::new(buttons);
    for (key, b) in buttons.iter() {
        let (key, app, buttons) = (*key, app.clone(), buttons.clone());
        b.connect_clicked(move |_| {
            for (k, other) in buttons.iter() {
                if *k == key { other.add_css_class("current") } else { other.remove_css_class("current") }
            }
            app.settings.borrow_mut().app_icon = key.to_string();
            crate::storage::save_settings(&app.settings.borrow());
            app_icon::apply(key);
            app.refresh_app_icon();
        });
    }
    group.add(&row);
    group
}

/// Project name and project-wide auth (the top of the auth inheritance chain).
pub fn project_settings(app: &Rc<App>) {
    let (name, auth) = {
        let ws = app.ws.borrow();
        (ws.name.clone(), ws.auth.clone())
    };
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let name_row = adw::EntryRow::builder().title("Project name").text(&name).build();
    list.append(&name_row);

    let auth_title = gtk::Label::builder().label("Project Authorization").xalign(0.0).margin_top(18).build();
    auth_title.add_css_class("heading");
    let auth_desc = gtk::Label::builder()
        .label("Applied to every request whose auth is <b>Inherit</b>, unless a folder or collection sets its own. Tip: use a variable such as <tt>{{token}}</tt> so each environment can supply its own credentials.")
        .use_markup(true)
        .wrap(true)
        .xalign(0.0)
        .build();
    auth_desc.add_css_class("dim-label");
    let auth_form = AuthForm::new(false);
    auth_form.load(&auth);
    let auth_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    auth_card.add_css_class("card");
    let inner = pad(auth_form.widget());
    auth_card.append(&inner);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&icon_picker(app));
    body.append(&list);
    body.append(&auth_title);
    body.append(&auth_desc);
    body.append(&auth_card);
    let header = adw::HeaderBar::new();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    let scroll = gtk::ScrolledWindow::builder().child(&pad(&body)).hscrollbar_policy(gtk::PolicyType::Never).propagate_natural_height(true).build();
    tv.set_content(Some(&scroll));
    let dialog = adw::Dialog::builder().title("Project Settings").content_width(600).child(&tv).build();

    let app2 = app.clone();
    dialog.connect_closed(move |_| {
        let new_name = name_row.text().trim().to_string();
        app2.ws.borrow_mut().auth = auth_form.collect();
        if !new_name.is_empty() && new_name != name {
            app2.rename_project(&new_name);
        }
        app2.project_changed();
    });
    dialog.present(Some(&app.window));
}

/// Icon section of Project Settings. Changes apply immediately.
fn icon_picker(app: &Rc<App>) -> gtk::Box {
    use super::project_icon;
    use std::cell::RefCell;

    let current: Rc<RefCell<Option<ProjectIcon>>> = Rc::new(RefCell::new(app.ws.borrow().icon.clone()));

    let preview = project_icon::image(current.borrow().as_ref(), 48);
    let preview_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    preview_box.add_css_class("project-icon-preview");
    preview_box.set_valign(gtk::Align::Start);
    preview_box.append(&preview);

    let grid = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(12).min_children_per_line(6).column_spacing(4).row_spacing(4).homogeneous(true).build();
    grid.add_css_class("icon-grid");
    let mut icon_buttons: Vec<(String, gtk::Button)> = vec![];
    for name in project_icon::available() {
        let b = gtk::Button::builder().icon_name(name).tooltip_text(name.trim_end_matches("-symbolic").replace('-', " ")).build();
        b.add_css_class("flat");
        b.add_css_class("icon-choice");
        grid.insert(&b, -1);
        if let Some(cell) = grid.last_child() {
            cell.set_focusable(false);
        }
        icon_buttons.push((name.to_string(), b));
    }

    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    swatches.add_css_class("color-picker");
    let mut swatch_buttons: Vec<(String, gtk::Button)> = vec![];
    let none = gtk::Button::builder().icon_name("window-close-symbolic").tooltip_text("No color").valign(gtk::Align::Center).build();
    none.add_css_class("color-swatch");
    none.add_css_class("none-swatch");
    swatches.append(&none);
    swatch_buttons.push((String::new(), none));
    for (key, label, _) in super::colors::COLORS {
        let b = gtk::Button::builder().tooltip_text(label).valign(gtk::Align::Center).build();
        b.add_css_class("color-swatch");
        b.add_css_class(&format!("swatch-{key}"));
        swatches.append(&b);
        swatch_buttons.push((key.to_string(), b));
    }

    let choose = gtk::Button::builder().label("Choose Image…").build();
    let reset = gtk::Button::builder().label("Reset").tooltip_text("Use the default icon").build();
    reset.add_css_class("flat");
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    buttons.append(&choose);
    buttons.append(&reset);

    let hint = gtk::Label::builder().label("Pick an icon and an optional color, or use your own PNG / SVG / JPEG image.").xalign(0.0).wrap(true).build();
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");

    let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
    right.set_hexpand(true);
    right.append(&grid);
    right.append(&swatches);
    right.append(&buttons);
    right.append(&hint);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.append(&preview_box);
    row.append(&right);

    let title = gtk::Label::builder().label("Icon").xalign(0.0).build();
    title.add_css_class("heading");
    let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
    section.set_margin_bottom(12);
    section.append(&title);
    section.append(&row);

    // redraw highlight + preview, and push the icon to the project
    let icon_buttons = Rc::new(icon_buttons);
    let swatch_buttons = Rc::new(swatch_buttons);
    let render = {
        let (current, preview, icon_buttons, swatch_buttons, swatches) = (current.clone(), preview.clone(), icon_buttons.clone(), swatch_buttons.clone(), swatches.clone());
        Rc::new(move || {
            let icon = current.borrow().clone();
            project_icon::apply(&preview, icon.as_ref(), 48);
            let (sel_name, sel_color) = match &icon {
                Some(ProjectIcon::Symbolic { name, color }) => (Some(name.clone()), color.clone().unwrap_or_default()),
                Some(ProjectIcon::Image { .. }) => (None, String::new()),
                None => (Some(project_icon::DEFAULT_ICON.to_string()), String::new()),
            };
            for (n, b) in icon_buttons.iter() {
                if Some(n) == sel_name.as_ref() { b.add_css_class("current") } else { b.remove_css_class("current") }
            }
            for (k, b) in swatch_buttons.iter() {
                if *k == sel_color { b.add_css_class("current") } else { b.remove_css_class("current") }
            }
            // colors only apply to themed icons
            swatches.set_sensitive(!matches!(icon, Some(ProjectIcon::Image { .. })));
        })
    };
    let commit = {
        let (app, current, render) = (app.clone(), current.clone(), render.clone());
        Rc::new(move |icon: Option<ProjectIcon>| {
            *current.borrow_mut() = icon.clone();
            app.ws.borrow_mut().icon = icon;
            app.project_changed();
            render();
        })
    };
    render();

    for (name, b) in icon_buttons.iter() {
        let (name, current, commit) = (name.clone(), current.clone(), commit.clone());
        b.connect_clicked(move |_| {
            let color = match &*current.borrow() {
                Some(ProjectIcon::Symbolic { color, .. }) => color.clone(),
                _ => None,
            };
            commit(if name == project_icon::DEFAULT_ICON && color.is_none() { None } else { Some(ProjectIcon::Symbolic { name: name.clone(), color }) });
        });
    }
    for (key, b) in swatch_buttons.iter() {
        let (key, current, commit) = (key.clone(), current.clone(), commit.clone());
        b.connect_clicked(move |_| {
            let name = match &*current.borrow() {
                Some(ProjectIcon::Symbolic { name, .. }) => name.clone(),
                _ => project_icon::DEFAULT_ICON.to_string(),
            };
            let color = (!key.is_empty()).then(|| key.clone());
            commit(if name == project_icon::DEFAULT_ICON && color.is_none() { None } else { Some(ProjectIcon::Symbolic { name, color }) });
        });
    }
    let c = commit.clone();
    reset.connect_clicked(move |_| c(None));
    let (app2, c) = (app.clone(), commit.clone());
    choose.connect_clicked(move |btn| {
        let (app, commit) = (app2.clone(), c.clone());
        let root = btn.root().and_downcast::<gtk::Window>();
        gtk::glib::spawn_future_local(async move {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for pattern in ["*.png", "*.svg", "*.jpg", "*.jpeg", "*.webp"] {
                filter.add_pattern(pattern);
            }
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Choose Project Icon").filters(&filters).build();
            let Ok(file) = dialog.open_future(root.as_ref()).await else { return };
            let Some(path) = file.path() else { return };
            if gtk::gdk::Texture::from_filename(&path).is_err() {
                app.toast("That file isn't an image Pigeon can display");
                return;
            }
            let project_id = app.ws.borrow().id.clone();
            match crate::storage::store_project_image(&project_id, &path) {
                Ok(stored) => commit(Some(ProjectIcon::Image { path: stored.to_string_lossy().into_owned() })),
                Err(e) => app.toast(&format!("Couldn't copy image: {e}")),
            }
        });
    });
    section
}

fn pad(w: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_margin_start(16);
    b.set_margin_end(16);
    b.set_margin_top(12);
    b.set_margin_bottom(16);
    b.append(w);
    b
}

pub fn shortcuts(parent: &impl IsA<gtk::Widget>) {
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    for (keys, what) in [
        ("Ctrl+Enter", "Send request"),
        ("Ctrl+S", "Save request"),
        ("Ctrl+T", "New request tab"),
        ("Ctrl+W", "Close tab"),
        ("Ctrl+L", "Focus URL bar"),
        ("Ctrl+O", "Import collection / environment"),
        ("Ctrl+E", "Manage environments"),
        ("Ctrl+Shift+C", "Generate code"),
        ("Ctrl+Page Up/Down", "Switch tabs"),
        ("F9", "Toggle sidebar"),
        ("Ctrl+,", "Preferences"),
    ] {
        let row = adw::ActionRow::builder().title(what).build();
        let k = gtk::Label::new(Some(keys));
        k.add_css_class("keycap");
        k.set_valign(gtk::Align::Center);
        row.add_suffix(&k);
        list.append(&row);
    }
    let body = pad(&list);
    let header = adw::HeaderBar::new();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&body));
    adw::Dialog::builder().title("Keyboard Shortcuts").content_width(420).child(&tv).build().present(Some(parent));
}
