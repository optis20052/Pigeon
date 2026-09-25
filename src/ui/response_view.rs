//! Response pane: status line, body (pretty/raw/preview), headers, cookies and test results.

use super::code_view::{self, Lang};
use crate::assertions::TestResult;
use crate::http::{self, Response};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::rc::Rc;

pub struct ResponseView {
    root: gtk::Box,
    state: gtk::Stack,
    error_page: adw::StatusPage,
    status: gtk::Label,
    time: gtk::Label,
    size: gtk::Label,
    meta: gtk::Box,
    switcher: adw::InlineViewSwitcher,
    stack: adw::ViewStack,
    body_mode: gtk::DropDown,
    body_stack: gtk::Stack,
    body_view: gtk::TextView,
    picture: gtk::Picture,
    search: gtk::SearchEntry,
    headers: gtk::ListBox,
    cookies: gtk::ListBox,
    tests: gtk::ListBox,
    headers_page: adw::ViewStackPage,
    cookies_page: adw::ViewStackPage,
    tests_page: adw::ViewStackPage,
    current: RefCell<Option<Response>>,
}

fn kv_row(key: &str, value: &str) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.set_margin_top(6);
    b.set_margin_bottom(6);
    b.set_margin_start(10);
    b.set_margin_end(10);
    let k = gtk::Label::builder().label(key).xalign(0.0).selectable(true).width_chars(22).max_width_chars(34).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).build();
    k.add_css_class("heading");
    let v = gtk::Label::builder().label(value).xalign(0.0).selectable(true).hexpand(true).wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar).build();
    v.add_css_class("monospace");
    b.append(&k);
    b.append(&v);
    gtk::ListBoxRow::builder().child(&b).activatable(false).build()
}

fn clear(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn scrolled_list() -> (gtk::ScrolledWindow, gtk::ListBox) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.set_margin_top(8);
    list.set_margin_bottom(8);
    list.set_margin_start(8);
    list.set_margin_end(8);
    list.set_valign(gtk::Align::Start);
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
    (scroll, list)
}

impl ResponseView {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("response-pane");

        // --- top bar: switcher + status meta
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        top.set_margin_start(12);
        top.set_margin_end(12);
        top.set_margin_top(6);
        top.set_margin_bottom(6);
        let title = gtk::Label::new(Some("Response"));
        title.add_css_class("heading");
        top.append(&title);

        let stack = adw::ViewStack::new();
        let switcher = adw::InlineViewSwitcher::builder().stack(&stack).build();
        switcher.add_css_class("flat");
        top.append(&switcher);

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        top.append(&spacer);

        let meta = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let status = gtk::Label::new(None);
        status.add_css_class("status-pill");
        let time = gtk::Label::new(None);
        let size = gtk::Label::new(None);
        for l in [&time, &size] {
            l.add_css_class("dim-label");
            l.add_css_class("numeric");
        }
        meta.append(&status);
        meta.append(&time);
        meta.append(&size);
        meta.set_visible(false);
        top.append(&meta);

        // --- body page
        let body_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let body_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        body_bar.set_margin_start(12);
        body_bar.set_margin_end(12);
        body_bar.set_margin_bottom(6);
        let body_mode = super::util::dropdown(&["Pretty", "Raw", "Preview"]);
        body_bar.append(&body_mode);
        let search = gtk::SearchEntry::builder().placeholder_text("Find in response").hexpand(true).build();
        body_bar.append(&search);
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some("Copy response body"));
        copy.add_css_class("flat");
        let save = gtk::Button::from_icon_name("document-save-symbolic");
        save.set_tooltip_text(Some("Save response to file"));
        save.add_css_class("flat");
        let wrap = gtk::ToggleButton::builder().icon_name("format-justify-left-symbolic").active(true).tooltip_text("Wrap lines").build();
        wrap.add_css_class("flat");
        body_bar.append(&wrap);
        body_bar.append(&copy);
        body_bar.append(&save);
        body_box.append(&body_bar);

        let (body_scroll, body_view) = code_view::new_view(false);
        let picture = gtk::Picture::builder().can_shrink(true).content_fit(gtk::ContentFit::ScaleDown).build();
        let body_stack = gtk::Stack::new();
        body_stack.add_named(&body_scroll, Some("text"));
        body_stack.add_named(&picture, Some("image"));
        body_stack.set_vexpand(true);
        body_box.append(&body_stack);

        stack.add_titled(&body_box, Some("body"), "Body");
        let (hs, headers) = scrolled_list();
        let headers_page = stack.add_titled(&hs, Some("headers"), "Headers");
        let (cs, cookies) = scrolled_list();
        let cookies_page = stack.add_titled(&cs, Some("cookies"), "Cookies");
        let (ts, tests) = scrolled_list();
        let tests_page = stack.add_titled(&ts, Some("tests"), "Tests");

        // --- overall state: empty / loading / error / response
        let state = gtk::Stack::new();
        state.set_vexpand(true);
        let empty = adw::StatusPage::builder()
            .icon_name("mail-send-symbolic")
            .title("No Response Yet")
            .description("Enter a URL and press <b>Send</b> or <b>Ctrl+Enter</b>")
            .build();
        empty.add_css_class("compact");
        let spinner = adw::Spinner::builder().width_request(32).height_request(32).halign(gtk::Align::Center).valign(gtk::Align::Center).build();
        let error_page = adw::StatusPage::builder().icon_name("dialog-error-symbolic").title("Could Not Send Request").build();
        error_page.add_css_class("compact");
        state.add_named(&empty, Some("empty"));
        state.add_named(&spinner, Some("loading"));
        state.add_named(&error_page, Some("error"));
        state.add_named(&stack, Some("response"));

        root.append(&top);
        root.append(&state);

        switcher.set_visible(false);
        cookies_page.set_visible(false);
        tests_page.set_visible(false);

        let this = Rc::new(Self {
            root,
            state,
            error_page,
            status,
            time,
            size,
            meta,
            switcher,
            stack,
            body_mode,
            body_stack,
            body_view,
            picture,
            search,
            headers,
            cookies,
            tests,
            headers_page,
            cookies_page,
            tests_page,
            current: RefCell::new(None),
        });

        let weak = Rc::downgrade(&this);
        this.body_mode.connect_selected_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.render_body();
            }
        });
        let view = this.body_view.clone();
        wrap.connect_toggled(move |b| view.set_wrap_mode(if b.is_active() { gtk::WrapMode::WordChar } else { gtk::WrapMode::None }));

        let view = this.body_view.clone();
        this.search.connect_search_changed(move |e| {
            code_view::find_next(&view, &e.text(), false);
        });
        let view = this.body_view.clone();
        this.search.connect_activate(move |e| {
            code_view::find_next(&view, &e.text(), false);
        });
        let view = this.body_view.clone();
        this.search.connect_next_match(move |e| {
            code_view::find_next(&view, &e.text(), false);
        });
        let view = this.body_view.clone();
        this.search.connect_previous_match(move |e| {
            code_view::find_next(&view, &e.text(), true);
        });

        let weak = Rc::downgrade(&this);
        copy.connect_clicked(move |btn| {
            if let Some(this) = weak.upgrade() {
                super::util::copy_to_clipboard(btn, &super::util::text_of(&this.body_view.buffer()));
            }
        });
        let weak = Rc::downgrade(&this);
        save.connect_clicked(move |btn| {
            let Some(this) = weak.upgrade() else { return };
            let Some(resp) = this.current.borrow().clone() else { return };
            let root = btn.root().and_downcast::<gtk::Window>();
            let name = suggested_filename(&resp);
            glib::spawn_future_local(async move {
                let dialog = gtk::FileDialog::builder().title("Save Response").initial_name(name).build();
                if let Ok(file) = dialog.save_future(root.as_ref()).await {
                    if let Some(path) = file.path() {
                        if let Err(e) = std::fs::write(&path, &resp.body) {
                            eprintln!("pigeon: saving response: {e}");
                        }
                    }
                }
            });
        });
        this
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn set_loading(&self) {
        self.switcher.set_visible(false);
        self.state.set_visible_child_name("loading");
    }

    pub fn set_error(&self, msg: &str) {
        self.meta.set_visible(false);
        self.switcher.set_visible(false);
        self.error_page.set_description(Some(&glib::markup_escape_text(msg)));
        self.state.set_visible_child_name("error");
    }

    pub fn set_response(&self, resp: &Response, results: &[TestResult]) {
        self.status.set_text(&format!("{} {}", resp.status, resp.reason));
        for c in ["status-ok", "status-redirect", "status-client-error", "status-server-error"] {
            self.status.remove_css_class(c);
        }
        self.status.add_css_class(super::util::status_css(resp.status));
        self.status.set_tooltip_text(Some(&format!("{}\n{}{}", resp.version, resp.final_url, resp.remote_addr.as_ref().map(|a| format!("\nRemote: {a}")).unwrap_or_default())));
        self.time.set_text(&http::human_duration(resp.elapsed));
        self.size.set_text(&http::human_size(resp.size()));
        self.meta.set_visible(true);

        clear(&self.headers);
        for (k, v) in &resp.headers {
            self.headers.append(&kv_row(k, v));
        }
        self.headers_page.set_title(Some(&format!("Headers ({})", resp.headers.len())));

        clear(&self.cookies);
        let cookies = http::parse_set_cookies(resp);
        for (name, value, attrs) in &cookies {
            let row = kv_row(name, &if attrs.is_empty() { value.clone() } else { format!("{value}\n{attrs}") });
            self.cookies.append(&row);
        }
        self.cookies_page.set_title(Some(&format!("Cookies ({})", cookies.len())));
        self.cookies_page.set_visible(!cookies.is_empty());

        clear(&self.tests);
        for r in results {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            b.set_margin_top(6);
            b.set_margin_bottom(6);
            b.set_margin_start(10);
            b.set_margin_end(10);
            let badge = gtk::Label::new(Some(if r.passed { "PASS" } else { "FAIL" }));
            badge.add_css_class(if r.passed { "test-pass" } else { "test-fail" });
            b.append(&badge);
            let name = gtk::Label::builder().label(&r.name).xalign(0.0).hexpand(true).wrap(true).build();
            b.append(&name);
            if !r.message.is_empty() {
                let m = gtk::Label::new(Some(&r.message));
                m.add_css_class("dim-label");
                m.add_css_class("caption");
                b.append(&m);
            }
            self.tests.append(&gtk::ListBoxRow::builder().child(&b).activatable(false).build());
        }
        let passed = results.iter().filter(|r| r.passed).count();
        self.tests_page.set_title(Some(&format!("Tests ({passed}/{})", results.len())));
        self.tests_page.set_visible(!results.is_empty());

        *self.current.borrow_mut() = Some(resp.clone());
        let is_image = resp.content_type().starts_with("image/");
        self.body_mode.set_selected(if is_image { 2 } else { 0 });
        self.render_body();
        self.switcher.set_visible(true);
        self.state.set_visible_child_name("response");
        if self.stack.visible_child_name().as_deref() != Some("body") && !results.is_empty() && passed < results.len() {
            self.stack.set_visible_child_name("tests");
        }
    }

    fn render_body(&self) {
        let Some(resp) = self.current.borrow().clone() else { return };
        let buffer = self.body_view.buffer();
        let ct = resp.content_type();
        match self.body_mode.selected() {
            2 if ct.starts_with("image/") => {
                let bytes = glib::Bytes::from(&resp.body);
                match gdk::Texture::from_bytes(&bytes) {
                    Ok(tex) => {
                        self.picture.set_paintable(Some(&tex));
                        self.body_stack.set_visible_child_name("image");
                        return;
                    }
                    Err(_) => buffer.set_text("Unable to preview this image."),
                }
            }
            1 => {
                buffer.set_text(&resp.text());
                code_view::highlight(&buffer, Lang::Plain);
            }
            _ => {
                let text = resp.text();
                let (pretty, lang) = if resp.is_json() {
                    (code_view::pretty_json(&text).unwrap_or(text), Lang::Json)
                } else if ct.contains("xml") || ct.contains("html") {
                    (code_view::pretty_xml(&text), Lang::Xml)
                } else {
                    (text, code_view::lang_for_content_type(&ct))
                };
                buffer.set_text(&pretty);
                code_view::highlight(&buffer, lang);
            }
        }
        self.body_stack.set_visible_child_name("text");
        buffer.place_cursor(&buffer.start_iter());
    }
}

fn suggested_filename(resp: &Response) -> String {
    let from_url = url::Url::parse(&resp.final_url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(|s| s.to_string())))
        .filter(|s| s.contains('.'));
    from_url.unwrap_or_else(|| {
        let ct = resp.content_type();
        let ext = if ct.contains("json") {
            "json"
        } else if ct.contains("html") {
            "html"
        } else if ct.contains("xml") {
            "xml"
        } else if let Some(sub) = ct.strip_prefix("image/") {
            sub.split(';').next().unwrap_or("bin")
        } else {
            "txt"
        };
        format!("response.{ext}")
    })
}
