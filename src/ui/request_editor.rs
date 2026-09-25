//! The request editor shown in each tab: URL bar, request config tabs and the response pane.

use super::auth_form::AuthForm;
use super::code_view::{self, Lang};
use super::kv_editor::{KvEditor, Row};
use super::response_view::ResponseView;
use super::tests_editor::TestsEditor;
use super::util::{self, dropdown, text_of};
use crate::model::*;
use crate::vars::Scope;
use adw::prelude::*;
use gtk::{glib, pango};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type Callback = Box<dyn Fn()>;

pub struct RequestEditor {
    root: gtk::Box,
    base: RefCell<Request>,

    method: gtk::DropDown,
    url: gtk::Entry,
    send: gtk::Button,
    save: gtk::Button,
    more: gtk::MenuButton,

    params: Rc<KvEditor>,
    path_vars: Rc<KvEditor>,
    path_box: gtk::Box,
    headers: Rc<KvEditor>,
    params_page: adw::ViewStackPage,
    headers_page: adw::ViewStackPage,
    body_page: adw::ViewStackPage,

    body_mode: gtk::DropDown,
    body_stack: gtk::Stack,
    raw_lang: gtk::DropDown,
    raw_view: gtk::TextView,
    beautify: gtk::Button,
    urlencoded: Rc<KvEditor>,
    formdata: Rc<KvEditor>,
    binary_path: gtk::Entry,
    gql_query: gtk::TextView,
    gql_vars: gtk::TextView,

    auth: Rc<AuthForm>,

    follow_redirects: adw::SwitchRow,
    verify_tls: adw::SwitchRow,
    timeout: adw::SpinRow,

    tests: Rc<TestsEditor>,
    description: gtk::TextView,

    pub response: Rc<ResponseView>,

    loading: Cell<bool>,
    suppress: Cell<bool>,
    on_change: RefCell<Vec<Callback>>,
    on_send: RefCell<Vec<Callback>>,
    on_save: RefCell<Vec<Callback>>,
    on_curl_pasted: RefCell<Vec<Box<dyn Fn(Request)>>>,
    scope: RefCell<Box<dyn Fn() -> Scope>>,
}

const BODY_MODES: [&str; 6] = ["none", "raw", "x-www-form-urlencoded", "form-data", "binary", "GraphQL"];

fn padded(child: &impl IsA<gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_start(12);
    b.set_margin_end(12);
    b.set_margin_top(6);
    b.set_margin_bottom(6);
    b.append(child);
    b
}

impl RequestEditor {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // ------------------------------------------------ URL bar
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        bar.add_css_class("url-bar");
        bar.add_css_class("linked");
        bar.set_margin_start(12);
        bar.set_margin_end(12);
        bar.set_margin_top(12);
        bar.set_margin_bottom(6);

        let methods: Vec<&str> = Method::ALL.iter().map(|m| m.as_str()).collect();
        let method = dropdown(&methods);
        method.add_css_class("method-dropdown");
        let url = gtk::Entry::builder().placeholder_text("Enter URL or paste a cURL command").hexpand(true).build();
        url.add_css_class("url-entry");
        let send = gtk::Button::with_label("Send");
        send.add_css_class("suggested-action");
        send.add_css_class("send-button");
        send.set_tooltip_text(Some("Send (Ctrl+Enter)"));
        bar.append(&method);
        bar.append(&url);
        bar.append(&send);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_margin_start(8);
        let save = gtk::Button::builder().icon_name("media-floppy-symbolic").tooltip_text("Save (Ctrl+S)").build();
        let menu = gtk::gio::Menu::new();
        menu.append(Some("Generate Code…"), Some("win.code"));
        menu.append(Some("Copy as cURL"), Some("win.copy-curl"));
        menu.append(Some("Rename…"), Some("win.rename-tab"));
        menu.append(Some("Duplicate Tab"), Some("win.duplicate-tab"));
        let more = gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&menu).tooltip_text("More").build();
        actions.append(&save);
        actions.append(&more);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        top.append(&bar);
        actions.set_margin_end(12);
        actions.set_margin_top(12);
        actions.set_margin_bottom(6);
        top.append(&actions);
        bar.set_hexpand(true);

        // ------------------------------------------------ config tabs
        let stack = adw::ViewStack::new();
        stack.set_vexpand(true);
        let switcher = adw::InlineViewSwitcher::builder().stack(&stack).halign(gtk::Align::Start).build();
        switcher.add_css_class("flat");
        switcher.set_margin_start(12);
        switcher.set_margin_bottom(4);

        let params = KvEditor::new(false, "Key", "Value");
        let path_vars = KvEditor::new(false, "Variable", "Value");
        path_vars.set_fixed_keys();
        let params_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let section = |text: &str| {
            let l = gtk::Label::builder().label(text).xalign(0.0).build();
            l.add_css_class("caption-heading");
            l.add_css_class("dim-label");
            l
        };
        params_box.append(&section("Query Params"));
        params_box.append(params.widget());
        let path_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        path_box.append(&section("Path Variables"));
        path_box.append(path_vars.widget());
        path_box.set_visible(false);
        path_box.set_margin_top(8);
        params_box.append(&path_box);
        let params_page = stack.add_titled(&padded(&params_box), Some("params"), "Params");
        let headers = KvEditor::new(false, "Header", "Value");
        let headers_page = stack.add_titled(&padded(headers.widget()), Some("headers"), "Headers");

        // body
        let body_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let body_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let body_mode = dropdown(&BODY_MODES);
        let langs: Vec<&str> = RawLanguage::ALL.iter().map(|l| l.label()).collect();
        let raw_lang = dropdown(&langs);
        let beautify = gtk::Button::with_label("Beautify");
        beautify.add_css_class("flat");
        body_bar.append(&body_mode);
        body_bar.append(&raw_lang);
        let sp = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sp.set_hexpand(true);
        body_bar.append(&sp);
        body_bar.append(&beautify);
        body_box.append(&body_bar);

        let body_stack = gtk::Stack::new();
        body_stack.set_vexpand(true);
        let none = gtk::Label::new(Some("This request does not have a body"));
        none.add_css_class("dim-label");
        body_stack.add_named(&none, Some("none"));
        let (raw_scroll, raw_view) = code_view::new_view(true);
        raw_view.set_wrap_mode(gtk::WrapMode::None);
        raw_scroll.add_css_class("card");
        body_stack.add_named(&raw_scroll, Some("raw"));
        let urlencoded = KvEditor::new(false, "Key", "Value");
        body_stack.add_named(urlencoded.widget(), Some("x-www-form-urlencoded"));
        let formdata = KvEditor::new(true, "Key", "Value");
        body_stack.add_named(formdata.widget(), Some("form-data"));

        let bin_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bin_box.set_valign(gtk::Align::Start);
        let binary_path = gtk::Entry::builder().placeholder_text("Path to file").hexpand(true).build();
        let bin_pick = gtk::Button::with_label("Select File…");
        bin_box.append(&binary_path);
        bin_box.append(&bin_pick);
        body_stack.add_named(&bin_box, Some("binary"));

        let gql = gtk::Paned::new(gtk::Orientation::Horizontal);
        let (gq_scroll, gql_query) = code_view::new_view(true);
        let (gv_scroll, gql_vars) = code_view::new_view(true);
        let gq_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let l = gtk::Label::builder().label("Query").xalign(0.0).build();
        l.add_css_class("caption-heading");
        gq_box.append(&l);
        gq_scroll.add_css_class("card");
        gq_box.append(&gq_scroll);
        let gv_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let l = gtk::Label::builder().label("Variables (JSON)").xalign(0.0).build();
        l.add_css_class("caption-heading");
        gv_box.append(&l);
        gv_scroll.add_css_class("card");
        gv_box.append(&gv_scroll);
        gql.set_start_child(Some(&gq_box));
        gql.set_end_child(Some(&gv_box));
        gql.set_position(420);
        body_stack.add_named(&gql, Some("GraphQL"));
        body_box.append(&body_stack);
        let body_page = stack.add_titled(&padded(&body_box), Some("body"), "Body");

        // auth
        let auth = AuthForm::new(true);
        let auth_scroll = gtk::ScrolledWindow::builder().child(&padded(auth.widget())).vexpand(true).build();
        stack.add_titled(&auth_scroll, Some("auth"), "Auth");

        // tests
        let tests = TestsEditor::new();
        stack.add_titled(&padded(tests.widget()), Some("tests"), "Tests");

        // settings
        let settings = gtk::ListBox::new();
        settings.add_css_class("boxed-list");
        settings.set_selection_mode(gtk::SelectionMode::None);
        settings.set_valign(gtk::Align::Start);
        let follow_redirects = adw::SwitchRow::builder().title("Follow redirects").subtitle("Automatically follow 3xx responses").build();
        let verify_tls = adw::SwitchRow::builder().title("Verify TLS certificates").subtitle("Disable for self-signed certificates").build();
        let timeout = adw::SpinRow::with_range(0.0, 600_000.0, 500.0);
        timeout.set_title("Timeout (ms)");
        timeout.set_subtitle("0 means no timeout");
        settings.append(&follow_redirects);
        settings.append(&verify_tls);
        settings.append(&timeout);
        let settings_scroll = gtk::ScrolledWindow::builder().child(&padded(&settings)).vexpand(true).build();
        stack.add_titled(&settings_scroll, Some("settings"), "Settings");

        // docs
        let (desc_scroll, description) = code_view::new_view(true);
        description.set_monospace(false);
        desc_scroll.add_css_class("card");
        stack.add_titled(&padded(&desc_scroll), Some("docs"), "Docs");

        let config = gtk::Box::new(gtk::Orientation::Vertical, 0);
        config.append(&switcher);
        // the tab contents scroll as a whole when the pane is short, keeping the switcher visible
        let config_scroll = gtk::ScrolledWindow::builder().child(&stack).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        config.append(&config_scroll);

        // ------------------------------------------------ split with response
        let response = ResponseView::new();
        let paned = gtk::Paned::new(gtk::Orientation::Vertical);
        paned.set_start_child(Some(&config));
        paned.set_end_child(Some(response.widget()));
        paned.set_resize_start_child(true);
        // Both halves have small minimums (the request tabs scroll), so neither needs clipping.
        paned.set_shrink_start_child(false);
        paned.set_shrink_end_child(false);
        paned.set_position(300);
        // Keep the split proportional when the window is resized; dragging the handle sets the ratio.
        let ratio = Rc::new(Cell::new(0.5_f64));
        let adjusting = Rc::new(Cell::new(false));
        {
            let (ratio, adjusting) = (ratio.clone(), adjusting.clone());
            paned.connect_max_position_notify(move |p| {
                adjusting.set(true);
                p.set_position((p.max_position() as f64 * ratio.get()).round() as i32);
                adjusting.set(false);
            });
        }
        paned.connect_position_notify(move |p| {
            if !adjusting.get() && p.max_position() > 0 {
                ratio.set((p.position() as f64 / p.max_position() as f64).clamp(0.1, 0.9));
            }
        });
        paned.set_vexpand(true);

        root.append(&top);
        root.append(&paned);

        let this = Rc::new(Self {
            root,
            base: RefCell::new(Request::default()),
            method,
            url,
            send,
            save,
            more,
            params,
            path_vars,
            path_box,
            headers,
            params_page,
            headers_page,
            body_page,
            body_mode,
            body_stack,
            raw_lang,
            raw_view,
            beautify,
            urlencoded,
            formdata,
            binary_path,
            gql_query,
            gql_vars,
            auth,
            follow_redirects,
            verify_tls,
            timeout,
            tests,
            description,
            response,
            loading: Cell::new(false),
            suppress: Cell::new(false),
            on_change: RefCell::new(vec![]),
            on_send: RefCell::new(vec![]),
            on_save: RefCell::new(vec![]),
            on_curl_pasted: RefCell::new(vec![]),
            scope: RefCell::new(Box::new(Scope::default)),
        });
        this.wire(&bin_pick);
        this.load(&Request::default());
        this
    }

    fn wire(self: &Rc<Self>, bin_pick: &gtk::Button) {
        let weak = Rc::downgrade(self);
        let changed = move || {
            if let Some(t) = weak.upgrade() {
                t.emit_changed();
            }
        };

        // URL <-> params sync
        let weak = Rc::downgrade(self);
        self.url.connect_changed(move |e| {
            let Some(t) = weak.upgrade() else { return };
            t.highlight_url();
            if t.suppress.get() {
                return;
            }
            let text = e.text().to_string();
            if crate::interchange::looks_like_curl(&text) {
                if let Ok(req) = crate::interchange::parse_curl(&text) {
                    let t2 = t.clone();
                    // defer: we can't rewrite the entry from inside its own changed handler
                    glib::idle_add_local_once(move || {
                        for f in t2.on_curl_pasted.borrow().iter() {
                            f(req.clone());
                        }
                    });
                    return;
                }
            }
            let disabled: Vec<Row> = t.params.items().into_iter().filter(|r| !r.enabled).collect();
            let mut rows: Vec<Row> = util::parse_query(&text).iter().map(Row::from).collect();
            rows.extend(disabled);
            t.params.set_items(&rows);
            t.sync_path_vars();
            t.emit_changed();
        });
        let weak = Rc::downgrade(self);
        self.path_vars.connect_changed(move || {
            if let Some(t) = weak.upgrade() {
                t.highlight_url();
                t.emit_changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.params.connect_changed(move || {
            let Some(t) = weak.upgrade() else { return };
            let url = util::with_query(&t.url.text(), &t.params.kvs());
            if url != t.url.text() {
                t.suppress.set(true);
                let pos = t.url.position();
                t.url.set_text(&url);
                t.url.set_position(pos);
                t.suppress.set(false);
            }
            t.emit_changed();
        });
        let weak = Rc::downgrade(self);
        self.url.connect_activate(move |_| {
            if let Some(t) = weak.upgrade() {
                t.emit_send();
            }
        });
        let weak = Rc::downgrade(self);
        self.send.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                t.emit_send();
            }
        });
        let weak = Rc::downgrade(self);
        self.save.connect_clicked(move |_| {
            if let Some(t) = weak.upgrade() {
                for f in t.on_save.borrow().iter() {
                    f();
                }
            }
        });

        let c = changed.clone();
        self.method.connect_selected_notify(move |dd| {
            let m = Method::ALL[dd.selected() as usize];
            for cls in Method::ALL.iter().map(|m| m.css_class()) {
                dd.remove_css_class(cls);
            }
            dd.add_css_class(m.css_class());
            c();
        });
        for kv in [&self.headers, &self.urlencoded, &self.formdata] {
            let c = changed.clone();
            kv.connect_changed(move || c());
        }
        let c = changed.clone();
        self.tests.connect_changed(move || c());

        // body
        let weak = Rc::downgrade(self);
        self.body_mode.connect_selected_notify(move |dd| {
            let Some(t) = weak.upgrade() else { return };
            let mode = BODY_MODES[dd.selected() as usize];
            t.body_stack.set_visible_child_name(mode);
            t.raw_lang.set_visible(mode == "raw");
            t.beautify.set_visible(mode == "raw" || mode == "GraphQL");
            t.emit_changed();
        });
        let weak = Rc::downgrade(self);
        self.raw_lang.connect_selected_notify(move |_| {
            let Some(t) = weak.upgrade() else { return };
            code_view::highlight(&t.raw_view.buffer(), t.raw_lang_hl());
            t.emit_changed();
        });
        let weak = Rc::downgrade(self);
        code_view::auto_highlight(&self.raw_view.buffer(), move || weak.upgrade().map(|t| t.raw_lang_hl()).unwrap_or(Lang::Plain));
        code_view::auto_highlight(&self.gql_vars.buffer(), || Lang::Json);
        for view in [&self.raw_view, &self.gql_query, &self.gql_vars, &self.description] {
            let c = changed.clone();
            view.buffer().connect_changed(move |_| c());
        }
        let weak = Rc::downgrade(self);
        self.beautify.connect_clicked(move |_| {
            let Some(t) = weak.upgrade() else { return };
            let (view, lang) = if BODY_MODES[t.body_mode.selected() as usize] == "GraphQL" {
                (t.gql_vars.clone(), RawLanguage::Json)
            } else {
                (t.raw_view.clone(), RawLanguage::ALL[t.raw_lang.selected() as usize])
            };
            let text = text_of(&view.buffer());
            let pretty = match lang {
                RawLanguage::Json => code_view::pretty_json(&text),
                RawLanguage::Xml | RawLanguage::Html => Some(code_view::pretty_xml(&text)),
                _ => None,
            };
            if let Some(p) = pretty {
                view.buffer().set_text(&p);
            }
        });
        let c = changed.clone();
        self.binary_path.connect_changed(move |_| c());
        let entry = self.binary_path.clone();
        bin_pick.connect_clicked(move |btn| {
            let entry = entry.clone();
            let root = btn.root().and_downcast::<gtk::Window>();
            glib::spawn_future_local(async move {
                if let Ok(file) = gtk::FileDialog::builder().title("Select File").build().open_future(root.as_ref()).await {
                    if let Some(p) = file.path() {
                        entry.set_text(&p.to_string_lossy());
                    }
                }
            });
        });

        // auth
        let c = changed.clone();
        self.auth.connect_changed(move || c());

        // settings
        let c = changed.clone();
        self.follow_redirects.connect_active_notify(move |_| c());
        let c = changed.clone();
        self.verify_tls.connect_active_notify(move |_| c());
        let c = changed;
        self.timeout.connect_value_notify(move |_| c());
    }

    fn raw_lang_hl(&self) -> Lang {
        match RawLanguage::ALL[self.raw_lang.selected() as usize] {
            RawLanguage::Json => Lang::Json,
            RawLanguage::Xml | RawLanguage::Html => Lang::Xml,
            _ => Lang::Plain,
        }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn focus_url(&self) {
        self.url.grab_focus();
    }

    pub fn set_scope_provider(&self, f: impl Fn() -> Scope + 'static) {
        *self.scope.borrow_mut() = Box::new(f);
        self.highlight_url();
    }

    pub fn set_inherit_hint(&self, markup: &str) {
        self.auth.set_inherit_hint(markup);
    }

    /// Re-evaluates variable highlighting, e.g. after the environment changed.
    pub fn refresh_scope(&self) {
        self.highlight_url();
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        self.on_change.borrow_mut().push(Box::new(f));
    }

    pub fn connect_send(&self, f: impl Fn() + 'static) {
        self.on_send.borrow_mut().push(Box::new(f));
    }

    pub fn connect_save(&self, f: impl Fn() + 'static) {
        self.on_save.borrow_mut().push(Box::new(f));
    }

    pub fn connect_curl_pasted(&self, f: impl Fn(Request) + 'static) {
        self.on_curl_pasted.borrow_mut().push(Box::new(f));
    }

    fn emit_changed(&self) {
        if self.suppress.get() {
            return;
        }
        self.update_badges();
        for f in self.on_change.borrow().iter() {
            f();
        }
    }

    fn emit_send(&self) {
        for f in self.on_send.borrow().iter() {
            f();
        }
    }

    pub fn set_loading(&self, loading: bool) {
        self.loading.set(loading);
        self.send.set_label(if loading { "Cancel" } else { "Send" });
        if loading {
            self.send.remove_css_class("suggested-action");
            self.send.add_css_class("destructive-action");
            self.response.set_loading();
        } else {
            self.send.remove_css_class("destructive-action");
            self.send.add_css_class("suggested-action");
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading.get()
    }

    /// Keeps the path-variable table in line with the `:name` segments of the URL.
    fn sync_path_vars(&self) {
        let names: Vec<String> = crate::vars::path_vars(&self.url.text()).into_iter().map(|(_, n)| n).collect();
        let current = self.path_vars.items();
        let rows: Vec<Row> = names
            .iter()
            .map(|n| current.iter().find(|r| &r.key == n).cloned().unwrap_or(Row { enabled: true, key: n.clone(), ..Default::default() }))
            .collect();
        self.path_vars.set_items(&rows);
        self.path_box.set_visible(!names.is_empty());
    }

    /// Colors `{{variables}}` in the URL: green when defined, red when not.
    fn highlight_url(&self) {
        let text = self.url.text();
        let scope = (self.scope.borrow())();
        let attrs = pango::AttrList::new();
        for (range, name) in crate::vars::references(&text) {
            let (r, g, b) = if scope.is_defined(&name) { (0x2e, 0xc2, 0x7e) } else { (0xe0, 0x1b, 0x24) };
            let mut fg = pango::AttrColor::new_foreground(r * 257, g * 257, b * 257);
            fg.set_start_index(range.start as u32);
            fg.set_end_index(range.end as u32);
            attrs.insert(fg);
            let mut w = pango::AttrInt::new_weight(pango::Weight::Bold);
            w.set_start_index(range.start as u32);
            w.set_end_index(range.end as u32);
            attrs.insert(w);
        }
        // `:path` variables in blue
        for (range, _) in crate::vars::path_vars(&text) {
            let mut fg = pango::AttrColor::new_foreground(0x35 * 257, 0x84 * 257, 0xe4 * 257);
            fg.set_start_index(range.start as u32);
            fg.set_end_index(range.end as u32);
            attrs.insert(fg);
        }
        self.url.set_attributes(&attrs);
        let values: Vec<(String, String)> = self.path_vars.kvs().into_iter().filter(|kv| kv.enabled).map(|kv| (kv.key, scope.apply(&kv.value))).collect();
        let resolved = crate::vars::apply_path_vars(&scope.apply(&text), &values);
        self.url.set_tooltip_text(if resolved != text.as_str() { Some(resolved.as_str()) } else { None });
    }

    fn update_badges(&self) {
        let count = |n: usize, name: &str| if n > 0 { format!("{name} ({n})") } else { name.to_string() };
        self.params_page.set_title(Some(&count(self.params.items().iter().filter(|r| r.enabled).count(), "Params")));
        self.headers_page.set_title(Some(&count(self.headers.items().iter().filter(|r| r.enabled).count(), "Headers")));
        let has_body = self.body_mode.selected() != 0;
        self.body_page.set_title(Some(if has_body { "Body •" } else { "Body" }));
    }

    /// Loads a request into the widgets (without emitting change notifications).
    pub fn load(&self, r: &Request) {
        self.suppress.set(true);
        *self.base.borrow_mut() = r.clone();
        self.method.set_selected(r.method.index());
        for cls in Method::ALL.iter().map(|m| m.css_class()) {
            self.method.remove_css_class(cls);
        }
        self.method.add_css_class(r.method.css_class());
        self.url.set_text(&r.url);
        let mut params: Vec<Row> = r.params.iter().map(Row::from).collect();
        if params.is_empty() {
            params = util::parse_query(&r.url).iter().map(Row::from).collect();
        }
        self.params.set_items(&params);
        self.path_vars.set_items(&r.path_vars.iter().map(Row::from).collect::<Vec<_>>());
        self.sync_path_vars();
        self.headers.set_items(&r.headers.iter().map(Row::from).collect::<Vec<_>>());

        self.body_mode.set_selected(r.body.mode_index());
        self.body_stack.set_visible_child_name(BODY_MODES[r.body.mode_index() as usize]);
        self.raw_lang.set_visible(matches!(r.body, Body::Raw { .. }));
        self.beautify.set_visible(matches!(r.body, Body::Raw { .. } | Body::GraphQl { .. }));
        match &r.body {
            Body::Raw { language, content } => {
                self.raw_lang.set_selected(RawLanguage::ALL.iter().position(|l| l == language).unwrap_or(0) as u32);
                self.raw_view.buffer().set_text(content);
            }
            Body::UrlEncoded { fields } => self.urlencoded.set_items(&fields.iter().map(Row::from).collect::<Vec<_>>()),
            Body::FormData { fields } => self.formdata.set_items(&fields.iter().map(Row::from).collect::<Vec<_>>()),
            Body::Binary { path } => self.binary_path.set_text(path),
            Body::GraphQl { query, variables } => {
                self.gql_query.buffer().set_text(query);
                self.gql_vars.buffer().set_text(variables);
            }
            Body::None => {}
        }
        code_view::highlight(&self.raw_view.buffer(), self.raw_lang_hl());

        self.auth.load(&r.auth);

        self.follow_redirects.set_active(r.settings.follow_redirects);
        self.verify_tls.set_active(r.settings.verify_tls);
        self.timeout.set_value(r.settings.timeout_ms as f64);
        self.tests.set_items(&r.tests);
        self.description.buffer().set_text(&r.description);
        self.suppress.set(false);
        self.update_badges();
        self.highlight_url();
    }

    /// Reads the current widget state into a `Request`.
    pub fn collect(&self) -> Request {
        let base = self.base.borrow().clone();
        let body = match BODY_MODES[self.body_mode.selected() as usize] {
            "raw" => Body::Raw { language: RawLanguage::ALL[self.raw_lang.selected() as usize], content: text_of(&self.raw_view.buffer()) },
            "x-www-form-urlencoded" => Body::UrlEncoded { fields: self.urlencoded.kvs() },
            "form-data" => Body::FormData { fields: self.formdata.fields() },
            "binary" => Body::Binary { path: self.binary_path.text().to_string() },
            "GraphQL" => Body::GraphQl { query: text_of(&self.gql_query.buffer()), variables: text_of(&self.gql_vars.buffer()) },
            _ => Body::None,
        };
        let auth = self.auth.collect();
        Request {
            id: base.id,
            name: base.name,
            method: Method::ALL[self.method.selected() as usize],
            url: self.url.text().to_string(),
            params: self.params.kvs(),
            path_vars: self.path_vars.kvs(),
            headers: self.headers.kvs(),
            body,
            auth,
            settings: RequestSettings {
                follow_redirects: self.follow_redirects.is_active(),
                verify_tls: self.verify_tls.is_active(),
                timeout_ms: self.timeout.value() as u64,
            },
            description: text_of(&self.description.buffer()),
            tests: self.tests.items(),
            operation: base.operation,
        }
    }

    pub fn set_name(&self, name: &str) {
        self.base.borrow_mut().name = name.to_string();
    }

    #[allow(dead_code)]
    pub fn more_button(&self) -> &gtk::MenuButton {
        &self.more
    }
}
