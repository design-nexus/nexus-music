//! A page shown as a card over the window instead of in the sidebar (the
//! equalizer), with the same look as the settings dialog.

use crate::widgets::{self, Page};
use crate::window;
use gtk::prelude::*;
use std::cell::RefCell;

struct Panel {
    veil: gtk::Box,
    card: gtk::Box,
    scroll: gtk::ScrolledWindow,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

/// Open the card, building the page the first time.
pub fn open(id: &str, title: &str, build: fn(&Page)) {
    let Some(overlay) = window::overlay() else { return };
    if PANEL.with(|p| p.borrow().is_none()) {
        let panel = make(id, title, build);
        overlay.add_overlay(&panel.veil);
        PANEL.with(|p| *p.borrow_mut() = Some(panel));
    }
    PANEL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.veil.set_visible(true);
        }
    });
    if let Some(w) = window::window() {
        fit(&w);
    }
}

pub fn is_open() -> bool {
    PANEL.with(|p| p.borrow().as_ref().is_some_and(|p| p.veil.is_visible()))
}

pub fn close() {
    PANEL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p.veil.set_visible(false);
        }
    });
}

/// Size the card to the window: as wide as it allows, up to a limit, and as tall
/// as its page (scrolling when the window is shorter).
pub fn fit(window: &gtk::ApplicationWindow) {
    let (w, h) = (window.width(), window.height());
    if w <= 0 || h <= 0 {
        return;
    }
    PANEL.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            let width = (w - 48).clamp(0, 860);
            if p.card.width_request() != width {
                p.card.set_size_request(width, -1);
            }
            // The header takes about 64 px of the height.
            p.scroll.set_max_content_height((h - 48 - 64).max(120));
        }
    });
}

fn make(id: &str, title: &str, build: fn(&Page)) -> Panel {
    let page = widgets::page(id);
    build(&page);
    // The card's header names the page; its first group needn't say it again.
    if let Some(heading) = page.body.first_child().and_then(|g| g.first_child()).filter(|t| t.has_css_class("group-title")) {
        heading.set_visible(false);
    }

    let veil = gtk::Box::new(gtk::Orientation::Vertical, 0);
    veil.add_css_class("settings-veil");
    veil.set_hexpand(true);
    veil.set_vexpand(true);
    veil.set_visible(false);
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("settings-dialog");
    card.add_css_class("panel-dialog");
    card.set_halign(gtk::Align::Center);
    card.set_valign(gtk::Align::Center);
    card.set_vexpand(true);
    card.set_overflow(gtk::Overflow::Hidden);
    veil.append(&card);

    // A click on the dimmed window around the card closes it.
    let click = gtk::GestureClick::new();
    let c = card.clone();
    click.connect_pressed(move |g, _, x, y| {
        let Some(veil) = g.widget() else { return };
        let inside = c.compute_bounds(&veil).is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
        if !inside {
            close();
        }
    });
    veil.add_controller(click);

    let head = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    head.add_css_class("dialog-head");
    let t = widgets::label(title, "dialog-title");
    t.set_hexpand(true);
    head.append(&t);
    let x = widgets::bar_button("window-close-symbolic", "Close (Esc)");
    x.connect_clicked(|_| close());
    head.append(&x);
    card.append(&head);

    // The page brings its own scrolled window; let it be as tall as its content.
    let scroll = page.root.clone();
    scroll.set_propagate_natural_height(true);
    scroll.set_vexpand(false);
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.add_css_class("settings-dialog-main");
    main.append(&scroll);
    card.append(&main);
    Panel { veil, card, scroll }
}
