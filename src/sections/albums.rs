//! Albums: a cover grid; an album opens into its own view with its songs.

use super::{library_stack, scan_banner};
use crate::library::art::Cover;
use crate::library::store::{self, Album};
use crate::tracklist::{Col, Options, TrackTable};
use crate::views::{self, AlbumGrid};
use crate::widgets::{self, Page};
use crate::{fmt, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Ui {
    inner: gtk::Stack,
    detail: gtk::Box,
    open_key: String,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let inner = gtk::Stack::new();
    inner.set_transition_type(gtk::StackTransitionType::Crossfade);
    inner.set_transition_duration(if crate::prefs::get().reduce_motion { 0 } else { 160 });
    let grid = AlbumGrid::new(|a| open(&a));
    inner.add_named(&grid.root, Some("grid"));
    let detail = widgets::vbox(0);
    detail.set_vexpand(true);
    inner.add_named(&detail, Some("detail"));
    page.body.append(&library_stack(&inner));
    grid.set(&store::albums());
    let g = grid.clone();
    store::subscribe(&inner, move |c| {
        if c != store::Change::Library {
            return;
        }
        g.set(&store::albums());
        // Re-open the album being viewed with fresh data, or fall back to the grid.
        let key = UI.with(|u| u.borrow().as_ref().map(|u| u.open_key.clone())).unwrap_or_default();
        if !key.is_empty() {
            match store::album(&key) {
                Some(a) => open(&a),
                None => back(),
            }
        }
    });
    UI.with(|u| *u.borrow_mut() = Some(Ui { inner, detail, open_key: String::new() }));
}

fn back() {
    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.inner.set_visible_child_name("grid");
            u.open_key.clear();
        }
    });
}

/// Go to an album from anywhere.
pub fn show(key: &str) {
    window::navigate("albums");
    if let Some(a) = store::album(key) {
        open(&a);
    }
}

fn open(album: &Rc<Album>) {
    let Some(detail) = UI.with(|u| u.borrow().as_ref().map(|u| u.detail.clone())) else { return };
    while let Some(c) = detail.first_child() {
        detail.remove(&c);
    }
    detail.append(&views::back_button("Albums", back));

    let cover = Cover::new(if window::narrow() { 140 } else { 200 }, false);
    cover.set_key(&album.art);
    let mut meta = Vec::new();
    if let Some(y) = album.year {
        meta.push(y.to_string());
    }
    meta.push(fmt::count(album.tracks.len(), "song", "songs"));
    meta.push(fmt::total(album.duration));
    let a = album.clone();
    let actions = views::play_buttons(move || a.tracks.iter().map(|t| t.path.clone()).collect());
    let (header, _) = views::detail_header(
        Some(&cover),
        "Album",
        &album.title,
        &views::artist_link(&album.artist),
        &meta.join(" · "),
        &actions,
    );
    detail.append(&header);

    let various = album.tracks.iter().any(|t| t.artist != album.artist);
    let cols: &'static [Col] = if various {
        &[Col::Num, Col::Title, Col::Artist, Col::Time, Col::Plays]
    } else {
        &[Col::Num, Col::Title, Col::Time, Col::Plays]
    };
    let discs: std::collections::HashSet<u32> = album.tracks.iter().map(|t| t.disc_no.unwrap_or(1)).collect();
    let table = TrackTable::new(Options { cols, sortable: false, disc_sections: discs.len() > 1, ..Default::default() });
    table.set(&album.tracks);
    detail.append(&table.root);

    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.open_key = album.key.clone();
            u.inner.set_visible_child_name("detail");
        }
    });
}
