//! Collection runner: sends every request in a collection/folder in order and reports results.

use super::window::App;
use crate::http::{self, RUNTIME};
use crate::model::*;
use crate::assertions;
use adw::prelude::*;
use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;

fn collect(items: &[Item], out: &mut Vec<Request>) {
    for i in items {
        match i {
            Item::Request(r) => out.push(r.clone()),
            Item::Folder(f) => collect(&f.items, out),
        }
    }
}

pub fn show(app: &Rc<App>, id: &str) {
    let Some(cid) = app.collection_of(id) else { return };
    let (title, requests) = {
        let ws = app.ws.borrow();
        let Some(c) = ws.collections.iter().find(|c| c.id == cid) else { return };
        let mut reqs = vec![];
        if c.id == id {
            collect(&c.items, &mut reqs);
            (c.name.clone(), reqs)
        } else {
            fn find<'a>(items: &'a [Item], id: &str) -> Option<&'a Folder> {
                items.iter().find_map(|i| match i {
                    Item::Folder(f) if f.id == id => Some(f),
                    Item::Folder(f) => find(&f.items, id),
                    _ => None,
                })
            }
            let Some(f) = find(&c.items, id) else { return };
            collect(&f.items, &mut reqs);
            (f.name.clone(), reqs)
        }
    };

    let iterations = adw::SpinRow::with_range(1.0, 1000.0, 1.0);
    iterations.set_title("Iterations");
    let delay = adw::SpinRow::with_range(0.0, 60_000.0, 100.0);
    delay.set_title("Delay between requests (ms)");
    let stop_on_fail = adw::SwitchRow::builder().title("Stop on first failure").build();
    let opts = gtk::ListBox::new();
    opts.add_css_class("boxed-list");
    opts.set_selection_mode(gtk::SelectionMode::None);
    opts.append(&iterations);
    opts.append(&delay);
    opts.append(&stop_on_fail);

    let summary = gtk::Label::builder().label(format!("{} requests", requests.len())).xalign(0.0).build();
    summary.add_css_class("heading");
    let results = gtk::ListBox::new();
    results.add_css_class("boxed-list");
    results.set_selection_mode(gtk::SelectionMode::None);
    results.set_valign(gtk::Align::Start);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.set_margin_top(12);
    body.set_margin_bottom(16);
    body.append(&opts);
    body.append(&summary);
    body.append(&results);
    let scroll = gtk::ScrolledWindow::builder().child(&body).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

    let run = gtk::Button::with_label("Run");
    run.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&run);
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&header);
    tv.set_content(Some(&scroll));
    let dialog = adw::Dialog::builder().title(format!("Run “{title}”")).content_width(720).content_height(640).child(&tv).build();

    let running = Rc::new(Cell::new(false));
    let cancelled = Rc::new(Cell::new(false));
    let app2 = app.clone();
    let c2 = cancelled.clone();
    dialog.connect_closed(move |_| c2.set(true));

    run.connect_clicked(move |btn| {
        if running.get() {
            cancelled.set(true);
            return;
        }
        while let Some(c) = results.first_child() {
            results.remove(&c);
        }
        cancelled.set(false);
        running.set(true);
        btn.set_label("Stop");
        btn.remove_css_class("suggested-action");
        btn.add_css_class("destructive-action");

        let app = app2.clone();
        let requests = requests.clone();
        let cid = cid.clone();
        let results = results.clone();
        let summary = summary.clone();
        let btn = btn.clone();
        let running = running.clone();
        let cancelled = cancelled.clone();
        let n_iter = iterations.value() as usize;
        let delay_ms = delay.value() as u64;
        let stop_on_fail = stop_on_fail.is_active();
        glib::spawn_future_local(async move {
            let (mut passed, mut failed, mut total_ms) = (0usize, 0usize, 0u128);
            'outer: for iter in 0..n_iter {
                for req in &requests {
                    if cancelled.get() {
                        break 'outer;
                    }
                    let prepared = app.prepare_request(req, Some(&cid));
                    let res = RUNTIME.spawn(http::execute(prepared)).await;
                    let row = adw::ActionRow::builder().title(glib::markup_escape_text(&format!("{}  {}", req.method.as_str(), req.name))).build();
                    if n_iter > 1 {
                        row.set_subtitle(&format!("Iteration {}", iter + 1));
                    }
                    let ok = match res {
                        Ok(Ok(resp)) => {
                            total_ms += resp.elapsed.as_millis();
                            let tests = assertions::run(&req.tests, &resp);
                            let t_pass = tests.iter().filter(|t| t.passed).count();
                            let status = gtk::Label::new(Some(&format!("{} · {}", resp.status, http::human_duration(resp.elapsed))));
                            status.add_css_class(super::util::status_css(resp.status));
                            row.add_suffix(&status);
                            if !tests.is_empty() {
                                let tl = gtk::Label::new(Some(&format!("{t_pass}/{} tests", tests.len())));
                                tl.add_css_class(if t_pass == tests.len() { "test-pass" } else { "test-fail" });
                                row.add_suffix(&tl);
                                let failures: Vec<String> = tests.iter().filter(|t| !t.passed).map(|t| format!("✗ {} ({})", t.name, t.message)).collect();
                                if !failures.is_empty() {
                                    row.set_subtitle(&glib::markup_escape_text(&failures.join("\n")));
                                }
                            }
                            resp.status < 400 && t_pass == tests.len()
                        }
                        Ok(Err(e)) => {
                            row.set_subtitle(&glib::markup_escape_text(&e.to_string()));
                            let l = gtk::Label::new(Some("Error"));
                            l.add_css_class("test-fail");
                            row.add_suffix(&l);
                            false
                        }
                        Err(_) => false,
                    };
                    if ok { passed += 1 } else { failed += 1 }
                    results.append(&row);
                    summary.set_text(&format!("{passed} passed · {failed} failed · {total_ms} ms total"));
                    if !ok && stop_on_fail {
                        break 'outer;
                    }
                    if delay_ms > 0 {
                        glib::timeout_future(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
            running.set(false);
            btn.set_label("Run Again");
            btn.remove_css_class("destructive-action");
            btn.add_css_class("suggested-action");
        });
    });

    dialog.present(Some(&app.window));
}
