//! Hand ("pointer") cursor over clickable widgets.
//!
//! GTK keeps the arrow cursor on buttons (GNOME HIG) and its CSS has no `cursor` property,
//! so a single emission hook on the `map` signal sets the cursor on every clickable widget
//! as it appears - including ones inside dialogs, popovers and rebuilt lists.

use gtk::glib;
use gtk::glib::translate::*;
use gtk::prelude::*;

fn is_clickable(w: &gtk::Widget) -> bool {
    if w.is::<gtk::Button>() || w.is::<gtk::CheckButton>() || w.is::<gtk::Switch>() {
        return true;
    }
    // menu items in popovers
    if w.type_().name() == "GtkModelButton" {
        return true;
    }
    // request tabs are boxes with a click gesture
    if w.has_css_class("tab-chip") {
        return true;
    }
    // activatable list rows (sidebar tree, history, preference rows); key/value editor rows aren't
    if let Some(row) = w.downcast_ref::<gtk::ListBoxRow>() {
        let in_editor = row.parent().is_some_and(|p| p.has_css_class("kv-list"));
        return row.is_activatable() && !in_editor;
    }
    false
}

unsafe extern "C" fn on_map(
    _hint: *mut glib::gobject_ffi::GSignalInvocationHint,
    n_params: u32,
    params: *const glib::gobject_ffi::GValue,
    _data: glib::ffi::gpointer,
) -> glib::ffi::gboolean {
    if n_params > 0 && !params.is_null() {
        let object = unsafe { glib::gobject_ffi::g_value_get_object(params) };
        if !object.is_null() {
            let widget: gtk::Widget = unsafe { from_glib_none(object as *mut gtk::ffi::GtkWidget) };
            if widget.cursor().is_none() && is_clickable(&widget) {
                widget.set_cursor_from_name(Some("pointer"));
            }
        }
    }
    glib::ffi::GTRUE // keep the hook installed
}

/// Installs the hook once for the whole app.
pub fn install() {
    unsafe {
        // signals are registered when the class is initialized, which may not have happened yet
        let widget_type = gtk::Widget::static_type().into_glib();
        glib::gobject_ffi::g_type_class_ref(widget_type); // intentionally kept for the app's lifetime
        let signal = glib::gobject_ffi::g_signal_lookup(c"map".as_ptr(), widget_type);
        if signal != 0 {
            glib::gobject_ffi::g_signal_add_emission_hook(signal, 0, Some(on_map), std::ptr::null_mut(), None);
        }
    }
}
