//! Lists made from what the library already knows: albums recently added,
//! and songs recently or most played.

use super::{library_stack, scan_banner};
use crate::library::store;
use crate::library::Track;
use crate::tracklist::{Col, Options, TrackTable};
use crate::views::{self, AlbumGrid};
use crate::widgets::{self, Page};
use crate::fmt;
use gtk::prelude::*;
use std::rc::Rc;

const ALBUMS: usize = 60;
const SONGS: usize = 200;

pub fn build_added(page: &Page) {
    page.body.append(&scan_banner());
    let grid = AlbumGrid::new(|a| super::albums::show(&a.key));
    page.body.append(&library_stack(&grid.root));
    let refresh = {
        let grid = grid.clone();
        move || grid.set(&store::sorted_albums("added").into_iter().take(ALBUMS).collect::<Vec<_>>())
    };
    refresh();
    store::subscribe(&grid.root, move |c| {
        if c == store::Change::Library {
            refresh();
        }
    });
}

pub fn build_recent(page: &Page) {
    songs(page, "recent", &[Col::Indicator, Col::Title, Col::Artist, Col::Album, Col::Played, Col::Time], |tracks| {
        let mut played: Vec<_> = tracks.into_iter().filter(|t| t.last_played.get() > 0).collect();
        played.sort_by_key(|t| std::cmp::Reverse(t.last_played.get()));
        played
    });
}

pub fn build_most(page: &Page) {
    songs(page, "most-played", &[Col::Num, Col::Title, Col::Artist, Col::Album, Col::Plays, Col::Time], |tracks| {
        let mut played: Vec<_> = tracks.into_iter().filter(|t| t.plays.get() > 0).collect();
        played.sort_by_key(|t| std::cmp::Reverse((t.plays.get(), t.last_played.get())));
        played
    });
}

/// A song list picked from the library, kept up to date as songs play.
fn songs(page: &Page, key: &'static str, cols: &'static [Col], pick: fn(Vec<Rc<Track>>) -> Vec<Rc<Track>>) {
    page.body.append(&scan_banner());
    let content = widgets::vbox(12);
    let toolbar = widgets::hbox(12);
    toolbar.add_css_class("list-toolbar");
    let summary = widgets::label("", "dim");
    summary.add_css_class("mono");
    summary.set_hexpand(true);
    toolbar.append(&summary);
    let table =
        TrackTable::new(Options { cols, sortable: false, positions: cols.contains(&Col::Num), key: Some(key), ..Default::default() });
    let t = table.clone();
    let play = views::play_buttons(move || t.paths());
    toolbar.append(&play);
    content.append(&toolbar);
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&table.root, Some("table"));
    stack.add_named(
        &widgets::empty_state(
            "document-open-recent-symbolic",
            "Nothing played yet",
            "Songs show up here once they've been played to the end.",
            None,
        ),
        Some("empty"),
    );
    content.append(&stack);
    page.body.append(&library_stack(&content));

    let refresh = move || {
        let tracks: Vec<_> = pick(store::tracks()).into_iter().take(SONGS).collect();
        let secs: f64 = tracks.iter().map(|t| t.duration).sum();
        summary.set_text(&format!("{} · {}", fmt::count(tracks.len(), "song", "songs"), fmt::total(secs)));
        play.set_sensitive(!tracks.is_empty());
        stack.set_visible_child_name(if tracks.is_empty() { "empty" } else { "table" });
        table.set(&tracks);
    };
    refresh();
    store::subscribe(&content, move |c| {
        if c == store::Change::Library || c == store::Change::Plays {
            refresh();
        }
    });
}
