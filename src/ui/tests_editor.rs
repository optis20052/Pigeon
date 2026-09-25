//! Editor for declarative response assertions.

use crate::model::{AssertOp, Assertion};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

struct RowW {
    row: gtk::ListBoxRow,
    check: gtk::CheckButton,
    source: gtk::Entry,
    op: gtk::DropDown,
    expected: gtk::Entry,
}

pub struct TestsEditor {
    root: gtk::Box,
    list: gtk::ListBox,
    rows: RefCell<Vec<RowW>>,
    on_change: RefCell<Vec<Box<dyn Fn()>>>,
    suppress: Cell<bool>,
    this: RefCell<Weak<TestsEditor>>,
}

impl TestsEditor {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let help = gtk::Label::builder()
            .label("Assert on <tt>status</tt>, <tt>time</tt> (ms), <tt>body</tt>, <tt>header.Name</tt> or <tt>json.path[0].to.value</tt>")
            .use_markup(true)
            .xalign(0.0)
            .wrap(true)
            .build();
        help.add_css_class("dim-label");
        root.append(&help);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("kv-list");
        let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        root.append(&scroll);

        let add = gtk::Button::builder().label("Add Test").halign(gtk::Align::Start).build();
        add.add_css_class("flat");
        root.append(&add);

        let this = Rc::new(Self {
            root,
            list,
            rows: RefCell::new(vec![]),
            on_change: RefCell::new(vec![]),
            suppress: Cell::new(false),
            this: RefCell::new(Weak::new()),
        });
        *this.this.borrow_mut() = Rc::downgrade(&this);
        let weak = Rc::downgrade(&this);
        add.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.add_row(&Assertion { enabled: true, source: "status".into(), op: AssertOp::Equals, expected: "200".into() });
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
        if !self.suppress.get() {
            for f in self.on_change.borrow().iter() {
                f();
            }
        }
    }

    pub fn items(&self) -> Vec<Assertion> {
        self.rows
            .borrow()
            .iter()
            .map(|r| Assertion {
                enabled: r.check.is_active(),
                source: r.source.text().to_string(),
                op: AssertOp::ALL[r.op.selected() as usize],
                expected: r.expected.text().to_string(),
            })
            .collect()
    }

    pub fn set_items(&self, items: &[Assertion]) {
        if self.items() == items {
            return;
        }
        self.suppress.set(true);
        for r in self.rows.borrow_mut().drain(..) {
            self.list.remove(&r.row);
        }
        for a in items {
            self.add_row(a);
        }
        self.suppress.set(false);
    }

    fn add_row(&self, a: &Assertion) {
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hbox.set_margin_top(2);
        hbox.set_margin_bottom(2);
        hbox.set_margin_start(4);
        hbox.set_margin_end(4);
        let check = gtk::CheckButton::new();
        check.set_active(a.enabled);
        let source = gtk::Entry::builder().text(&a.source).placeholder_text("json.data.id").hexpand(true).build();
        let labels: Vec<&str> = AssertOp::ALL.iter().map(|o| o.label()).collect();
        let op = super::util::dropdown(&labels);
        op.set_selected(AssertOp::ALL.iter().position(|o| *o == a.op).unwrap_or(0) as u32);
        let expected = gtk::Entry::builder().text(&a.expected).placeholder_text("expected").hexpand(true).build();
        let delete = gtk::Button::from_icon_name("edit-delete-symbolic");
        delete.add_css_class("flat");
        for w in [check.upcast_ref::<gtk::Widget>(), source.upcast_ref(), op.upcast_ref(), expected.upcast_ref(), delete.upcast_ref()] {
            hbox.append(w);
        }
        let row = gtk::ListBoxRow::builder().child(&hbox).activatable(false).build();
        self.list.append(&row);

        let weak = self.this.borrow().clone();
        let emit = move || {
            if let Some(t) = weak.upgrade() {
                t.emit();
            }
        };
        let e = emit.clone();
        check.connect_toggled(move |_| e());
        let e = emit.clone();
        source.connect_changed(move |_| e());
        let e = emit.clone();
        expected.connect_changed(move |_| e());
        let e = emit;
        let exp = expected.clone();
        op.connect_selected_notify(move |dd| {
            exp.set_sensitive(AssertOp::ALL[dd.selected() as usize] != AssertOp::Exists);
            e();
        });
        expected.set_sensitive(a.op != AssertOp::Exists);

        let weak = self.this.borrow().clone();
        let row2 = row.clone();
        delete.connect_clicked(move |_| {
            let Some(t) = weak.upgrade() else { return };
            let pos = t.rows.borrow().iter().position(|r| r.row == row2);
            if let Some(pos) = pos {
                let r = t.rows.borrow_mut().remove(pos);
                t.list.remove(&r.row);
                t.emit();
            }
        });
        self.rows.borrow_mut().push(RowW { row, check, source, op, expected });
    }
}
