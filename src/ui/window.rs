//! Main window: sidebar (collections / history), request tabs, environment picker and actions.

use super::request_editor::RequestEditor;
use crate::http::{self, RUNTIME};
use crate::model::*;
use crate::vars::Scope;
use crate::{assertions, interchange, storage};
use adw::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

pub struct Tab {
    pub page: adw::TabPage,
    pub editor: Rc<RequestEditor>,
    /// Collection the request is saved in; `None` for unsaved drafts.
    pub collection: RefCell<Option<String>>,
    pub saved: RefCell<Request>,
    abort: RefCell<Option<tokio::task::AbortHandle>>,
    chip: TabChip,
}

/// A tab in the custom tab strip (AdwTabBar can't color part of a title).
struct TabChip {
    root: gtk::Box,
    method: gtk::Label,
    name: gtk::Label,
    dirty: gtk::Label,
    close: gtk::Button,
}

impl TabChip {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        root.add_css_class("tab-chip");
        let method = gtk::Label::new(None);
        method.add_css_class("method-badge");
        let name = gtk::Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(22).build();
        let dirty = gtk::Label::new(Some("●"));
        dirty.add_css_class("tab-dirty");
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.add_css_class("flat");
        close.add_css_class("circular");
        close.add_css_class("tab-close");
        close.set_tooltip_text(Some("Close Tab (Ctrl+W)"));
        root.append(&method);
        root.append(&name);
        root.append(&dirty);
        root.append(&close);
        Self { root, method, name, dirty, close }
    }
}

impl Tab {
    pub fn is_dirty(&self) -> bool {
        self.collection.borrow().is_none() || self.editor.collect() != *self.saved.borrow()
    }
}

#[derive(Clone)]
enum TreeKind {
    Collection,
    Folder,
    Request,
}

/// Everything needed to render one sidebar row.
struct RowSpec<'a> {
    depth: i32,
    id: &'a str,
    cid: &'a str,
    name: &'a str,
    kind: TreeKind,
    method: Option<Method>,
    open: bool,
    syncable: bool,
    /// The item's own color (collections / folders).
    color: Option<&'a str>,
    /// Background tint: own color or the nearest colored ancestor's.
    tint: Option<&'a str>,
}

/// Context menu of a request tab (actions apply to the selected tab; right click selects first).
fn tab_context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let s1 = gio::Menu::new();
    s1.append(Some("Save"), Some("win.save"));
    s1.append(Some("Rename…"), Some("win.rename-tab"));
    s1.append(Some("Duplicate Tab"), Some("win.duplicate-tab"));
    s1.append(Some("Reveal in Sidebar"), Some("win.reveal-tab"));
    let s2 = gio::Menu::new();
    s2.append(Some("Copy as cURL"), Some("win.copy-curl"));
    s2.append(Some("Generate Code…"), Some("win.code"));
    let s3 = gio::Menu::new();
    s3.append(Some("Close Tab"), Some("win.close-tab"));
    s3.append(Some("Close Other Tabs"), Some("win.close-other-tabs"));
    s3.append(Some("Close Tabs to the Right"), Some("win.close-tabs-right"));
    s3.append(Some("Close All Tabs"), Some("win.close-all-tabs"));
    menu.append_section(None, &s1);
    menu.append_section(None, &s2);
    menu.append_section(None, &s3);
    menu
}

/// A row of color swatches placed inside a tree item's menu.
fn color_picker(id: &str, current: Option<&str>, popover: &gtk::PopoverMenu) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.add_css_class("color-picker");
    let swatch = |key: &str, tooltip: &str| {
        let b = gtk::Button::builder().tooltip_text(tooltip).valign(gtk::Align::Center).build();
        b.add_css_class("color-swatch");
        if key.is_empty() {
            b.add_css_class("none-swatch");
            b.set_icon_name("window-close-symbolic");
        } else {
            b.add_css_class(&format!("swatch-{key}"));
        }
        if current.unwrap_or("") == key {
            b.add_css_class("current");
        }
        let target = format!("{id}|{key}").to_variant();
        b.set_action_name(Some("win.set-color"));
        b.set_action_target_value(Some(&target));
        let pop = popover.clone();
        b.connect_clicked(move |_| pop.popdown());
        b
    };
    bar.append(&swatch("", "No color"));
    for (key, label, _) in super::colors::COLORS {
        bar.append(&swatch(key, label));
    }
    bar
}

#[derive(Clone)]
struct TreeRow {
    id: String,
    collection: String,
    kind: TreeKind,
}

pub struct App {
    /// The active project.
    pub ws: RefCell<Workspace>,
    /// App-wide preferences.
    pub settings: RefCell<Settings>,
    projects: RefCell<storage::ProjectIndex>,
    project_btn: gtk::MenuButton,
    project_label: gtk::Label,
    project_icon: gtk::Image,
    /// Set while tabs are closed programmatically (project switch): skips save prompts.
    closing_all: Cell<bool>,
    pub window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    tab_view: adw::TabView,
    tabs: RefCell<Vec<Rc<Tab>>>,
    tree: gtk::ListBox,
    tree_rows: RefCell<Vec<TreeRow>>,
    expanded: RefCell<HashSet<String>>,
    filter: gtk::SearchEntry,
    history: gtk::ListBox,
    env_dropdown: gtk::DropDown,
    env_ids: RefCell<Vec<Option<String>>>,
    updating_envs: Cell<bool>,
    save_source: RefCell<Option<glib::SourceId>>,
    sidebar_toggle: gtk::ToggleButton,
    /// Shown when no tab is open; carries the app icon.
    empty_page: adw::StatusPage,
    tab_strip: gtk::Box,
    strip_scroll: gtk::ScrolledWindow,
}

/// Formats an error including its source chain (reqwest errors are terse otherwise).
fn count_requests(items: &[Item]) -> usize {
    items.iter().map(|i| match i {
        Item::Request(_) => 1,
        Item::Folder(f) => count_requests(&f.items),
    }).sum()
}

fn error_chain(e: &anyhow::Error) -> String {
    let mut parts: Vec<String> = vec![];
    for cause in e.chain() {
        let s = cause.to_string();
        if !parts.iter().any(|p| p.contains(&s)) {
            parts.push(s);
        }
    }
    parts.join("\n")
}

impl App {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let index = storage::load_index();
        let ws = storage::load_project(&index.active);

        // Minimum size: URL bar plus a usable strip of both request and response panes.
        // (Below 800sp the sidebar turns into an overlay, see the breakpoint further down.)
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Pigeon")
            .default_width(1320)
            .default_height(860)
            .width_request(640)
            .height_request(480)
            .build();

        // ------------------------------------------------ sidebar
        let sidebar_header = adw::HeaderBar::new();
        // project switcher
        let project_label = gtk::Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(18).build();
        project_label.add_css_class("heading");
        let project_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let project_icon = super::project_icon::image(None, 16);
        project_box.append(&project_icon);
        project_box.append(&project_label);
        project_box.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
        let project_btn = gtk::MenuButton::builder().child(&project_box).tooltip_text("Switch Project").build();
        project_btn.add_css_class("flat");
        sidebar_header.set_title_widget(Some(&project_btn));
        let new_menu = gio::Menu::new();
        new_menu.append(Some("New Request"), Some("win.new-tab"));
        new_menu.append(Some("New Collection"), Some("win.new-collection"));
        new_menu.append(Some("Import File…"), Some("win.import"));
        new_menu.append(Some("Import from URL (OpenAPI / Swagger)…"), Some("win.import-url"));
        let new_btn = gtk::MenuButton::builder().icon_name("list-add-symbolic").menu_model(&new_menu).tooltip_text("New").build();
        sidebar_header.pack_start(&new_btn);

        let filter = gtk::SearchEntry::builder().placeholder_text("Filter requests").build();
        filter.set_margin_start(8);
        filter.set_margin_end(8);
        filter.set_margin_bottom(6);

        let side_stack = adw::ViewStack::new();
        side_stack.set_vexpand(true);

        let tree = gtk::ListBox::new();
        tree.add_css_class("navigation-sidebar");
        tree.add_css_class("collection-tree");
        let tree_scroll = gtk::ScrolledWindow::builder().child(&tree).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let collections_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        collections_box.append(&filter);
        collections_box.append(&tree_scroll);
        side_stack.add_titled_with_icon(&collections_box, Some("collections"), "Collections", "folder-symbolic");

        let history = gtk::ListBox::new();
        history.add_css_class("navigation-sidebar");
        let history_scroll = gtk::ScrolledWindow::builder().child(&history).hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let history_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let clear_history = gtk::Button::builder().label("Clear History").halign(gtk::Align::End).margin_end(8).margin_bottom(4).build();
        clear_history.add_css_class("flat");
        clear_history.set_action_name(Some("win.clear-history"));
        history_box.append(&clear_history);
        history_box.append(&history_scroll);
        side_stack.add_titled_with_icon(&history_box, Some("history"), "History", "document-open-recent-symbolic");

        let side_switcher = adw::InlineViewSwitcher::builder().stack(&side_stack).margin_start(8).margin_end(8).margin_bottom(6).build();
        side_switcher.set_homogeneous(true);

        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.append(&side_switcher);
        sidebar_box.append(&side_stack);
        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(&sidebar_header);
        sidebar.set_content(Some(&sidebar_box));

        // ------------------------------------------------ content
        let content_header = adw::HeaderBar::new();
        content_header.set_show_title(false);
        let toggle_sidebar = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").active(true).tooltip_text("Toggle Sidebar (F9)").build();
        content_header.pack_start(&toggle_sidebar);

        let env_dropdown = gtk::DropDown::from_strings(&["No Environment"]);
        env_dropdown.set_tooltip_text(Some("Active environment"));
        let env_btn = gtk::Button::builder().icon_name("emblem-system-symbolic").tooltip_text("Manage Environments (Ctrl+E)").action_name("win.environments").build();
        let env_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        env_box.add_css_class("linked");
        env_box.append(&env_dropdown);
        env_box.append(&env_btn);

        let main_menu = gio::Menu::new();
        let section = gio::Menu::new();
        section.append(Some("New Request"), Some("win.new-tab"));
        section.append(Some("New Collection"), Some("win.new-collection"));
        section.append(Some("Import File…"), Some("win.import"));
        section.append(Some("Import from URL (OpenAPI / Swagger)…"), Some("win.import-url"));
        main_menu.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some("Project Settings & Auth…"), Some("win.project-settings"));
        section.append(Some("Environments & Globals"), Some("win.environments"));
        section.append(Some("Clear Cookies"), Some("win.clear-cookies"));
        main_menu.append_section(None, &section);
        let section = gio::Menu::new();
        section.append(Some("Preferences"), Some("win.preferences"));
        section.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
        section.append(Some("About Pigeon"), Some("win.about"));
        main_menu.append_section(None, &section);
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&main_menu).primary(true).tooltip_text("Main Menu").build();
        content_header.pack_end(&menu_btn);
        content_header.pack_end(&env_box);

        let tab_view = adw::TabView::new();
        let tab_strip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tab_strip.add_css_class("tab-strip");
        let strip_scroll = gtk::ScrolledWindow::builder()
            .child(&tab_strip)
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .build();
        // mouse wheel scrolls the strip horizontally
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let adj = strip_scroll.hadjustment();
        wheel.connect_scroll(move |_, dx, dy| {
            adj.set_value(adj.value() + (dx + dy) * 40.0);
            glib::Propagation::Stop
        });
        strip_scroll.add_controller(wheel);
        let new_tab_btn = gtk::Button::builder().icon_name("tab-new-symbolic").tooltip_text("New Tab (Ctrl+T)").action_name("win.new-tab").valign(gtk::Align::Center).build();
        new_tab_btn.add_css_class("flat");
        let tab_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tab_bar.add_css_class("tab-bar");
        tab_bar.append(&strip_scroll);
        tab_bar.append(&new_tab_btn);

        let empty = adw::StatusPage::builder()
            .title("Pigeon")
            .description("Open a request from the sidebar, or create a new one")
            .build();
        let new_req = gtk::Button::builder().label("New Request").halign(gtk::Align::Center).action_name("win.new-tab").build();
        new_req.add_css_class("pill");
        new_req.add_css_class("suggested-action");
        empty.set_child(Some(&new_req));

        let content_stack = gtk::Stack::new();
        content_stack.add_named(&empty, Some("empty"));
        content_stack.add_named(&tab_view, Some("tabs"));

        let content = adw::ToolbarView::new();
        content.add_top_bar(&content_header);
        content.add_top_bar(&tab_bar);
        content.set_content(Some(&content_stack));

        let split = adw::OverlaySplitView::builder().sidebar(&sidebar).content(&content).min_sidebar_width(260.0).max_sidebar_width(380.0).sidebar_width_fraction(0.24).build();
        toggle_sidebar.bind_property("active", &split, "show-sidebar").bidirectional().sync_create().build();

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        window.set_content(Some(&toasts));

        // collapse sidebar on narrow windows
        let bp = adw::Breakpoint::new(adw::BreakpointCondition::new_length(adw::BreakpointConditionLengthType::MaxWidth, 800.0, adw::LengthUnit::Sp));
        bp.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(bp);

        let this = Rc::new(Self {
            ws: RefCell::new(ws),
            settings: RefCell::new(storage::load_settings()),
            projects: RefCell::new(index),
            project_btn,
            project_label,
            project_icon,
            closing_all: Cell::new(false),
            window,
            toasts,
            tab_view,
            tabs: RefCell::new(vec![]),
            tree,
            tree_rows: RefCell::new(vec![]),
            expanded: RefCell::new(HashSet::new()),
            filter,
            history,
            env_dropdown,
            env_ids: RefCell::new(vec![]),
            updating_envs: Cell::new(false),
            save_source: RefCell::new(None),
            sidebar_toggle: toggle_sidebar,
            empty_page: empty.clone(),
            tab_strip,
            strip_scroll,
        });

        // tab stack visibility
        let cs = content_stack.clone();
        this.tab_view.connect_n_pages_notify(move |tv| cs.set_visible_child_name(if tv.n_pages() > 0 { "tabs" } else { "empty" }));

        let weak = Rc::downgrade(&this);
        this.tab_view.connect_selected_page_notify(move |_| {
            if let Some(t) = weak.upgrade() {
                t.sync_strip();
            }
        });
        let weak = Rc::downgrade(&this);
        this.tab_view.connect_page_reordered(move |_, _, _| {
            if let Some(t) = weak.upgrade() {
                t.sync_strip();
            }
        });

        this.setup_signals();
        this.setup_actions();
        this.refresh_tree();
        this.refresh_history();
        this.refresh_envs();
        this.refresh_projects();
        this.refresh_app_icon();
        this.restore_geometry();
        this.restore_session();
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn toast(&self, msg: &str) {
        self.toasts.add_toast(adw::Toast::builder().title(glib::markup_escape_text(msg)).timeout(3).build());
    }

    /// Persists the workspace shortly after the last change.
    pub fn save_soon(self: &Rc<Self>) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        *self.save_source.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            if let Some(t) = weak.upgrade() {
                t.save_source.borrow_mut().take();
                storage::save(&t.ws.borrow());
            }
        }));
    }

    pub fn save_now(&self) {
        if let Some(id) = self.save_source.borrow_mut().take() {
            id.remove();
        }
        storage::save(&self.ws.borrow());
    }

    // ================================================================ signals

    fn setup_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tree.connect_row_activated(move |_, row| {
            let Some(t) = weak.upgrade() else { return };
            let idx = row.index();
            let Some(tr) = t.tree_rows.borrow().get(idx as usize).cloned() else { return };
            match tr.kind {
                TreeKind::Request => t.open_saved_request(&tr.collection, &tr.id),
                _ => {
                    let mut exp = t.expanded.borrow_mut();
                    if !exp.remove(&tr.id) {
                        exp.insert(tr.id.clone());
                    }
                    drop(exp);
                    t.refresh_tree();
                }
            }
        });

        let weak = Rc::downgrade(self);
        self.filter.connect_search_changed(move |_| {
            if let Some(t) = weak.upgrade() {
                t.refresh_tree();
            }
        });

        let weak = Rc::downgrade(self);
        self.history.connect_row_activated(move |_, row| {
            let Some(t) = weak.upgrade() else { return };
            let entry = t.ws.borrow().history.get(row.index() as usize).cloned();
            if let Some(entry) = entry {
                let mut req = entry.request.clone();
                req.id = new_id();
                t.open_tab(req, None, true);
            }
        });

        let weak = Rc::downgrade(self);
        self.env_dropdown.connect_selected_notify(move |dd| {
            let Some(t) = weak.upgrade() else { return };
            if t.updating_envs.get() {
                return;
            }
            let id = t.env_ids.borrow().get(dd.selected() as usize).cloned().flatten();
            t.ws.borrow_mut().active_environment = id;
            t.save_soon();
            t.refresh_scopes();
        });

        let weak = Rc::downgrade(self);
        self.tab_view.connect_close_page(move |tv, page| {
            let Some(t) = weak.upgrade() else { return glib::Propagation::Proceed };
            let tab = t.tab_for_page(page);
            match tab {
                Some(tab) if !t.closing_all.get() && tab.is_dirty() && tab.collection.borrow().is_some() => {
                    let dialog = adw::AlertDialog::new(Some("Save Changes?"), Some("This request has unsaved changes that will be lost if you close it."));
                    dialog.add_responses(&[("cancel", "Cancel"), ("discard", "Discard"), ("save", "Save")]);
                    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
                    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
                    dialog.set_default_response(Some("save"));
                    dialog.set_close_response("cancel");
                    let t2 = t.clone();
                    let tv = tv.clone();
                    let page = page.clone();
                    dialog.choose(Some(&t.window), gio::Cancellable::NONE, move |resp| {
                        match resp.as_str() {
                            "save" => {
                                t2.save_tab(&tab);
                                t2.finish_close(&tv, &page, true);
                            }
                            "discard" => t2.finish_close(&tv, &page, true),
                            _ => tv.close_page_finish(&page, false),
                        }
                    });
                    glib::Propagation::Stop
                }
                _ => {
                    t.finish_close(tv, page, true);
                    glib::Propagation::Stop
                }
            }
        });

        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(t) = weak.upgrade() {
                t.save_session();
                t.save_geometry();
                t.save_now();
            }
            glib::Propagation::Proceed
        });
    }

    fn finish_close(&self, tv: &adw::TabView, page: &adw::TabPage, confirm: bool) {
        let tab = self.tab_for_page(page);
        if let Some(tab) = &tab {
            if let Some(h) = tab.abort.borrow_mut().take() {
                h.abort();
            }
        }
        if let Some(tab) = &tab {
            self.tab_strip.remove(&tab.chip.root);
        }
        self.tabs.borrow_mut().retain(|t| &t.page != page);
        tv.close_page_finish(page, confirm);
    }

    /// Closes the pages matching `pred(position, page)`; unsaved tabs still ask first.
    fn close_pages(&self, pred: impl Fn(i32, &adw::TabPage) -> bool) {
        let pages: Vec<adw::TabPage> = self.tab_view.pages().iter::<adw::TabPage>().filter_map(|p| p.ok()).collect();
        let doomed: Vec<adw::TabPage> = pages.into_iter().enumerate().filter(|(i, p)| pred(*i as i32, p)).map(|(_, p)| p).collect();
        for page in doomed {
            self.tab_view.close_page(&page);
        }
    }

    /// Expands the tree down to the tab's request and highlights it.
    fn reveal_in_sidebar(self: &Rc<Self>, tab: &Tab) {
        let Some(cid) = tab.collection.borrow().clone() else {
            self.toast("This request isn't saved in a collection yet");
            return;
        };
        let id = tab.editor.collect().id;
        let ancestors = self.ws.borrow().ancestor_ids(&cid, &id);
        self.filter.set_text("");
        self.expanded.borrow_mut().insert(cid.clone());
        self.expanded.borrow_mut().extend(ancestors);
        self.sidebar_toggle.set_active(true);
        self.refresh_tree();
        let index = self.tree_rows.borrow().iter().position(|r| r.id == id);
        if let Some(row) = index.and_then(|i| self.tree.row_at_index(i as i32)) {
            self.tree.select_row(Some(&row));
            row.grab_focus();
        }
    }

    fn tab_for_page(&self, page: &adw::TabPage) -> Option<Rc<Tab>> {
        self.tabs.borrow().iter().find(|t| &t.page == page).cloned()
    }

    pub fn current_tab(&self) -> Option<Rc<Tab>> {
        self.tab_view.selected_page().and_then(|p| self.tab_for_page(&p))
    }

    // ================================================================ actions

    fn setup_actions(self: &Rc<Self>) {
        let add = |name: &str, f: Box<dyn Fn(&Rc<App>)>| {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(t) = weak.upgrade() {
                    f(&t);
                }
            });
            self.window.add_action(&action);
        };
        add("new-tab", Box::new(|t| {
            t.open_tab(Request::default(), None, true);
        }));
        add("close-tab", Box::new(|t| {
            if let Some(p) = t.tab_view.selected_page() {
                t.tab_view.close_page(&p);
            }
        }));
        add("close-other-tabs", Box::new(|t| {
            let keep = t.tab_view.selected_page();
            t.close_pages(|_, page| Some(page) != keep.as_ref());
        }));
        add("close-tabs-right", Box::new(|t| {
            let Some(sel) = t.tab_view.selected_page() else { return };
            let from = t.tab_view.page_position(&sel);
            t.close_pages(|pos, _| pos > from);
        }));
        add("close-all-tabs", Box::new(|t| t.close_pages(|_, _| true)));
        add("reveal-tab", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                t.reveal_in_sidebar(&tab);
            }
        }));
        add("send", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                t.send(&tab);
            }
        }));
        add("save", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                t.save_tab(&tab);
            }
        }));
        add("focus-url", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                tab.editor.focus_url();
            }
        }));
        add("new-collection", Box::new(|t| {
            let t2 = t.clone();
            super::dialogs::prompt(&t.window, "New Collection", "Name", "New Collection", "Create", move |name| {
                let c = Collection::new(name);
                t2.expanded.borrow_mut().insert(c.id.clone());
                t2.ws.borrow_mut().collections.push(c);
                t2.save_soon();
                t2.refresh_tree();
            });
        }));
        add("import", Box::new(|t| t.import_file()));
        add("import-url", Box::new(|t| t.import_url()));
        add("new-project", Box::new(|t| {
            let t2 = t.clone();
            super::dialogs::prompt(&t.window, "New Project", "Name", "New Project", "Create", move |name| t2.create_project(&name));
        }));
        add("rename-project", Box::new(|t| {
            let current = t.ws.borrow().name.clone();
            let t2 = t.clone();
            super::dialogs::prompt(&t.window, "Rename Project", "Name", &current, "Rename", move |name| t2.rename_project(&name));
        }));
        add("delete-project", Box::new(|t| {
            let name = t.ws.borrow().name.clone();
            let t2 = t.clone();
            super::dialogs::confirm(&t.window, &format!("Delete project “{name}”?"), "All of its collections, environments and history will be permanently deleted.", "Delete", move || t2.delete_current_project());
        }));
        add("project-settings", Box::new(|t| super::dialogs::project_settings(t)));
        {
            let active = self.projects.borrow().active.clone();
            let action = gio::SimpleAction::new_stateful("switch-project", Some(glib::VariantTy::STRING), &active.to_variant());
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, param| {
                if let (Some(t), Some(id)) = (weak.upgrade(), param.and_then(|p| p.get::<String>())) {
                    t.switch_project(&id);
                }
            });
            self.window.add_action(&action);
        }
        add("environments", Box::new(|t| super::environments::show(t)));
        add("clear-history", Box::new(|t| {
            t.ws.borrow_mut().history.clear();
            t.save_soon();
            t.refresh_history();
        }));
        add("clear-cookies", Box::new(|t| {
            http::clear_cookies();
            t.toast("Cookies cleared");
        }));
        add("code", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                let p = t.prepare(&tab);
                super::dialogs::codegen(&t.window, &p);
            }
        }));
        add("copy-curl", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                let p = t.prepare(&tab);
                super::util::copy_to_clipboard(&t.window, &crate::codegen::generate(&p, crate::codegen::Target::Curl));
                t.toast("Copied cURL command");
            }
        }));
        add("rename-tab", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                let current = tab.editor.collect().name;
                let t2 = t.clone();
                super::dialogs::prompt(&t.window, "Rename Request", "Name", &current, "Rename", move |name| {
                    tab.editor.set_name(&name);
                    if tab.collection.borrow().is_some() {
                        let id = tab.editor.collect().id;
                        t2.rename(&id, &name);
                    } else {
                        t2.update_tab_title(&tab);
                    }
                });
            }
        }));
        add("duplicate-tab", Box::new(|t| {
            if let Some(tab) = t.current_tab() {
                let mut r = tab.editor.collect();
                r.id = new_id();
                r.name = format!("{} Copy", r.name);
                t.open_tab(r, None, true);
            }
        }));
        add("about", Box::new(|t| {
            let about = adw::AboutDialog::builder()
                .application_name("Pigeon")
                .application_icon("dev.pigeon.Pigeon")
                .version(env!("CARGO_PKG_VERSION"))
                .comments("A fast, native API client for GNOME.\nBuilt with Rust, GTK4 and libadwaita.")
                .license_type(gtk::License::MitX11)
                .build();
            about.present(Some(&t.window));
        }));
        add("shortcuts", Box::new(|t| super::dialogs::shortcuts(&t.window)));
        add("preferences", Box::new(|t| super::dialogs::preferences(t)));
        add("toggle-sidebar", Box::new(|t| t.sidebar_toggle.set_active(!t.sidebar_toggle.is_active())));

        // item actions carrying an id
        let add_s = |name: &str, f: Box<dyn Fn(&Rc<App>, String)>| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, param| {
                if let (Some(t), Some(id)) = (weak.upgrade(), param.and_then(|p| p.get::<String>())) {
                    f(&t, id);
                }
            });
            self.window.add_action(&action);
        };
        add_s("add-request", Box::new(|t, id| t.add_request_to(&id)));
        add_s("add-folder", Box::new(|t, id| {
            let t2 = t.clone();
            super::dialogs::prompt(&t.window, "New Folder", "Name", "New Folder", "Create", move |name| {
                let f = Folder::new(name);
                let fid = f.id.clone();
                if t2.with_container(&id, |items| items.push(Item::Folder(f))) {
                    t2.expanded.borrow_mut().insert(id.clone());
                    t2.expanded.borrow_mut().insert(fid);
                    t2.save_soon();
                    t2.refresh_tree();
                }
            });
        }));
        add_s("rename-item", Box::new(|t, id| {
            let current = t.item_name(&id).unwrap_or_default();
            let t2 = t.clone();
            super::dialogs::prompt(&t.window, "Rename", "Name", &current, "Rename", move |name| t2.rename(&id, &name));
        }));
        add_s("duplicate-item", Box::new(|t, id| t.duplicate(&id)));
        add_s("delete-item", Box::new(|t, id| {
            let name = t.item_name(&id).unwrap_or_default();
            let t2 = t.clone();
            super::dialogs::confirm(&t.window, &format!("Delete “{name}”?"), "This cannot be undone.", "Delete", move || t2.delete(&id));
        }));
        add_s("export-collection", Box::new(|t, id| t.export_collection(&id)));
        add_s("sync-openapi", Box::new(|t, id| t.sync_openapi(&id)));
        add_s("set-color", Box::new(|t, target| {
            let (id, key) = target.split_once('|').unwrap_or((&target, ""));
            let color = super::colors::is_valid(key).then(|| key.to_string());
            if t.ws.borrow_mut().set_color(id, color) {
                t.save_soon();
                t.refresh_tree();
                for tab in t.tabs.borrow().iter() {
                    t.update_tab_title(tab);
                }
            }
        }));
        add_s("edit-item", Box::new(|t, id| super::dialogs::edit_container(t, &id)));
        add_s("run", Box::new(|t, id| super::runner::show(t, &id)));
    }

    // ================================================================ tabs

    pub fn open_saved_request(self: &Rc<Self>, collection: &str, id: &str) {
        let existing = self.tabs.borrow().iter().find(|t| t.editor.collect().id == id).cloned();
        if let Some(tab) = existing {
            self.tab_view.set_selected_page(&tab.page);
            return;
        }
        let req = self.ws.borrow().collections.iter().find(|c| c.id == collection).and_then(|c| find_request(&c.items, id).cloned());
        if let Some(req) = req {
            self.open_tab(req, Some(collection.to_string()), true);
        }
    }

    pub fn open_tab(self: &Rc<Self>, req: Request, collection: Option<String>, select: bool) -> Rc<Tab> {
        let editor = RequestEditor::new();
        editor.load(&req);
        let page = self.tab_view.append(editor.widget());
        let tab = Rc::new(Tab {
            page,
            editor: editor.clone(),
            collection: RefCell::new(collection),
            saved: RefCell::new(req),
            abort: RefCell::new(None),
            chip: TabChip::new(),
        });
        self.tabs.borrow_mut().push(tab.clone());
        self.tab_strip.append(&tab.chip.root);
        // left click selects, middle click closes, right click opens the tab menu
        let menu = gtk::PopoverMenu::from_model(Some(&tab_context_menu()));
        menu.set_parent(&tab.chip.root);
        menu.set_has_arrow(false);
        menu.set_halign(gtk::Align::Start);
        let m = menu.clone();
        tab.chip.root.connect_destroy(move |_| m.unparent());
        let click = gtk::GestureClick::builder().button(0).build();
        let (tv, page) = (self.tab_view.clone(), tab.page.clone());
        click.connect_pressed(move |g, _, x, y| match g.current_button() {
            2 => tv.close_page(&page),
            3 => {
                // claim it so the title bar underneath doesn't show the window-manager menu
                g.set_state(gtk::EventSequenceState::Claimed);
                tv.set_selected_page(&page);
                menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                menu.popup();
            }
            _ => tv.set_selected_page(&page),
        });
        tab.chip.root.add_controller(click);
        let (tv, page) = (self.tab_view.clone(), tab.page.clone());
        tab.chip.close.connect_clicked(move |_| tv.close_page(&page));
        self.sync_strip();
        self.update_tab_title(&tab);
        self.update_inherit_hint(&tab);

        let weak_app = Rc::downgrade(self);
        let weak_tab = Rc::downgrade(&tab);
        editor.set_scope_provider(move || match (weak_app.upgrade(), weak_tab.upgrade()) {
            (Some(a), Some(t)) => a.scope_for(t.collection.borrow().as_deref()),
            _ => Scope::default(),
        });
        let (wa, wt) = (Rc::downgrade(self), Rc::downgrade(&tab));
        editor.connect_changed(move || {
            if let (Some(a), Some(t)) = (wa.upgrade(), wt.upgrade()) {
                a.update_tab_title(&t);
            }
        });
        let (wa, wt) = (Rc::downgrade(self), Rc::downgrade(&tab));
        editor.connect_send(move || {
            if let (Some(a), Some(t)) = (wa.upgrade(), wt.upgrade()) {
                a.send(&t);
            }
        });
        let (wa, wt) = (Rc::downgrade(self), Rc::downgrade(&tab));
        editor.connect_save(move || {
            if let (Some(a), Some(t)) = (wa.upgrade(), wt.upgrade()) {
                a.save_tab(&t);
            }
        });
        let (wa, wt) = (Rc::downgrade(self), Rc::downgrade(&tab));
        editor.connect_curl_pasted(move |mut req| {
            if let (Some(a), Some(t)) = (wa.upgrade(), wt.upgrade()) {
                let cur = t.editor.collect();
                req.id = cur.id;
                if t.collection.borrow().is_some() {
                    req.name = cur.name;
                }
                t.editor.load(&req);
                a.update_tab_title(&t);
                a.toast("Imported cURL command");
            }
        });

        if select {
            self.tab_view.set_selected_page(&tab.page);
            let e = editor.clone();
            glib::idle_add_local_once(move || e.focus_url());
        }
        tab
    }

    /// Marks the selected chip, keeps chip order in line with the TabView and scrolls it into view.
    fn sync_strip(&self) {
        let selected = self.tab_view.selected_page();
        let mut prev: Option<gtk::Widget> = None;
        for page in self.tab_view.pages().iter::<adw::TabPage>().filter_map(|p| p.ok()) {
            let Some(tab) = self.tab_for_page(&page) else { continue };
            let chip = &tab.chip.root;
            self.tab_strip.reorder_child_after(chip, prev.as_ref());
            prev = Some(chip.clone().upcast());
            if Some(&page) == selected.as_ref() {
                chip.add_css_class("selected");
                let (chip, scroll, strip) = (chip.clone(), self.strip_scroll.clone(), self.tab_strip.clone());
                glib::idle_add_local_once(move || {
                    if let Some(b) = chip.compute_bounds(&strip) {
                        let adj = scroll.hadjustment();
                        let (x, w) = (b.x() as f64, b.width() as f64);
                        if x < adj.value() {
                            adj.set_value(x);
                        } else if x + w > adj.value() + adj.page_size() {
                            adj.set_value(x + w - adj.page_size());
                        }
                    }
                });
            } else {
                chip.remove_css_class("selected");
            }
        }
    }

    fn update_tab_title(&self, tab: &Tab) {
        let r = tab.editor.collect();
        let is_dirty = tab.is_dirty() && !(tab.collection.borrow().is_none() && r.url.is_empty());
        let chip = &tab.chip;
        chip.method.set_text(match r.method {
            Method::DELETE => "DEL",
            Method::OPTIONS => "OPT",
            m => m.as_str(),
        });
        for cls in Method::ALL.iter().map(|m| m.css_class()) {
            chip.method.remove_css_class(cls);
        }
        chip.method.add_css_class(r.method.css_class());
        chip.name.set_text(&r.name);
        chip.dirty.set_visible(is_dirty);
        for cls in super::colors::all_tint_classes() {
            chip.root.remove_css_class(&cls);
        }
        if let Some(c) = self.ws.borrow().tint_for(tab.collection.borrow().as_deref(), &r.id, self.settings.borrow().color_scope) {
            chip.root.add_css_class(&super::colors::tint_class(&c));
        }
        chip.root.set_tooltip_text(Some(if r.url.is_empty() { &r.name } else { &r.url }));
        let dirty = if is_dirty { " •" } else { "" };
        tab.page.set_title(&format!("{}  {}{}", r.method.as_str(), r.name, dirty));
        tab.page.set_tooltip(&glib::markup_escape_text(if r.url.is_empty() { &r.name } else { &r.url }));
    }

    fn refresh_scopes(&self) {
        for tab in self.tabs.borrow().iter() {
            tab.editor.refresh_scope();
            self.update_inherit_hint(tab);
        }
    }

    /// Markup describing what "Inherit" resolves to for `item_id`.
    pub fn inherit_hint(&self, collection: Option<&str>, item_id: &str) -> String {
        let (auth, source) = self.ws.borrow().inherited_auth(collection, item_id);
        let kind = match auth {
            Auth::Bearer { .. } => "Bearer Token",
            Auth::Basic { .. } => "Basic Auth",
            Auth::ApiKey { .. } => "API Key",
            _ => return "Inheriting: <b>No Auth</b>. Set auth on a parent folder, the collection, or the project (<i>Project Settings</i>).".into(),
        };
        format!("Inheriting <b>{kind}</b> from {}.", glib::markup_escape_text(&source))
    }

    fn update_inherit_hint(&self, tab: &Tab) {
        let id = tab.editor.collect().id;
        tab.editor.set_inherit_hint(&self.inherit_hint(tab.collection.borrow().as_deref(), &id));
    }

    // ================================================================ send

    pub fn scope_for(&self, collection: Option<&str>) -> Scope {
        let ws = self.ws.borrow();
        let env = ws.active_environment.as_ref().and_then(|id| ws.environments.iter().find(|e| &e.id == id)).map(|e| e.variables.clone()).unwrap_or_default();
        let col = collection.and_then(|cid| ws.collections.iter().find(|c| c.id == cid)).map(|c| c.variables.clone()).unwrap_or_default();
        Scope::new(&ws.globals, &col, &env)
    }

    pub fn prepare_request(&self, req: &Request, collection: Option<&str>) -> http::Prepared {
        let scope = self.scope_for(collection);
        let auth = self.ws.borrow().effective_auth(collection, req);
        http::prepare(req, &auth, &scope)
    }

    fn prepare(&self, tab: &Tab) -> http::Prepared {
        self.prepare_request(&tab.editor.collect(), tab.collection.borrow().as_deref())
    }

    pub fn send(self: &Rc<Self>, tab: &Rc<Tab>) {
        if tab.editor.is_loading() {
            if let Some(h) = tab.abort.borrow_mut().take() {
                h.abort();
            }
            tab.editor.set_loading(false);
            tab.editor.response.set_error("Request cancelled");
            return;
        }
        let req = tab.editor.collect();
        let prepared = self.prepare(tab);
        if prepared.url.is_empty() {
            self.toast("Enter a URL first");
            tab.editor.focus_url();
            return;
        }
        let handle = RUNTIME.spawn(http::execute(prepared));
        *tab.abort.borrow_mut() = Some(handle.abort_handle());
        tab.editor.set_loading(true);

        let weak_app = Rc::downgrade(self);
        let weak_tab = Rc::downgrade(tab);
        glib::spawn_future_local(async move {
            let result = handle.await;
            let (Some(app), Some(tab)) = (weak_app.upgrade(), weak_tab.upgrade()) else { return };
            tab.abort.borrow_mut().take();
            tab.editor.set_loading(false);
            let status = match result {
                Ok(Ok(resp)) => {
                    let results = assertions::run(&req.tests, &resp);
                    tab.editor.response.set_response(&resp, &results);
                    Some(resp.status)
                }
                Ok(Err(e)) => {
                    tab.editor.response.set_error(&error_chain(&e));
                    None
                }
                Err(e) if e.is_cancelled() => return,
                Err(e) => {
                    tab.editor.response.set_error(&e.to_string());
                    None
                }
            };
            app.add_history(req, status);
        });
    }

    fn add_history(self: &Rc<Self>, req: Request, status: Option<u16>) {
        let entry = HistoryEntry { id: new_id(), timestamp: chrono::Local::now(), request: req, status, elapsed_ms: 0 };
        {
            let mut ws = self.ws.borrow_mut();
            ws.history.insert(0, entry);
            ws.history.truncate(200);
        }
        self.save_soon();
        self.refresh_history();
    }

    // ================================================================ save

    pub fn save_tab(self: &Rc<Self>, tab: &Rc<Tab>) {
        let req = tab.editor.collect();
        let cid = tab.collection.borrow().clone();
        if let Some(cid) = cid {
            let mut ws = self.ws.borrow_mut();
            if let Some(c) = ws.collections.iter_mut().find(|c| c.id == cid) {
                if let Some(slot) = find_request_mut(&mut c.items, &req.id) {
                    *slot = req.clone();
                    drop(ws);
                    *tab.saved.borrow_mut() = req;
                    self.update_tab_title(tab);
                    self.save_soon();
                    self.refresh_tree();
                    return;
                }
            }
        }
        // draft (or its collection/request was deleted): ask where to save
        let t = self.clone();
        let tab = tab.clone();
        super::dialogs::save_request(self, &req.name, move |name, target| {
            let mut req = tab.editor.collect();
            req.name = name;
            tab.editor.set_name(&req.name);
            let collection_id = match target {
                Some(id) => id,
                None => {
                    let c = Collection::new("My Collection");
                    let id = c.id.clone();
                    t.ws.borrow_mut().collections.push(c);
                    id
                }
            };
            t.with_container(&collection_id, |items| items.push(Item::Request(req.clone())));
            t.expanded.borrow_mut().insert(collection_id.clone());
            *tab.collection.borrow_mut() = Some(collection_id);
            *tab.saved.borrow_mut() = req;
            t.update_tab_title(&tab);
            t.update_inherit_hint(&tab);
            t.save_soon();
            t.refresh_tree();
            t.toast("Request saved");
        });
    }

    // ================================================================ tree ops

    /// Returns the id of the collection containing `id` (which may itself be a collection).
    pub fn collection_of(&self, id: &str) -> Option<String> {
        fn contains(items: &[Item], id: &str) -> bool {
            items.iter().any(|i| {
                i.id() == id
                    || match i {
                        Item::Folder(f) => contains(&f.items, id),
                        _ => false,
                    }
            })
        }
        self.ws.borrow().collections.iter().find(|c| c.id == id || contains(&c.items, id)).map(|c| c.id.clone())
    }

    /// Runs `f` on the item list of a collection or folder.
    fn with_container(&self, id: &str, f: impl FnOnce(&mut Vec<Item>)) -> bool {
        let mut ws = self.ws.borrow_mut();
        for c in ws.collections.iter_mut() {
            if c.id == id {
                f(&mut c.items);
                return true;
            }
            if let Some(folder) = find_folder_mut(&mut c.items, id) {
                f(&mut folder.items);
                return true;
            }
        }
        false
    }

    fn item_name(&self, id: &str) -> Option<String> {
        fn find(items: &[Item], id: &str) -> Option<String> {
            for i in items {
                if i.id() == id {
                    return Some(i.name().to_string());
                }
                if let Item::Folder(f) = i {
                    if let Some(n) = find(&f.items, id) {
                        return Some(n);
                    }
                }
            }
            None
        }
        let ws = self.ws.borrow();
        ws.collections.iter().find(|c| c.id == id).map(|c| c.name.clone()).or_else(|| ws.collections.iter().find_map(|c| find(&c.items, id)))
    }

    fn add_request_to(self: &Rc<Self>, container: &str) {
        let req = Request { name: "New Request".into(), auth: Auth::Inherit, ..Default::default() };
        if self.with_container(container, |items| items.push(Item::Request(req.clone()))) {
            self.expanded.borrow_mut().insert(container.to_string());
            if let Some(cid) = self.collection_of(container) {
                self.expanded.borrow_mut().insert(cid.clone());
                self.save_soon();
                self.refresh_tree();
                self.open_tab(req, Some(cid), true);
            }
        }
    }

    fn rename(self: &Rc<Self>, id: &str, name: &str) {
        {
            let mut ws = self.ws.borrow_mut();
            if let Some(c) = ws.collections.iter_mut().find(|c| c.id == id) {
                c.name = name.to_string();
            } else {
                for c in ws.collections.iter_mut() {
                    if rename_item(&mut c.items, id, name) {
                        break;
                    }
                }
            }
        }
        let tabs = self.tabs.borrow().clone();
        for tab in tabs {
            if tab.editor.collect().id == id {
                tab.editor.set_name(name);
                tab.saved.borrow_mut().name = name.to_string();
                self.update_tab_title(&tab);
            }
        }
        self.save_soon();
        self.refresh_tree();
    }

    fn duplicate(self: &Rc<Self>, id: &str) {
        {
            let mut ws = self.ws.borrow_mut();
            if let Some(pos) = ws.collections.iter().position(|c| c.id == id) {
                let src = ws.collections[pos].clone();
                let mut c = src.clone();
                c.id = new_id();
                c.name = format!("{} Copy", src.name);
                c.items = src.items.iter().map(|i| {
                    let mut d = duplicate_item(i);
                    match &mut d {
                        Item::Request(r) => r.name = i.name().to_string(),
                        Item::Folder(f) => f.name = i.name().to_string(),
                    }
                    d
                }).collect();
                ws.collections.insert(pos + 1, c);
            } else {
                fn dup_in(items: &mut Vec<Item>, id: &str) -> bool {
                    if let Some(pos) = items.iter().position(|i| i.id() == id) {
                        let d = duplicate_item(&items[pos]);
                        items.insert(pos + 1, d);
                        return true;
                    }
                    items.iter_mut().any(|i| match i {
                        Item::Folder(f) => dup_in(&mut f.items, id),
                        _ => false,
                    })
                }
                for c in ws.collections.iter_mut() {
                    if dup_in(&mut c.items, id) {
                        break;
                    }
                }
            }
        }
        self.save_soon();
        self.refresh_tree();
    }

    fn delete(self: &Rc<Self>, id: &str) {
        {
            let mut ws = self.ws.borrow_mut();
            let before = ws.collections.len();
            ws.collections.retain(|c| c.id != id);
            if ws.collections.len() == before {
                for c in ws.collections.iter_mut() {
                    if remove_item(&mut c.items, id).is_some() {
                        break;
                    }
                }
            }
        }
        // open tabs of deleted requests become drafts
        for tab in self.tabs.borrow().iter() {
            let cid = tab.collection.borrow().clone();
            if let Some(cid) = cid {
                let rid = tab.editor.collect().id;
                let exists = self.ws.borrow().collections.iter().any(|c| c.id == cid && find_request(&c.items, &rid).is_some());
                if !exists {
                    *tab.collection.borrow_mut() = None;
                    self.update_tab_title(tab);
                }
            }
        }
        self.save_soon();
        self.refresh_tree();
    }

    // ================================================================ import / export

    fn import_file(self: &Rc<Self>) {
        let t = self.clone();
        glib::spawn_future_local(async move {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Collections, environments, OpenAPI / Swagger (JSON, YAML)"));
            for pattern in ["*.json", "*.yaml", "*.yml"] {
                filter.add_pattern(pattern);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Import Collection, Environment or OpenAPI Spec").filters(&filters).build();
            let Ok(files) = dialog.open_multiple_future(Some(&t.window)).await else { return };
            for i in 0..files.n_items() {
                let Some(file) = files.item(i).and_downcast::<gio::File>() else { continue };
                let Some(path) = file.path() else { continue };
                let source = path.to_string_lossy().to_string();
                match std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|s| interchange::import(&s, Some(&source))) {
                    Ok(imported) => t.add_imported(imported),
                    Err(e) => t.toast(&format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy())),
                }
            }
        });
    }

    fn import_url(self: &Rc<Self>) {
        let t = self.clone();
        super::dialogs::prompt(
            &self.window,
            "Import from URL",
            "OpenAPI / Swagger URL, e.g. http://localhost:3000/docs",
            "http://localhost:3000/docs",
            "Import",
            move |url| t.import_from_url(url),
        );
    }

    pub fn import_from_url(self: &Rc<Self>, url: String) {
                let t = self.clone();
                t.toast(&format!("Fetching {url}…"));
                glib::spawn_future_local(async move {
                    let res = RUNTIME.spawn(async move { http::fetch_spec(&url).await }).await;
                    match res {
                        Ok(Ok((text, found_at))) => match interchange::import(&text, Some(&found_at)) {
                            Ok(imported) => t.add_imported(imported),
                            Err(e) => t.toast(&format!("Import failed: {e}")),
                        },
                        Ok(Err(e)) => t.toast(&format!("Import failed: {}", error_chain(&e))),
                        Err(e) => t.toast(&format!("Import failed: {e}")),
                    }
                });
    }

    /// Adds an imported collection/environment. Re-importing an OpenAPI spec that is already
    /// in the project syncs the existing collection instead of creating a duplicate.
    fn add_imported(self: &Rc<Self>, imported: interchange::Imported) {
        match imported {
            interchange::Imported::Collection(c) => {
                let existing = c.openapi_source.as_ref().and_then(|src| self.ws.borrow().collections.iter().find(|x| x.openapi_source.as_ref() == Some(src)).map(|x| x.id.clone()));
                if let Some(id) = existing {
                    self.apply_sync(&id, c);
                    return;
                }
                let count = count_requests(&c.items);
                self.toast(&format!("Imported “{}” ({count} requests)", c.name));
                self.expanded.borrow_mut().insert(c.id.clone());
                self.ws.borrow_mut().collections.push(c);
            }
            interchange::Imported::Environment(e) => {
                self.toast(&format!("Imported environment “{}”", e.name));
                self.ws.borrow_mut().environments.push(e);
            }
        }
        self.save_soon();
        self.refresh_tree();
        self.refresh_envs();
    }

    /// Re-fetches a collection's OpenAPI spec and adds endpoints that are new.
    fn sync_openapi(self: &Rc<Self>, id: &str) {
        let Some(source) = self.ws.borrow().collections.iter().find(|c| c.id == id).and_then(|c| c.openapi_source.clone()) else { return };
        let t = self.clone();
        let id = id.to_string();
        glib::spawn_future_local(async move {
            let text = if source.contains("://") {
                let src = source.clone();
                match RUNTIME.spawn(async move { http::fetch_spec(&src).await }).await {
                    Ok(Ok((text, _))) => text,
                    Ok(Err(e)) => return t.toast(&format!("Sync failed: {}", error_chain(&e))),
                    Err(e) => return t.toast(&format!("Sync failed: {e}")),
                }
            } else {
                match std::fs::read_to_string(&source) {
                    Ok(text) => text,
                    Err(e) => return t.toast(&format!("Sync failed: {source}: {e}")),
                }
            };
            match interchange::import(&text, Some(&source)) {
                Ok(interchange::Imported::Collection(fresh)) => t.apply_sync(&id, fresh),
                Ok(_) => t.toast("Sync failed: the source is no longer an OpenAPI spec"),
                Err(e) => t.toast(&format!("Sync failed: {e}")),
            }
        });
    }

    fn apply_sync(self: &Rc<Self>, id: &str, fresh: Collection) {
        let result = {
            let mut ws = self.ws.borrow_mut();
            ws.collections.iter_mut().find(|c| c.id == id).map(|c| (c.name.clone(), crate::openapi::sync(c, fresh)))
        };
        let Some((name, (added, removed))) = result else { return };
        let mut msg = match added {
            0 => format!("“{name}” is up to date"),
            1 => format!("Added 1 new endpoint to “{name}”"),
            n => format!("Added {n} new endpoints to “{name}”"),
        };
        if removed > 0 {
            msg += &format!(" · {removed} no longer in the spec");
        }
        self.toast(&msg);
        self.expanded.borrow_mut().insert(id.to_string());
        self.save_soon();
        self.refresh_tree();
    }

    fn export_collection(self: &Rc<Self>, id: &str) {
        let Some(c) = self.ws.borrow().collections.iter().find(|c| c.id == id).cloned() else { return };
        let t = self.clone();
        glib::spawn_future_local(async move {
            let dialog = gtk::FileDialog::builder().title("Export Collection").initial_name(format!("{}.collection.json", c.name)).build();
            if let Ok(file) = dialog.save_future(Some(&t.window)).await {
                if let Some(path) = file.path() {
                    match std::fs::write(&path, interchange::export_collection(&c)) {
                        Ok(_) => t.toast("Collection exported (v2.1 JSON)"),
                        Err(e) => t.toast(&format!("Export failed: {e}")),
                    }
                }
            }
        });
    }

    // ================================================================ sidebar rendering

    pub fn refresh_tree(self: &Rc<Self>) {
        while let Some(child) = self.tree.first_child() {
            self.tree.remove(&child);
        }
        self.tree_rows.borrow_mut().clear();
        let query = self.filter.text().to_lowercase();
        let ws = self.ws.borrow();

        if ws.collections.is_empty() {
            let row = gtk::ListBoxRow::builder().activatable(false).selectable(false).build();
            let l = gtk::Label::builder().label("No collections yet.\nCreate one with + or import a collection or OpenAPI spec.").justify(gtk::Justification::Center).wrap(true).margin_top(24).build();
            l.add_css_class("dim-label");
            row.set_child(Some(&l));
            self.tree.append(&row);
            self.tree_rows.borrow_mut().push(TreeRow { id: String::new(), collection: String::new(), kind: TreeKind::Collection });
            return;
        }

        fn matches(item: &Item, q: &str) -> bool {
            match item {
                Item::Request(r) => r.name.to_lowercase().contains(q) || r.url.to_lowercase().contains(q),
                Item::Folder(f) => f.name.to_lowercase().contains(q) || f.items.iter().any(|i| matches(i, q)),
            }
        }

        for c in &ws.collections {
            if !query.is_empty() && !c.items.iter().any(|i| matches(i, &query)) && !c.name.to_lowercase().contains(&query) {
                continue;
            }
            let open = !query.is_empty() || self.expanded.borrow().contains(&c.id);
            let color = c.color.as_deref();
            let scope = self.settings.borrow().color_scope;
            let child_tint = if scope == ColorScope::RowOnly { None } else { color };
            self.append_tree_row(&RowSpec {
                depth: 0,
                id: &c.id,
                cid: &c.id,
                name: &c.name,
                kind: TreeKind::Collection,
                method: None,
                open,
                syncable: c.openapi_source.is_some(),
                color,
                tint: color,
            });
            if open {
                self.append_items(&c.items, 1, &c.id, child_tint, scope, &query, &matches);
            }
        }
    }

    /// `tint` is what requests at this level get; folders compute theirs from `scope`.
    #[allow(clippy::too_many_arguments)]
    fn append_items(&self, items: &[Item], depth: i32, cid: &str, tint: Option<&str>, scope: ColorScope, q: &str, matches: &dyn Fn(&Item, &str) -> bool) {
        for item in items {
            if !q.is_empty() && !matches(item, q) {
                continue;
            }
            match item {
                Item::Folder(f) => {
                    let open = !q.is_empty() || self.expanded.borrow().contains(&f.id);
                    let own = f.color.as_deref();
                    // (tint of the folder row, tint handed to its contents)
                    let (row_tint, child_tint) = match scope {
                        ColorScope::Cascade => (own.or(tint), own.or(tint)),
                        ColorScope::Direct => (own, own),
                        ColorScope::RowOnly => (own, None),
                    };
                    self.append_tree_row(&RowSpec {
                        depth,
                        id: &f.id,
                        cid,
                        name: &f.name,
                        kind: TreeKind::Folder,
                        method: None,
                        open,
                        syncable: false,
                        color: own,
                        tint: row_tint,
                    });
                    if open {
                        self.append_items(&f.items, depth + 1, cid, child_tint, scope, q, matches);
                    }
                }
                Item::Request(r) => self.append_tree_row(&RowSpec {
                    depth,
                    id: &r.id,
                    cid,
                    name: &r.name,
                    kind: TreeKind::Request,
                    method: Some(r.method),
                    open: false,
                    syncable: false,
                    color: None,
                    tint,
                }),
            }
        }
    }

    fn append_tree_row(&self, spec: &RowSpec) {
        let RowSpec { depth, id, cid, name, method, open, syncable, color, tint, .. } = *spec;
        let kind = spec.kind.clone();
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        hbox.set_margin_start(depth * 14);

        match kind {
            TreeKind::Collection | TreeKind::Folder => {
                let arrow = gtk::Image::from_icon_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" });
                arrow.add_css_class("dim-label");
                hbox.append(&arrow);
                let icon = gtk::Image::from_icon_name(if matches!(kind, TreeKind::Collection) { "view-list-bullet-symbolic" } else { "folder-symbolic" });
                icon.add_css_class("tree-icon");
                if let Some(c) = color {
                    icon.add_css_class(&format!("color-{c}"));
                }
                hbox.append(&icon);
            }
            TreeKind::Request => {
                let m = method.unwrap_or_default();
                let label = gtk::Label::new(Some(match m {
                    Method::DELETE => "DEL",
                    Method::OPTIONS => "OPT",
                    other => other.as_str(),
                }));
                label.add_css_class("method-badge");
                label.add_css_class(m.css_class());
                label.set_width_chars(5);
                label.set_xalign(1.0);
                hbox.append(&label);
            }
        }
        let label = gtk::Label::builder().label(name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
        if matches!(kind, TreeKind::Collection) {
            label.add_css_class("heading");
        }
        hbox.append(&label);

        let menu = gio::Menu::new();
        let item = |label: &str, action: &str| {
            let mi = gio::MenuItem::new(Some(label), None);
            mi.set_action_and_target_value(Some(action), Some(&id.to_variant()));
            mi
        };
        let s1 = gio::Menu::new();
        let s2 = gio::Menu::new();
        let s3 = gio::Menu::new();
        match kind {
            TreeKind::Collection | TreeKind::Folder => {
                s1.append_item(&item("Add Request", "win.add-request"));
                s1.append_item(&item("Add Folder", "win.add-folder"));
                s1.append_item(&item(if matches!(kind, TreeKind::Collection) { "Run Collection" } else { "Run Folder" }, "win.run"));
                s2.append_item(&item(if matches!(kind, TreeKind::Collection) { "Variables, Auth & Docs…" } else { "Auth & Docs…" }, "win.edit-item"));
                s2.append_item(&item("Rename…", "win.rename-item"));
                s2.append_item(&item("Duplicate", "win.duplicate-item"));
                if matches!(kind, TreeKind::Collection) {
                    s2.append_item(&item("Export…", "win.export-collection"));
                }
                if syncable {
                    s1.append_item(&item("Sync with OpenAPI Spec", "win.sync-openapi"));
                }
            }
            TreeKind::Request => {
                s2.append_item(&item("Rename…", "win.rename-item"));
                s2.append_item(&item("Duplicate", "win.duplicate-item"));
            }
        }
        s3.append_item(&item("Delete", "win.delete-item"));
        menu.append_section(None, &s1);
        menu.append_section(None, &s2);
        let has_color_picker = !matches!(kind, TreeKind::Request);
        if has_color_picker {
            let picker = gio::MenuItem::new(None, None);
            picker.set_attribute_value("custom", Some(&"color-picker".to_variant()));
            let section = gio::Menu::new();
            section.append_item(&picker);
            menu.append_section(Some("Color"), &section);
        }
        menu.append_section(None, &s3);

        let more = gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&menu).valign(gtk::Align::Center).build();
        more.add_css_class("flat");
        more.add_css_class("tree-more");
        hbox.append(&more);
        if has_color_picker {
            if let Some(popover) = more.popover().and_downcast::<gtk::PopoverMenu>() {
                popover.add_child(&color_picker(id, color, &popover), "color-picker");
            }
        }

        let row = gtk::ListBoxRow::builder().child(&hbox).build();
        if let Some(t) = tint {
            row.add_css_class(&super::colors::tint_class(t));
            row.add_css_class("tinted");
        }
        // right click opens the same menu
        let gesture = gtk::GestureClick::builder().button(3).build();
        let more2 = more.clone();
        gesture.connect_pressed(move |_, _, _, _| more2.popup());
        row.add_controller(gesture);

        self.tree.append(&row);
        self.tree_rows.borrow_mut().push(TreeRow { id: id.to_string(), collection: cid.to_string(), kind });
    }

    pub fn refresh_history(&self) {
        while let Some(child) = self.history.first_child() {
            self.history.remove(&child);
        }
        let ws = self.ws.borrow();
        for entry in &ws.history {
            let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let m = gtk::Label::new(Some(entry.request.method.as_str()));
            m.add_css_class("method-badge");
            m.add_css_class(entry.request.method.css_class());
            h.append(&m);
            let url = gtk::Label::builder().label(&entry.request.url).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
            h.append(&url);
            v.append(&h);
            let status = entry.status.map(|s| s.to_string()).unwrap_or_else(|| "Error".into());
            let sub = gtk::Label::builder().label(format!("{} · {}", status, entry.timestamp.format("%b %d, %H:%M:%S"))).xalign(0.0).build();
            sub.add_css_class("caption");
            sub.add_css_class("dim-label");
            v.append(&sub);
            v.set_margin_top(4);
            v.set_margin_bottom(4);
            self.history.append(&gtk::ListBoxRow::builder().child(&v).tooltip_text(&entry.request.url).build());
        }
    }

    pub fn refresh_envs(self: &Rc<Self>) {
        self.updating_envs.set(true);
        let ws = self.ws.borrow();
        let mut names = vec!["No Environment".to_string()];
        let mut ids = vec![None];
        for e in &ws.environments {
            names.push(e.name.clone());
            ids.push(Some(e.id.clone()));
        }
        let selected = ids.iter().position(|i| *i == ws.active_environment).unwrap_or(0);
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        self.env_dropdown.set_model(Some(&gtk::StringList::new(&refs)));
        self.env_dropdown.set_selected(selected as u32);
        *self.env_ids.borrow_mut() = ids;
        drop(ws);
        self.updating_envs.set(false);
        self.refresh_scopes();
    }

    // ================================================================ session

    fn save_session(&self) {
        let tabs: Vec<serde_json::Value> = self
            .tab_view
            .pages()
            .iter::<adw::TabPage>()
            .filter_map(|p| p.ok())
            .filter_map(|p| self.tab_for_page(&p))
            .map(|t| serde_json::json!({ "collection": *t.collection.borrow(), "request": t.editor.collect(), "saved": *t.saved.borrow() }))
            .collect();
        let selected = self.tab_view.selected_page().map(|p| self.tab_view.page_position(&p)).unwrap_or(0);
        let expanded: Vec<String> = self.expanded.borrow().iter().cloned().collect();
        let v = serde_json::json!({ "tabs": tabs, "selected": selected, "expanded": expanded });
        let _ = std::fs::write(storage::session_path(&self.ws.borrow().id), v.to_string());
    }

    fn save_geometry(&self) {
        let (w, h) = self.window.default_size();
        let v = serde_json::json!({ "width": w, "height": h, "maximized": self.window.is_maximized() });
        let _ = std::fs::write(storage::window_path(), v.to_string());
    }

    fn restore_geometry(&self) {
        let Ok(text) = std::fs::read_to_string(storage::window_path()) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if let (Some(w), Some(h)) = (v["width"].as_i64(), v["height"].as_i64()) {
            self.window.set_default_size(w as i32, h as i32);
        }
        if v["maximized"].as_bool() == Some(true) {
            self.window.maximize();
        }
    }

    fn restore_session(self: &Rc<Self>) {
        let id = self.ws.borrow().id.clone();
        let Ok(text) = std::fs::read_to_string(storage::session_path(&id)) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if let Some(exp) = v["expanded"].as_array() {
            self.expanded.borrow_mut().extend(exp.iter().filter_map(|e| e.as_str().map(String::from)));
            self.refresh_tree();
        }
        for tab in v["tabs"].as_array().cloned().unwrap_or_default() {
            let Ok(req) = serde_json::from_value::<Request>(tab["request"].clone()) else { continue };
            let collection = tab["collection"].as_str().map(String::from).filter(|cid| {
                self.ws.borrow().collections.iter().any(|c| &c.id == cid && find_request(&c.items, &req.id).is_some())
            });
            let saved = serde_json::from_value::<Request>(tab["saved"].clone()).unwrap_or_else(|_| req.clone());
            let t = self.open_tab(saved, collection, false);
            t.editor.load(&req);
            self.update_tab_title(&t);
        }
        let sel = v["selected"].as_i64().unwrap_or(0) as i32;
        if sel < self.tab_view.n_pages() {
            self.tab_view.set_selected_page(&self.tab_view.nth_page(sel));
        }
    }

    /// Debug aid: opens the ⋮ menu of the n-th sidebar row and returns its popover.
    #[cfg(debug_assertions)]
    pub fn debug_popup_row(&self, index: i32) -> Option<gtk::Popover> {
        if index < 0 {
            self.project_btn.popup();
            return self.project_btn.popover();
        }
        if index >= 1000 {
            // tab chip menus: 1000 + tab position
            let page = self.tab_view.nth_page(index - 1000);
            self.tab_view.set_selected_page(&page);
            let tab = self.tab_for_page(&page)?;
            let mut child = tab.chip.root.first_child();
            while let Some(c) = child {
                if let Some(pop) = c.downcast_ref::<gtk::PopoverMenu>() {
                    pop.popup();
                    return Some(pop.clone().upcast());
                }
                child = c.next_sibling();
            }
            return None;
        }
        let row = self.tree.row_at_index(index)?;
        let hbox = row.child()?;
        let more = hbox.last_child().and_downcast::<gtk::MenuButton>()?;
        more.popup();
        more.popover()
    }

    // ================================================================ projects

    fn refresh_projects(&self) {
        let idx = self.projects.borrow();
        let name = self.ws.borrow().name.clone();
        self.project_label.set_text(&name);
        self.window.set_title(Some(&format!("{name} - Pigeon")));

        super::project_icon::apply(&self.project_icon, self.ws.borrow().icon.as_ref(), 16);

        let menu = gio::Menu::new();
        let list = gio::Menu::new();
        let custom = gio::MenuItem::new(None, None);
        custom.set_attribute_value("custom", Some(&"project-list".to_variant()));
        list.append_item(&custom);
        menu.append_section(Some("Projects"), &list);
        let manage = gio::Menu::new();
        manage.append(Some("New Project…"), Some("win.new-project"));
        manage.append(Some("Project Settings & Auth…"), Some("win.project-settings"));
        manage.append(Some("Rename Project…"), Some("win.rename-project"));
        manage.append(Some("Delete Project…"), Some("win.delete-project"));
        menu.append_section(None, &manage);
        self.project_btn.set_menu_model(Some(&menu));
        if let Some(popover) = self.project_btn.popover().and_downcast::<gtk::PopoverMenu>() {
            let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for p in &idx.projects {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.append(&super::project_icon::image(p.icon.as_ref(), 16));
                row.append(&gtk::Label::builder().label(&p.name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(28).build());
                let check = gtk::Image::from_icon_name("object-select-symbolic");
                check.set_opacity(if p.id == idx.active { 1.0 } else { 0.0 });
                row.append(&check);
                let btn = gtk::Button::builder().child(&row).action_name("win.switch-project").action_target(&p.id.to_variant()).build();
                btn.add_css_class("flat");
                btn.add_css_class("project-row");
                let pop = popover.clone();
                btn.connect_clicked(move |_| pop.popdown());
                rows.append(&btn);
            }
            popover.add_child(&rows, "project-list");
        }

        if let Some(action) = self.window.lookup_action("switch-project") {
            action.change_state(&idx.active.to_variant());
        }
        if let Some(action) = self.window.lookup_action("delete-project").and_downcast::<gio::SimpleAction>() {
            action.set_enabled(idx.projects.len() > 1);
        }
    }

    pub fn switch_project(self: &Rc<Self>, id: &str) {
        if self.ws.borrow().id == id || !self.projects.borrow().projects.iter().any(|p| p.id == id) {
            return;
        }
        // persist the current project, then close its tabs without prompting (they live on in its session)
        self.save_session();
        self.save_now();
        self.closing_all.set(true);
        let pages: Vec<adw::TabPage> = self.tab_view.pages().iter::<adw::TabPage>().filter_map(|p| p.ok()).collect();
        for page in pages {
            self.tab_view.close_page(&page);
        }
        self.closing_all.set(false);

        *self.ws.borrow_mut() = storage::load_project(id);
        self.projects.borrow_mut().active = id.to_string();
        storage::save_index(&self.projects.borrow());
        self.expanded.borrow_mut().clear();
        self.filter.set_text("");
        self.refresh_projects();
        self.refresh_tree();
        self.refresh_history();
        self.refresh_envs();
        self.restore_session();
        let name = self.ws.borrow().name.clone();
        self.toast(&format!("Switched to project “{name}”"));
    }

    pub fn create_project(self: &Rc<Self>, name: &str) {
        let ws = Workspace::new(name);
        storage::save(&ws);
        self.projects.borrow_mut().projects.push(storage::ProjectRef { id: ws.id.clone(), name: ws.name.clone(), icon: None });
        storage::save_index(&self.projects.borrow());
        self.switch_project(&ws.id);
    }

    pub fn rename_project(self: &Rc<Self>, name: &str) {
        let id = {
            let mut ws = self.ws.borrow_mut();
            ws.name = name.to_string();
            ws.id.clone()
        };
        if let Some(p) = self.projects.borrow_mut().projects.iter_mut().find(|p| p.id == id) {
            p.name = name.to_string();
        }
        storage::save_index(&self.projects.borrow());
        self.save_now();
        self.refresh_projects();
        self.refresh_scopes();
    }

    fn delete_current_project(self: &Rc<Self>) {
        let id = self.ws.borrow().id.clone();
        let next = self.projects.borrow().projects.iter().find(|p| p.id != id).map(|p| p.id.clone());
        let Some(next) = next else { return };
        self.switch_project(&next);
        storage::delete_project(&id);
        self.projects.borrow_mut().projects.retain(|p| p.id != id);
        storage::save_index(&self.projects.borrow());
        self.refresh_projects();
    }

    pub fn project_changed(self: &Rc<Self>) {
        let (id, icon) = {
            let ws = self.ws.borrow();
            (ws.id.clone(), ws.icon.clone())
        };
        if let Some(p) = self.projects.borrow_mut().projects.iter_mut().find(|p| p.id == id) {
            p.icon = icon;
        }
        storage::save_index(&self.projects.borrow());
        self.save_soon();
        self.refresh_projects();
        self.refresh_scopes();
    }

    /// Shows the chosen app icon variant in the app's own UI.
    pub fn refresh_app_icon(&self) {
        let variant = super::app_icon::variant(&self.settings.borrow().app_icon);
        self.empty_page.set_paintable(variant.texture().as_ref());
    }

    pub fn settings_changed(self: &Rc<Self>) {
        storage::save_settings(&self.settings.borrow());
        self.refresh_tree();
        for tab in self.tabs.borrow().iter() {
            self.update_tab_title(tab);
        }
    }

    pub fn environments_changed(self: &Rc<Self>) {
        self.save_soon();
        self.refresh_envs();
    }

    pub fn collections_changed(self: &Rc<Self>) {
        self.save_soon();
        self.refresh_tree();
        self.refresh_scopes();
    }
}
