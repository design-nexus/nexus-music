//! Shared library views: the album grid, artist/genre lists and the header
//! of a detail view.

use crate::library::art::Cover;
use crate::library::store::{Album, Bucket};
use crate::{fmt, player, widgets};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;
use std::rc::Rc;

fn boxed<T: 'static>(obj: &glib::Object) -> Option<std::cell::Ref<'_, T>> {
    obj.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<T>())
}

/// A virtualized grid of album cards.
#[derive(Clone)]
pub struct AlbumGrid {
    pub root: gtk::ScrolledWindow,
    store: gio::ListStore,
}

impl AlbumGrid {
    pub fn new(on_open: impl Fn(Rc<Album>) + 'static) -> AlbumGrid {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let card = widgets::vbox(6);
            card.add_css_class("album-card");
            let cover = Cover::new(168, true);
            card.append(&cover.root);
            let title = widgets::label("", "album-title");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.set_max_width_chars(18);
            let artist = widgets::label("", "album-artist");
            artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
            artist.set_max_width_chars(18);
            card.append(&title);
            card.append(&artist);
            item.set_child(Some(&card));
            // Keep the parts with the item so bind can reach them.
            unsafe { item.set_data("parts", (cover, title, artist)) };
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(obj) = item.item() else { return };
            let Some(album) = boxed::<Rc<Album>>(&obj) else { return };
            if let Some(parts) = unsafe { item.data::<(Cover, gtk::Label, gtk::Label)>("parts") } {
                let (cover, title, artist) = unsafe { parts.as_ref() };
                cover.set_key(&album.art);
                title.set_text(&album.title);
                title.set_tooltip_text(Some(&album.title));
                artist.set_text(&match album.year {
                    Some(y) => format!("{} · {y}", album.artist),
                    None => album.artist.clone(),
                });
            }
        });
        let model = gtk::NoSelection::new(Some(store.clone()));
        let grid = gtk::GridView::new(Some(model), Some(factory));
        grid.add_css_class("album-grid");
        grid.set_min_columns(2);
        grid.set_max_columns(10);
        grid.set_single_click_activate(true);
        let s = store.clone();
        grid.connect_activate(move |_, pos| {
            if let Some(obj) = s.item(pos)
                && let Some(a) = boxed::<Rc<Album>>(&obj)
            {
                let a = a.clone();
                on_open(a);
            }
        });
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&grid)
            .vexpand(true)
            .build();
        AlbumGrid { root, store }
    }

    pub fn set(&self, albums: &[Rc<Album>]) {
        let objs: Vec<glib::BoxedAnyObject> = albums.iter().map(|a| glib::BoxedAnyObject::new(a.clone())).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }
}

/// A list of artists or genres: cover, name, counts.
#[derive(Clone)]
pub struct BucketList {
    pub root: gtk::Box,
    store: gio::ListStore,
}

impl BucketList {
    pub fn new(on_open: impl Fn(String) + 'static) -> BucketList {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let row = widgets::hbox(12);
            row.add_css_class("bucket-row");
            let cover = Cover::new(40, true);
            row.append(&cover.root);
            let text = widgets::vbox(1);
            text.set_valign(gtk::Align::Center);
            text.set_hexpand(true);
            let name = widgets::label("", "bucket-name");
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let meta = widgets::label("", "dim");
            meta.add_css_class("bucket-meta");
            text.append(&name);
            text.append(&meta);
            row.append(&text);
            item.set_child(Some(&row));
            unsafe { item.set_data("parts", (cover, name, meta)) };
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(obj) = item.item() else { return };
            let Some(b) = boxed::<Rc<Bucket>>(&obj) else { return };
            if let Some(parts) = unsafe { item.data::<(Cover, gtk::Label, gtk::Label)>("parts") } {
                let (cover, name, meta) = unsafe { parts.as_ref() };
                cover.set_key(&b.art);
                name.set_text(&b.name);
                meta.set_text(&format!(
                    "{} · {}",
                    fmt::count(b.albums.len(), "album", "albums"),
                    fmt::count(b.tracks, "song", "songs")
                ));
            }
        });
        let model = gtk::SingleSelection::new(Some(store.clone()));
        model.set_autoselect(false);
        model.set_can_unselect(true);
        let list = gtk::ListView::new(Some(model), Some(factory));
        list.add_css_class("bucket-list");
        list.set_single_click_activate(true);
        let s = store.clone();
        list.connect_activate(move |_, pos| {
            if let Some(obj) = s.item(pos)
                && let Some(b) = boxed::<Rc<Bucket>>(&obj)
            {
                let name = b.name.clone();
                on_open(name);
            }
        });
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&list)
            .vexpand(true)
            .build();
        let root = widgets::vbox(0);
        root.add_css_class("table-card");
        root.set_overflow(gtk::Overflow::Hidden);
        root.set_vexpand(true);
        root.append(&scroll);
        BucketList { root, store }
    }

    pub fn set(&self, buckets: Vec<Bucket>) {
        let objs: Vec<glib::BoxedAnyObject> = buckets.into_iter().map(|b| glib::BoxedAnyObject::new(Rc::new(b))).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }
}

/// A back button for detail views.
pub fn back_button(label: &str, on_back: impl Fn() + 'static) -> gtk::Button {
    let b = widgets::labeled_button("go-previous-symbolic", label);
    b.add_css_class("flat");
    b.add_css_class("back-button");
    b.set_halign(gtk::Align::Start);
    b.connect_clicked(move |_| on_back());
    b
}

/// Play · Shuffle · Add to queue, for a list of songs.
pub fn play_buttons(paths: impl Fn() -> Vec<PathBuf> + 'static) -> gtk::Box {
    let row = widgets::hbox(8);
    let paths = Rc::new(paths);
    let play = widgets::labeled_button("media-playback-start-symbolic", "Play");
    play.add_css_class("suggested-action");
    let p = paths.clone();
    play.connect_clicked(move |_| {
        player::set_shuffle(false);
        player::play_tracks(p(), 0);
    });
    let shuffle = widgets::labeled_button("media-playlist-shuffle-symbolic", "Shuffle");
    let p = paths.clone();
    shuffle.connect_clicked(move |_| player::shuffle_tracks(p()));
    let queue = widgets::labeled_button("list-add-symbolic", "Add to queue");
    let p = paths.clone();
    queue.connect_clicked(move |_| {
        let paths = p();
        let n = paths.len();
        player::enqueue(paths);
        crate::window::toast(&crate::tracklist::added_text(n, "to the queue"));
    });
    row.append(&play);
    row.append(&shuffle);
    row.append(&queue);
    row
}

/// The header of an album, artist, folder or playlist view.
/// Returns the header and its title label (to update in place).
pub fn detail_header(
    cover: Option<&Cover>,
    kicker: &str,
    title: &str,
    subtitle: &gtk::Widget,
    meta: &str,
    actions: &gtk::Box,
) -> (gtk::Box, gtk::Label) {
    let header = widgets::hbox(22);
    header.add_css_class("detail-header");
    if let Some(c) = cover {
        header.append(&c.root);
    }
    let text = widgets::vbox(4);
    text.set_valign(gtk::Align::End);
    text.set_hexpand(true);
    if !kicker.is_empty() {
        text.append(&widgets::label(&kicker.to_uppercase(), "group-title"));
    }
    let t = widgets::label(title, "detail-title");
    t.set_wrap(true);
    t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    t.set_lines(2);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&t);
    text.append(subtitle);
    if !meta.is_empty() {
        let m = widgets::label(meta, "dim");
        m.add_css_class("mono");
        m.add_css_class("detail-meta");
        text.append(&m);
    }
    actions.set_margin_top(10);
    text.append(actions);
    header.append(&text);
    (header, t)
}

/// A clickable artist name.
pub fn artist_link(name: &str) -> gtk::Widget {
    let b = gtk::Button::with_label(name);
    b.add_css_class("flat");
    b.add_css_class("link-button");
    b.set_halign(gtk::Align::Start);
    let n = name.to_string();
    b.connect_clicked(move |_| crate::sections::artists::show(&n));
    b.upcast()
}
