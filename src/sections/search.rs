//! Search results: artists, albums and songs matching every word typed in the
//! sidebar's search field.

use crate::fmt;
use crate::library::art::Cover;
use crate::library::store;
use crate::tracklist::{Options, TrackTable};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Ui {
    artists_group: gtk::Box,
    artists: gtk::FlowBox,
    albums_group: gtk::Box,
    albums: gtk::FlowBox,
    songs_title: gtk::Label,
    table: TrackTable,
    stack: gtk::Stack,
    nothing: gtk::Label,
    query: String,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
}

const MAX_ARTISTS: usize = 12;
const MAX_ALBUMS: usize = 10;

fn flow() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_row_spacing(8);
    f.set_column_spacing(8);
    f.set_homogeneous(false);
    f.set_max_children_per_line(12);
    f
}

pub fn build(page: &Page) {
    let content = widgets::vbox(0);
    content.set_vexpand(true);

    let artists_group = widgets::vbox(8);
    artists_group.append(&widgets::label("ARTISTS", "group-title"));
    let artists = flow();
    artists_group.append(&artists);
    content.append(&artists_group);

    let albums_group = widgets::vbox(8);
    albums_group.append(&widgets::label("ALBUMS", "group-title"));
    let albums = flow();
    albums_group.append(&albums);
    content.append(&albums_group);

    let songs_title = widgets::label("SONGS", "group-title");
    content.append(&songs_title);
    let table = TrackTable::new(Options::default());
    table.root.set_size_request(-1, 240);
    content.append(&table.root);

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&content, Some("results"));
    let nothing = widgets::label("", "dim");
    nothing.set_halign(gtk::Align::Center);
    nothing.set_valign(gtk::Align::Center);
    stack.add_named(&nothing, Some("nothing"));
    page.body.append(&stack);

    let ui = Rc::new(RefCell::new(Ui {
        artists_group,
        artists,
        albums_group,
        albums,
        songs_title,
        table,
        stack: stack.clone(),
        nothing,
        query: String::new(),
    }));
    UI.with(|u| *u.borrow_mut() = Some(ui));
    store::subscribe(&stack, |c| {
        if c == store::Change::Library {
            let q = UI.with(|u| u.borrow().as_ref().map(|u| u.borrow().query.clone())).unwrap_or_default();
            if !q.is_empty() {
                set_query(&q);
            }
        }
    });
}

fn clear(f: &gtk::FlowBox) {
    while let Some(c) = f.first_child() {
        f.remove(&c);
    }
}

pub fn set_query(q: &str) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else { return };
    let mut u = ui.borrow_mut();
    u.query = q.to_string();
    let terms: Vec<String> = q.to_lowercase().split_whitespace().map(str::to_string).collect();
    let hit = |hay: &str| terms.iter().all(|t| hay.contains(t.as_str()));

    let songs: Vec<_> = store::tracks().into_iter().filter(|t| hit(&t.haystack())).collect();
    let albums: Vec<_> = store::albums()
        .into_iter()
        .filter(|a| hit(&format!("{} {}", a.title, a.artist).to_lowercase()))
        .take(MAX_ALBUMS)
        .collect();
    let artists: Vec<_> = store::artists().into_iter().filter(|b| hit(&b.name.to_lowercase())).take(MAX_ARTISTS).collect();

    clear(&u.artists);
    for a in &artists {
        let b = gtk::Button::with_label(&a.name);
        b.add_css_class("chip");
        let name = a.name.clone();
        b.connect_clicked(move |_| super::artists::show(&name));
        u.artists.append(&b);
    }
    u.artists_group.set_visible(!artists.is_empty());

    clear(&u.albums);
    for a in &albums {
        let card = gtk::Button::new();
        card.add_css_class("flat");
        card.add_css_class("album-chip");
        let row = widgets::hbox(10);
        let cover = Cover::new(44, true);
        cover.set_key(&a.art);
        row.append(&cover.root);
        let text = widgets::vbox(1);
        text.set_valign(gtk::Align::Center);
        let t = widgets::label(&a.title, "album-title");
        t.set_ellipsize(gtk::pango::EllipsizeMode::End);
        t.set_max_width_chars(24);
        let ar = widgets::label(&a.artist, "album-artist");
        ar.set_ellipsize(gtk::pango::EllipsizeMode::End);
        ar.set_max_width_chars(24);
        text.append(&t);
        text.append(&ar);
        row.append(&text);
        card.set_child(Some(&row));
        let key = a.key.clone();
        card.connect_clicked(move |_| super::albums::show(&key));
        u.albums.append(&card);
    }
    u.albums_group.set_visible(!albums.is_empty());

    u.songs_title.set_text(&format!("SONGS · {}", fmt::thousands(songs.len())));
    u.songs_title.set_visible(!songs.is_empty());
    u.table.root.set_visible(!songs.is_empty());
    u.table.set(&songs);

    let any = !songs.is_empty() || !albums.is_empty() || !artists.is_empty();
    u.nothing.set_text(&format!("Nothing in your library matches “{q}”."));
    u.stack.set_visible_child_name(if any { "results" } else { "nothing" });
}

/// Enter in the search field: move into the songs.
pub fn focus_results() {
    if let Some(ui) = UI.with(|u| u.borrow().clone()) {
        let table = ui.borrow().table.clone();
        table.focus();
    }
}
