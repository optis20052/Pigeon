//! Editable key/value table with an always-present empty row and a bulk-edit mode.

use crate::model::{FormField, KeyValue};
use gtk::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub enabled: bool,
    pub key: String,
    pub value: String,
    pub is_file: bool,
}

impl From<&KeyValue> for Row {
    fn from(kv: &KeyValue) -> Self {
        Row { enabled: kv.enabled, key: kv.key.clone(), value: kv.value.clone(), is_file: false }
    }
}

impl From<&FormField> for Row {
    fn from(f: &FormField) -> Self {
        Row { enabled: f.enabled, key: f.key.clone(), value: f.value.clone(), is_file: f.is_file }
    }
}

impl Row {
    pub fn to_kv(&self) -> KeyValue {
        KeyValue { enabled: self.enabled, key: self.key.clone(), value: self.value.clone(), description: String::new() }
    }

    pub fn to_field(&self) -> FormField {
        FormField { enabled: self.enabled, key: self.key.clone(), value: self.value.clone(), is_file: self.is_file }
    }
}

struct RowWidgets {
    row: gtk::ListBoxRow,
    check: gtk::CheckButton,
    key: gtk::Entry,
    value: gtk::Entry,
    kind: Option<gtk::DropDown>,
    delete: gtk::Button,
}

pub struct KvEditor {
    root: gtk::Box,
    stack: gtk::Stack,
    list: gtk::ListBox,
    bulk: gtk::TextView,
    rows: RefCell<Vec<RowWidgets>>,
    on_change: RefCell<Vec<Box<dyn Fn()>>>,
    file_mode: bool,
    suppress: Cell<bool>,
    /// Keys are dictated elsewhere (e.g. path variables from the URL): no add/remove/bulk edit.
    fixed: Cell<bool>,
    toolbar: gtk::Box,
    placeholders: (String, String),
    this: RefCell<Weak<KvEditor>>,
}

impl KvEditor {
    pub fn new(file_mode: bool, key_ph: &str, value_ph: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        let bulk_btn = gtk::ToggleButton::with_label("Bulk Edit");
        bulk_btn.add_css_class("flat");
        bulk_btn.set_tooltip_text(Some("Edit as text, one \"key: value\" per line; prefix // to disable"));
        toolbar.append(&bulk_btn);
        root.append(&toolbar);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("kv-list");
        let list_scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        let (bulk_scroll, bulk) = super::code_view::new_view(true);

        let stack = gtk::Stack::new();
        stack.add_named(&list_scroll, Some("table"));
        stack.add_named(&bulk_scroll, Some("bulk"));
        stack.set_vexpand(true);
        root.append(&stack);

        let this = Rc::new(Self {
            root,
            stack,
            list,
            bulk,
            rows: RefCell::new(vec![]),
            on_change: RefCell::new(vec![]),
            file_mode,
            suppress: Cell::new(false),
            fixed: Cell::new(false),
            toolbar: toolbar.clone(),
            placeholders: (key_ph.into(), value_ph.into()),
            this: RefCell::new(Weak::new()),
        });
        *this.this.borrow_mut() = Rc::downgrade(&this);
        this.add_row(&Row { enabled: true, ..Default::default() });

        let weak = Rc::downgrade(&this);
        bulk_btn.connect_toggled(move |b| {
            let Some(this) = weak.upgrade() else { return };
            if b.is_active() {
                this.bulk.buffer().set_text(&to_bulk(&this.items()));
                this.stack.set_visible_child_name("bulk");
            } else {
                let rows = from_bulk(&super::util::text_of(&this.bulk.buffer()));
                this.set_items(&rows);
                this.stack.set_visible_child_name("table");
                this.emit();
            }
        });
        let weak = Rc::downgrade(&this);
        this.bulk.buffer().connect_changed(move |buf| {
            let Some(this) = weak.upgrade() else { return };
            if this.stack.visible_child_name().as_deref() == Some("bulk") && !this.suppress.get() {
                // keep the table in sync so readers of items() see the bulk text
                let rows = from_bulk(&super::util::text_of(buf));
                this.suppress.set(true);
                this.rebuild(&rows);
                this.suppress.set(false);
                this.emit();
            }
        });
        this
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_change.borrow_mut().push(Box::new(f));
    }

    fn emit(&self) {
        if self.suppress.get() {
            return;
        }
        for f in self.on_change.borrow().iter() {
            f();
        }
    }

    /// Current rows, excluding the trailing empty placeholder row(s).
    pub fn items(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self
            .rows
            .borrow()
            .iter()
            .map(|w| Row {
                enabled: w.check.is_active(),
                key: w.key.text().to_string(),
                value: w.value.text().to_string(),
                is_file: w.kind.as_ref().map(|k| k.selected() == 1).unwrap_or(false),
            })
            .collect();
        while rows.last().is_some_and(|r| r.key.is_empty() && r.value.is_empty()) {
            rows.pop();
        }
        rows
    }

    pub fn kvs(&self) -> Vec<KeyValue> {
        self.items().iter().map(Row::to_kv).collect()
    }

    pub fn fields(&self) -> Vec<FormField> {
        self.items().iter().map(Row::to_field).collect()
    }

    /// Replaces the contents without emitting change notifications.
    pub fn set_items(&self, items: &[Row]) {
        if self.items() == items {
            return;
        }
        self.suppress.set(true);
        self.rebuild(items);
        if self.stack.visible_child_name().as_deref() == Some("bulk") {
            self.bulk.buffer().set_text(&to_bulk(items));
        }
        self.suppress.set(false);
    }

    /// Switches to fixed-keys mode: keys are read-only and rows can't be added or removed.
    pub fn set_fixed_keys(&self) {
        self.fixed.set(true);
        self.toolbar.set_visible(false);
        self.stack.set_vexpand(false);
        let items = self.items();
        self.suppress.set(true);
        self.rebuild(&items);
        self.suppress.set(false);
    }

    fn rebuild(&self, items: &[Row]) {
        for w in self.rows.borrow_mut().drain(..) {
            self.list.remove(&w.row);
        }
        for r in items {
            self.add_row(r);
        }
        if !self.fixed.get() {
            self.add_row(&Row { enabled: true, ..Default::default() });
        }
    }

    fn add_row(&self, data: &Row) {
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hbox.set_margin_top(2);
        hbox.set_margin_bottom(2);
        hbox.set_margin_start(4);
        hbox.set_margin_end(4);

        let check = gtk::CheckButton::new();
        check.set_active(data.enabled);
        let key = gtk::Entry::builder().placeholder_text(&self.placeholders.0).text(&data.key).hexpand(true).build();
        let value = gtk::Entry::builder().placeholder_text(&self.placeholders.1).text(&data.value).hexpand(true).build();
        key.set_width_chars(12);
        value.set_width_chars(12);
        let delete = gtk::Button::from_icon_name("edit-delete-symbolic");
        delete.add_css_class("flat");
        delete.set_tooltip_text(Some("Remove"));

        hbox.append(&check);
        hbox.append(&key);

        let kind = if self.file_mode {
            let dd = super::util::dropdown(&["Text", "File"]);
            dd.set_selected(if data.is_file { 1 } else { 0 });
            hbox.append(&dd);
            Some(dd)
        } else {
            None
        };
        hbox.append(&value);

        if let Some(dd) = &kind {
            let pick = gtk::Button::from_icon_name("document-open-symbolic");
            pick.add_css_class("flat");
            pick.set_tooltip_text(Some("Choose file"));
            pick.set_visible(data.is_file);
            let entry = value.clone();
            pick.connect_clicked(move |btn| {
                let entry = entry.clone();
                let root = btn.root().and_downcast::<gtk::Window>();
                glib::spawn_future_local(async move {
                    let dialog = gtk::FileDialog::builder().title("Choose File").build();
                    if let Ok(file) = dialog.open_future(root.as_ref()).await {
                        if let Some(path) = file.path() {
                            entry.set_text(&path.to_string_lossy());
                        }
                    }
                });
            });
            let pick2 = pick.clone();
            let weak = self.this.borrow().clone();
            dd.connect_selected_notify(move |dd| {
                pick2.set_visible(dd.selected() == 1);
                if let Some(this) = weak.upgrade() {
                    this.emit();
                }
            });
            hbox.append(&pick);
        }
        hbox.append(&delete);

        let row = gtk::ListBoxRow::builder().child(&hbox).activatable(false).build();
        self.list.append(&row);

        let weak = self.this.borrow().clone();
        let on_edit = move || {
            if let Some(this) = weak.upgrade() {
                this.ensure_trailing_row();
                this.emit();
            }
        };
        let f = on_edit.clone();
        key.connect_changed(move |_| f());
        let f = on_edit.clone();
        value.connect_changed(move |_| f());
        let f = on_edit;
        check.connect_toggled(move |_| f());

        let weak = self.this.borrow().clone();
        let row2 = row.clone();
        delete.connect_clicked(move |_| {
            let Some(this) = weak.upgrade() else { return };
            let pos = this.rows.borrow().iter().position(|w| w.row == row2);
            if let Some(pos) = pos {
                let w = this.rows.borrow_mut().remove(pos);
                this.list.remove(&w.row);
                this.ensure_trailing_row();
                this.emit();
            }
        });

        self.rows.borrow_mut().push(RowWidgets { row, check, key, value, kind, delete });
        self.update_delete_visibility();
    }

    fn ensure_trailing_row(&self) {
        if self.fixed.get() {
            return;
        }
        let needs = self
            .rows
            .borrow()
            .last()
            .map(|w| !w.key.text().is_empty() || !w.value.text().is_empty())
            .unwrap_or(true);
        if needs {
            self.add_row(&Row { enabled: true, ..Default::default() });
        }
        self.update_delete_visibility();
    }

    fn update_delete_visibility(&self) {
        let rows = self.rows.borrow();
        let n = rows.len();
        for (i, w) in rows.iter().enumerate() {
            if self.fixed.get() {
                w.delete.set_visible(false);
                w.key.set_editable(false);
                w.key.set_can_focus(false);
                continue;
            }
            let last = i + 1 == n;
            w.delete.set_opacity(if last { 0.0 } else { 1.0 });
            w.delete.set_sensitive(!last);
            w.check.set_opacity(if last { 0.4 } else { 1.0 });
        }
    }
}

fn to_bulk(rows: &[Row]) -> String {
    rows.iter()
        .map(|r| {
            let prefix = if r.enabled { "" } else { "//" };
            let file = if r.is_file { "@" } else { "" };
            format!("{prefix}{}:{file}{}", r.key, r.value)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn from_bulk(text: &str) -> Vec<Row> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (enabled, l) = match l.trim_start().strip_prefix("//") {
                Some(rest) => (false, rest),
                None => (true, l),
            };
            let (k, v) = l.split_once(':').unwrap_or((l, ""));
            let v = v.trim();
            let (is_file, v) = match v.strip_prefix('@') {
                Some(rest) => (true, rest),
                None => (false, v),
            };
            Row { enabled, key: k.trim().into(), value: v.into(), is_file }
        })
        .collect()
}
