//! Songs: the whole library in one sortable table.

use super::{library_stack, scan_banner};
use crate::library::store;
use crate::tracklist::{Options, TrackTable};
use crate::widgets::{self, Page};
use crate::{fmt, views};
use gtk::prelude::*;

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let content = widgets::vbox(12);
    let toolbar = widgets::hbox(12);
    toolbar.add_css_class("list-toolbar");
    let summary = widgets::label("", "dim");
    summary.add_css_class("mono");
    summary.set_hexpand(true);
    toolbar.append(&summary);
    let table = TrackTable::new(Options::default());
    let t = table.clone();
    toolbar.append(&views::play_buttons(move || t.paths()));
    content.append(&toolbar);
    content.append(&table.root);
    page.body.append(&library_stack(&content));

    let refresh = move || {
        let tracks = store::tracks();
        let secs: f64 = tracks.iter().map(|t| t.duration).sum();
        summary.set_text(&format!("{} · {}", fmt::count(tracks.len(), "song", "songs"), fmt::total(secs)));
        table.set(&tracks);
    };
    refresh();
    store::subscribe(&content, move |c| {
        if c == store::Change::Library {
            refresh();
        }
    });
}
