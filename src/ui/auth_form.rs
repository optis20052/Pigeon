//! Authorization settings form, shared by requests, folders and collections.

use super::util::{dropdown, labeled_row};
use crate::model::{ApiKeyLocation, Auth};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const AUTH_TYPES: [&str; 5] = ["Inherit from parent", "No Auth", "Bearer Token", "Basic Auth", "API Key"];

pub struct AuthForm {
    root: gtk::Box,
    kind: gtk::DropDown,
    /// 0 when "Inherit" is offered, 1 when the list starts at "No Auth".
    offset: u32,
    inherit_hint: gtk::Label,
    stack: gtk::Stack,
    bearer_token: gtk::Entry,
    basic_user: gtk::Entry,
    basic_pass: gtk::PasswordEntry,
    apikey_key: gtk::Entry,
    apikey_value: gtk::Entry,
    apikey_in: gtk::DropDown,
    on_change: RefCell<Vec<Box<dyn Fn()>>>,
    suppress: Cell<bool>,
}

impl AuthForm {
    /// `allow_inherit` is false for the top of the chain (project auth).
    pub fn new(allow_inherit: bool) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let offset = if allow_inherit { 0 } else { 1 };
        let kind = dropdown(&AUTH_TYPES[offset as usize..]);
        kind.set_halign(gtk::Align::Start);
        root.append(&labeled_row("Type", &kind));

        let stack = gtk::Stack::new();
        stack.set_vhomogeneous(false);
        let inherit_hint = gtk::Label::builder().label("Uses the auth of the parent folder, collection or project.").xalign(0.0).wrap(true).build();
        inherit_hint.add_css_class("dim-label");
        stack.add_named(&inherit_hint, Some("0"));
        let noauth = gtk::Label::builder().label("No authorization is sent.").xalign(0.0).wrap(true).build();
        noauth.add_css_class("dim-label");
        stack.add_named(&noauth, Some("1"));

        let bearer_token = gtk::Entry::builder().placeholder_text("Token or {{variable}}").build();
        stack.add_named(&labeled_row("Token", &bearer_token), Some("2"));

        let basic = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let basic_user = gtk::Entry::builder().placeholder_text("Username").build();
        let basic_pass = gtk::PasswordEntry::builder().placeholder_text("Password").show_peek_icon(true).build();
        basic.append(&labeled_row("Username", &basic_user));
        basic.append(&labeled_row("Password", &basic_pass));
        stack.add_named(&basic, Some("3"));

        let apikey = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let apikey_key = gtk::Entry::builder().placeholder_text("X-API-Key").build();
        let apikey_value = gtk::Entry::builder().placeholder_text("Value").build();
        let apikey_in = dropdown(&["Header", "Query Params"]);
        apikey_in.set_halign(gtk::Align::Start);
        apikey.append(&labeled_row("Key", &apikey_key));
        apikey.append(&labeled_row("Value", &apikey_value));
        apikey.append(&labeled_row("Add to", &apikey_in));
        stack.add_named(&apikey, Some("4"));
        root.append(&stack);

        let this = Rc::new(Self {
            root,
            kind,
            offset,
            inherit_hint,
            stack,
            bearer_token,
            basic_user,
            basic_pass,
            apikey_key,
            apikey_value,
            apikey_in,
            on_change: RefCell::new(vec![]),
            suppress: Cell::new(false),
        });

        let weak = Rc::downgrade(&this);
        let changed = move || {
            if let Some(t) = weak.upgrade() {
                if !t.suppress.get() {
                    for f in t.on_change.borrow().iter() {
                        f();
                    }
                }
            }
        };
        let stack = this.stack.clone();
        let c = changed.clone();
        let offset = this.offset;
        this.kind.connect_selected_notify(move |dd| {
            stack.set_visible_child_name(&(dd.selected() + offset).to_string());
            c();
        });
        for e in [&this.bearer_token, &this.basic_user, &this.apikey_key, &this.apikey_value] {
            let c = changed.clone();
            e.connect_changed(move |_| c());
        }
        let c = changed.clone();
        this.basic_pass.connect_changed(move |_| c());
        let c = changed;
        this.apikey_in.connect_selected_notify(move |_| c());
        this
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_change.borrow_mut().push(Box::new(f));
    }

    /// Explains what "Inherit" currently resolves to.
    pub fn set_inherit_hint(&self, text: &str) {
        self.inherit_hint.set_markup(text);
    }

    pub fn load(&self, auth: &Auth) {
        self.suppress.set(true);
        let index = auth.index().max(self.offset);
        self.kind.set_selected(index - self.offset);
        self.stack.set_visible_child_name(&index.to_string());
        match auth {
            Auth::Bearer { token } => self.bearer_token.set_text(token),
            Auth::Basic { username, password } => {
                self.basic_user.set_text(username);
                self.basic_pass.set_text(password);
            }
            Auth::ApiKey { key, value, location } => {
                self.apikey_key.set_text(key);
                self.apikey_value.set_text(value);
                self.apikey_in.set_selected(if *location == ApiKeyLocation::Query { 1 } else { 0 });
            }
            _ => {}
        }
        self.suppress.set(false);
    }

    pub fn collect(&self) -> Auth {
        match self.kind.selected() + self.offset {
            0 => Auth::Inherit,
            2 => Auth::Bearer { token: self.bearer_token.text().into() },
            3 => Auth::Basic { username: self.basic_user.text().into(), password: self.basic_pass.text().into() },
            4 => Auth::ApiKey {
                key: self.apikey_key.text().into(),
                value: self.apikey_value.text().into(),
                location: if self.apikey_in.selected() == 1 { ApiKeyLocation::Query } else { ApiKeyLocation::Header },
            },
            _ => Auth::None,
        }
    }
}
