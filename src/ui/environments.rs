//! Environment & globals manager dialog.

use super::kv_editor::{KvEditor, Row};
use super::window::App;
use crate::model::*;
use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;

/// `None` selects the globals.
type Selection = Option<String>;

struct State {
    app: Rc<App>,
    list: gtk::ListBox,
    ids: RefCell<Vec<Selection>>,
    current: RefCell<Selection>,
    editor: Rc<KvEditor>,
    name: gtk::Entry,
    actions: gtk::Box,
    active: gtk::Button,
}

pub fn show(app: &Rc<App>) {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    let add = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("New environment").build();
    add.add_css_class("flat");
    let side_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let l = gtk::Label::builder().label("Environments").xalign(0.0).hexpand(true).build();
    l.add_css_class("heading");
    side_header.append(&l);
    side_header.append(&add);
    side_header.set_margin_start(12);
    side_header.set_margin_end(6);
    side_header.set_margin_top(6);
    let side = gtk::Box::new(gtk::Orientation::Vertical, 6);
    side.append(&side_header);
    side.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
    side.set_width_request(220);

    let name = gtk::Entry::builder().hexpand(true).build();
    name.add_css_class("title-4");
    let active = gtk::Button::with_label("Set Active");
    let dup = gtk::Button::builder().icon_name("edit-copy-symbolic").tooltip_text("Duplicate").build();
    let export = gtk::Button::builder().icon_name("document-send-symbolic").tooltip_text("Export as JSON").build();
    let delete = gtk::Button::builder().icon_name("user-trash-symbolic").tooltip_text("Delete").build();
    delete.add_css_class("destructive-action");
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for b in [&active, &dup, &export, &delete] {
        actions.append(b);
    }
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.append(&name);
    top.append(&actions);

    let editor = KvEditor::new(false, "Variable", "Value");
    let hint = gtk::Label::builder()
        .label("Use variables as <tt>{{name}}</tt> in URLs, headers, bodies and auth. Priority: environment › collection › globals. Built-ins: <tt>{{$guid}}</tt> <tt>{{$timestamp}}</tt> <tt>{{$isoTimestamp}}</tt> <tt>{{$randomInt}}</tt>")
        .use_markup(true)
        .wrap(true)
        .xalign(0.0)
        .build();
    hint.add_css_class("dim-label");
    hint.add_css_class("caption");
    let main = gtk::Box::new(gtk::Orientation::Vertical, 10);
    main.set_margin_start(12);
    main.set_margin_end(12);
    main.set_margin_top(12);
    main.set_margin_bottom(12);
    main.append(&top);
    main.append(editor.widget());
    main.append(&hint);
    main.set_hexpand(true);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&side);
    body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    body.append(&main);

    let header = adw::HeaderBar::new();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&body));
    let dialog = adw::Dialog::builder().title("Environments").content_width(900).content_height(560).child(&tv).build();

    let st = Rc::new(State { app: app.clone(), list, ids: RefCell::new(vec![]), current: RefCell::new(None), editor, name, actions, active });

    let initial = app.ws.borrow().active_environment.clone();
    st.rebuild_list();
    st.select(initial);

    let s = st.clone();
    st.list.connect_row_selected(move |_, row| {
        let Some(row) = row else { return };
        let sel = s.ids.borrow().get(row.index() as usize).cloned().flatten();
        if *s.current.borrow() != sel {
            s.commit();
            s.load(sel);
        }
    });
    let s = st.clone();
    add.connect_clicked(move |_| {
        s.commit();
        let env = Environment { id: new_id(), name: "New Environment".into(), variables: vec![] };
        let id = env.id.clone();
        s.app.ws.borrow_mut().environments.push(env);
        s.rebuild_list();
        s.select(Some(id));
        s.name.grab_focus();
        s.app.environments_changed();
    });
    let s = st.clone();
    st.name.connect_changed(move |e| {
        let Some(id) = s.current.borrow().clone() else { return };
        let text = e.text().to_string();
        if let Some(env) = s.app.ws.borrow_mut().environments.iter_mut().find(|x| x.id == id) {
            env.name = text.clone();
        }
        let idx = s.ids.borrow().iter().position(|i| i.as_deref() == Some(&id));
        if let Some(row) = idx.and_then(|i| s.list.row_at_index(i as i32)) {
            if let Some(label) = row.child().and_downcast::<gtk::Label>() {
                label.set_text(&text);
            }
        }
    });
    let s = st.clone();
    st.active.connect_clicked(move |_| {
        s.commit();
        let id = s.current.borrow().clone();
        s.app.ws.borrow_mut().active_environment = id;
        s.app.environments_changed();
        s.update_active_button();
    });
    let s = st.clone();
    dup.connect_clicked(move |_| {
        s.commit();
        let Some(id) = s.current.borrow().clone() else { return };
        let copy = s.app.ws.borrow().environments.iter().find(|e| e.id == id).cloned();
        if let Some(mut e) = copy {
            e.id = new_id();
            e.name = format!("{} Copy", e.name);
            let nid = e.id.clone();
            s.app.ws.borrow_mut().environments.push(e);
            s.rebuild_list();
            s.select(Some(nid));
            s.app.environments_changed();
        }
    });
    let s = st.clone();
    delete.connect_clicked(move |_| {
        let Some(id) = s.current.borrow().clone() else { return };
        {
            let mut ws = s.app.ws.borrow_mut();
            ws.environments.retain(|e| e.id != id);
            if ws.active_environment.as_deref() == Some(&id) {
                ws.active_environment = None;
            }
        }
        *s.current.borrow_mut() = None;
        s.rebuild_list();
        s.select(None);
        s.app.environments_changed();
    });
    let s = st.clone();
    export.connect_clicked(move |_| {
        s.commit();
        let Some(id) = s.current.borrow().clone() else { return };
        let Some(env) = s.app.ws.borrow().environments.iter().find(|e| e.id == id).cloned() else { return };
        let app = s.app.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::builder().title("Export Environment").initial_name(format!("{}.environment.json", env.name)).build();
            if let Ok(file) = dialog.save_future(Some(&app.window)).await {
                if let Some(path) = file.path() {
                    match std::fs::write(&path, crate::interchange::export_environment(&env)) {
                        Ok(_) => app.toast("Environment exported"),
                        Err(e) => app.toast(&format!("Export failed: {e}")),
                    }
                }
            }
        });
    });

    let s = st.clone();
    dialog.connect_closed(move |_| {
        s.commit();
        s.app.environments_changed();
    });
    dialog.present(Some(&app.window));
}

impl State {
    fn rebuild_list(&self) {
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        let mut ids = vec![None];
        let globals = gtk::Label::builder().label("Globals").xalign(0.0).build();
        globals.add_css_class("heading");
        self.list.append(&globals);
        for e in &self.app.ws.borrow().environments {
            let l = gtk::Label::builder().label(&e.name).xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build();
            self.list.append(&l);
            ids.push(Some(e.id.clone()));
        }
        *self.ids.borrow_mut() = ids;
    }

    fn select(&self, sel: Selection) {
        let idx = self.ids.borrow().iter().position(|i| *i == sel).unwrap_or(0);
        self.load(self.ids.borrow()[idx].clone());
        if let Some(row) = self.list.row_at_index(idx as i32) {
            self.list.select_row(Some(&row));
        }
    }

    fn load(&self, sel: Selection) {
        let ws = self.app.ws.borrow();
        let (name, vars) = match &sel {
            None => ("Globals".to_string(), ws.globals.clone()),
            Some(id) => match ws.environments.iter().find(|e| &e.id == id) {
                Some(e) => (e.name.clone(), e.variables.clone()),
                None => ("Globals".to_string(), ws.globals.clone()),
            },
        };
        drop(ws);
        *self.current.borrow_mut() = sel.clone();
        self.name.set_text(&name);
        self.name.set_sensitive(sel.is_some());
        self.actions.set_sensitive(sel.is_some());
        self.editor.set_items(&vars.iter().map(Row::from).collect::<Vec<_>>());
        self.update_active_button();
    }

    fn update_active_button(&self) {
        let is_active = self.current.borrow().is_some() && *self.current.borrow() == self.app.ws.borrow().active_environment;
        self.active.set_label(if is_active { "Active" } else { "Set Active" });
        self.active.set_sensitive(!is_active);
        if is_active {
            self.active.remove_css_class("suggested-action");
        } else {
            self.active.add_css_class("suggested-action");
        }
    }

    /// Writes the editor contents back into the workspace.
    fn commit(&self) {
        let vars = self.editor.kvs();
        let mut ws = self.app.ws.borrow_mut();
        match self.current.borrow().as_ref() {
            None => ws.globals = vars,
            Some(id) => {
                if let Some(e) = ws.environments.iter_mut().find(|e| &e.id == id) {
                    e.variables = vars;
                }
            }
        }
    }
}
