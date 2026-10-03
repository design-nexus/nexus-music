//! The song table used by Songs, albums, playlists, the queue and search:
//! sortable columns, multi-select, a right-click menu, double-click to play,
//! and drag-and-drop onto playlists in the sidebar.

use crate::library::{Kind, Track, store};
use crate::sections::{albums, playlist, podcasts};
use crate::{cmd, fmt, player, widgets, window};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    /// A speaker on the song that's playing.
    Indicator,
    /// Track number (album) or position (playlists, queue); a speaker when playing.
    Num,
    Title,
    Artist,
    Album,
    Year,
    Time,
    Plays,
}

impl Col {
    fn title(self) -> &'static str {
        match self {
            Col::Indicator | Col::Num => "",
            Col::Title => "Title",
            Col::Artist => "Artist",
            Col::Album => "Album",
            Col::Year => "Year",
            Col::Time => "Time",
            Col::Plays => "Plays",
        }
    }

    fn hidden_when_narrow(self) -> bool {
        matches!(self, Col::Year | Col::Plays)
    }
}

pub struct Row {
    pub track: Rc<Track>,
    /// Position in the list given to `set` (queue index, playlist position).
    pub index: usize,
}

pub type Extra = (&'static str, Rc<dyn Fn(Vec<usize>)>);

pub struct Options {
    pub cols: &'static [Col],
    pub sortable: bool,
    /// Numbers in `Num` are positions (1, 2, 3…) rather than track numbers.
    pub positions: bool,
    /// Instead of playing the list from the activated row.
    pub on_activate: Option<Rc<dyn Fn(usize)>>,
    /// Extra menu items acting on the selected rows' indexes.
    pub extra: Vec<Extra>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            cols: &[Col::Indicator, Col::Title, Col::Artist, Col::Album, Col::Year, Col::Time, Col::Plays],
            sortable: true,
            positions: false,
            on_activate: None,
            extra: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct TrackTable {
    pub root: gtk::Box,
    pub view: gtk::ColumnView,
    store: gio::ListStore,
    sorted: gtk::SortListModel,
    selection: gtk::MultiSelection,
}

thread_local! {
    static TABLES: RefCell<Vec<glib::WeakRef<gtk::ColumnView>>> = const { RefCell::new(Vec::new()) };
}

fn row_of(obj: &glib::Object) -> Option<std::cell::Ref<'_, Row>> {
    obj.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<Row>())
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    store::sort_key(a).cmp(&store::sort_key(b))
}

fn compare(col: Col, a: &Track, b: &Track) -> Ordering {
    let album_order = |a: &Track, b: &Track| {
        (a.disc_no.unwrap_or(1), a.track_no.unwrap_or(u32::MAX)).cmp(&(b.disc_no.unwrap_or(1), b.track_no.unwrap_or(u32::MAX)))
    };
    match col {
        Col::Title => cmp_text(&a.title, &b.title),
        Col::Artist => cmp_text(&a.artist, &b.artist)
            .then_with(|| a.year.cmp(&b.year))
            .then_with(|| cmp_text(&a.album, &b.album))
            .then_with(|| album_order(a, b)),
        Col::Album => cmp_text(&a.album, &b.album).then_with(|| album_order(a, b)),
        Col::Year => a.year.cmp(&b.year).then_with(|| cmp_text(&a.album, &b.album)).then_with(|| album_order(a, b)),
        Col::Time => a.duration.total_cmp(&b.duration),
        Col::Plays => a.plays.get().cmp(&b.plays.get()),
        Col::Indicator | Col::Num => Ordering::Equal,
    }
}

fn is_playing(t: &Track) -> bool {
    player::current().is_some_and(|c| c.path == t.path)
}

impl TrackTable {
    pub fn new(opts: Options) -> TrackTable {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let sorted = gtk::SortListModel::new(Some(store.clone()), None::<gtk::Sorter>);
        let selection = gtk::MultiSelection::new(Some(sorted.clone()));
        let view = gtk::ColumnView::new(Some(selection.clone()));
        view.add_css_class("track-table");
        view.set_reorderable(false);
        view.set_show_column_separators(false);
        let opts = Rc::new(opts);

        let table = TrackTable { root: widgets::vbox(0), view: view.clone(), store, sorted: sorted.clone(), selection };

        let mut default_sort: Option<gtk::ColumnViewColumn> = None;
        for &col in opts.cols {
            let factory = gtk::SignalListItemFactory::new();
            let t = table.clone();
            let o = opts.clone();
            factory.connect_setup(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let child: gtk::Widget = match col {
                    Col::Indicator => {
                        let i = gtk::Image::from_icon_name("audio-volume-high-symbolic");
                        i.add_css_class("accent-text");
                        i.upcast()
                    }
                    _ => {
                        let l = widgets::label("", "");
                        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                        if matches!(col, Col::Num | Col::Year | Col::Time | Col::Plays) {
                            l.add_css_class("mono");
                            l.add_css_class("cell-dim");
                            l.set_xalign(1.0);
                        }
                        if col == Col::Num {
                            l.set_xalign(1.0);
                        }
                        l.upcast()
                    }
                };
                child.set_hexpand(true);
                t.attach_row_gestures(&child, item, &o);
                item.set_child(Some(&child));
            });
            let o = opts.clone();
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
                let (Some(obj), Some(child)) = (item.item(), item.child()) else { return };
                let Some(row) = row_of(&obj) else { return };
                let t = &row.track;
                if col == Col::Indicator {
                    child.set_opacity(if is_playing(t) { 1.0 } else { 0.0 });
                    return;
                }
                let Some(l) = child.downcast_ref::<gtk::Label>() else { return };
                let text = match col {
                    Col::Num => {
                        if is_playing(t) {
                            "▶".to_string()
                        } else if o.positions {
                            (row.index + 1).to_string()
                        } else {
                            t.track_no.map(|n| n.to_string()).unwrap_or_default()
                        }
                    }
                    Col::Title => t.title.clone(),
                    Col::Artist => t.artist.clone(),
                    Col::Album => t.album.clone(),
                    Col::Year => t.year.map(|y| y.to_string()).unwrap_or_default(),
                    Col::Time if t.is_live() => "LIVE".to_string(),
                    Col::Time if t.duration <= 0.0 && t.is_remote() => String::new(),
                    Col::Time => fmt::time(t.duration),
                    Col::Plays => {
                        let p = t.plays.get();
                        if p == 0 { String::new() } else { p.to_string() }
                    }
                    Col::Indicator => String::new(),
                };
                l.set_text(&text);
                if col == Col::Num || col == Col::Title {
                    if is_playing(t) {
                        l.add_css_class("accent-text");
                    } else {
                        l.remove_css_class("accent-text");
                    }
                }
                if col == Col::Title || col == Col::Artist || col == Col::Album {
                    l.set_tooltip_text(Some(&text));
                }
            });
            let c = gtk::ColumnViewColumn::new(Some(col.title()), Some(factory));
            c.set_resizable(!matches!(col, Col::Indicator | Col::Num));
            match col {
                Col::Indicator => c.set_fixed_width(34),
                Col::Num => c.set_fixed_width(48),
                Col::Year => c.set_fixed_width(70),
                Col::Time => c.set_fixed_width(74),
                Col::Plays => c.set_fixed_width(70),
                Col::Title => {
                    c.set_expand(true);
                }
                Col::Artist | Col::Album => {
                    c.set_expand(true);
                }
            }
            if opts.sortable && !matches!(col, Col::Indicator | Col::Num) {
                c.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| match (row_of(a), row_of(b)) {
                    (Some(a), Some(b)) => compare(col, &a.track, &b.track).into(),
                    _ => gtk::Ordering::Equal,
                })));
                if col == Col::Artist {
                    default_sort = Some(c.clone());
                }
            }
            view.append_column(&c);
        }
        if opts.sortable {
            sorted.set_sorter(view.sorter().as_ref());
            if let Some(c) = default_sort {
                view.sort_by_column(Some(&c), gtk::SortType::Ascending);
            }
        }

        let t = table.clone();
        let o = opts.clone();
        view.connect_activate(move |_, pos| t.activate(pos, &o));

        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .child(&view)
            .vexpand(true)
            .build();
        table.root.add_css_class("table-card");
        table.root.set_overflow(gtk::Overflow::Hidden);
        table.root.set_vexpand(true);
        table.root.append(&scroll);

        // Re-bind on song changes so the playing marker follows.
        let weak = table.store.downgrade();
        player::subscribe(&view, move |e| {
            if e == player::Event::Track
                && let Some(s) = weak.upgrade()
            {
                let n = s.n_items();
                s.items_changed(0, n, n);
            }
        });
        TABLES.with(|t| t.borrow_mut().push(view.downgrade()));
        apply_narrow(&view, window::narrow());
        table
    }

    pub fn set(&self, tracks: &[Rc<Track>]) {
        let objs: Vec<glib::BoxedAnyObject> =
            tracks.iter().enumerate().map(|(index, t)| glib::BoxedAnyObject::new(Row { track: t.clone(), index })).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }

    pub fn len(&self) -> u32 {
        self.store.n_items()
    }

    /// Paths in the order they're shown.
    pub fn paths(&self) -> Vec<PathBuf> {
        (0..self.sorted.n_items())
            .filter_map(|i| self.sorted.item(i))
            .filter_map(|o| row_of(&o).map(|r| r.track.path.clone()))
            .collect()
    }

    pub fn focus(&self) {
        if self.sorted.n_items() > 0 {
            self.view.scroll_to(0, None, gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT, None);
        }
        self.view.grab_focus();
    }

    fn activate(&self, pos: u32, opts: &Options) {
        let Some(obj) = self.sorted.item(pos) else { return };
        let Some(index) = row_of(&obj).map(|r| r.index) else { return };
        if let Some(f) = &opts.on_activate {
            f(index);
        } else {
            player::play_tracks(self.paths(), pos as usize);
        }
    }

    /// Selected rows (in view order) — or just the clicked one if it isn't selected.
    fn targets(&self, clicked: u32) -> Vec<(u32, PathBuf, usize)> {
        let collect = |i: u32| self.sorted.item(i).and_then(|o| row_of(&o).map(|r| (i, r.track.path.clone(), r.index)));
        if !self.selection.is_selected(clicked) {
            self.selection.select_item(clicked, true);
            return collect(clicked).into_iter().collect();
        }
        let set = self.selection.selection();
        let mut out = Vec::new();
        if let Some((iter, first)) = gtk::BitsetIter::init_first(&set) {
            out.extend(collect(first));
            for i in iter {
                out.extend(collect(i));
            }
        }
        out
    }

    fn attach_row_gestures(&self, child: &gtk::Widget, item: &gtk::ListItem, opts: &Rc<Options>) {
        let click = gtk::GestureClick::new();
        click.set_button(gdk::BUTTON_SECONDARY);
        let t = self.clone();
        let o = opts.clone();
        let item_weak = item.downgrade();
        let w = child.downgrade();
        click.connect_pressed(move |_, _, x, y| {
            let (Some(item), Some(w)) = (item_weak.upgrade(), w.upgrade()) else { return };
            let pos = item.position();
            if pos == gtk::INVALID_LIST_POSITION {
                return;
            }
            t.menu(&w, x, y, pos, &o);
        });
        child.add_controller(click);

        // Drag songs onto a playlist in the sidebar.
        let drag = gtk::DragSource::new();
        drag.set_actions(gdk::DragAction::COPY);
        let t = self.clone();
        let item_weak = item.downgrade();
        drag.connect_prepare(move |_, _, _| {
            let item = item_weak.upgrade()?;
            let pos = item.position();
            if pos == gtk::INVALID_LIST_POSITION {
                return None;
            }
            let paths: Vec<String> = t.targets(pos).into_iter().map(|(_, p, _)| p.to_string_lossy().into_owned()).collect();
            Some(gdk::ContentProvider::for_value(&playlist::drag_payload(&paths).to_value()))
        });
        child.add_controller(drag);
    }

    fn menu(&self, anchor: &gtk::Widget, x: f64, y: f64, clicked: u32, opts: &Rc<Options>) {
        let targets = self.targets(clicked);
        if targets.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = targets.iter().map(|(_, p, _)| p.clone()).collect();
        let indexes: Vec<usize> = targets.iter().map(|(_, _, i)| *i).collect();
        let first = store::track_for(&paths[0]);

        let pop = gtk::Popover::new();
        pop.set_has_arrow(false);
        pop.set_parent(anchor);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.add_css_class("menu-popover");
        let pages = gtk::Stack::new();
        pages.set_vhomogeneous(false);
        pages.set_hhomogeneous(false);
        let main = widgets::vbox(1);
        let item = |label: &str, f: Box<dyn Fn()>| {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            b.add_css_class("menu-item");
            if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                l.set_xalign(0.0);
            }
            let pop = pop.clone();
            b.connect_clicked(move |_| {
                pop.popdown();
                f();
            });
            b
        };

        let (t, o, p) = (self.clone(), opts.clone(), paths.clone());
        main.append(&item(
            "Play",
            Box::new(move || {
                if p.len() == 1 {
                    t.activate(clicked, &o);
                } else {
                    player::play_tracks(p.clone(), 0);
                }
            }),
        ));
        let p = paths.clone();
        main.append(&item(
            "Play next",
            Box::new(move || {
                player::play_next(p.clone());
                window::toast(&added_text(p.len(), "to play next"));
            }),
        ));
        let p = paths.clone();
        main.append(&item(
            "Add to queue",
            Box::new(move || {
                player::enqueue(p.clone());
                window::toast(&added_text(p.len(), "to the queue"));
            }),
        ));
        let to_playlist = gtk::Button::new();
        to_playlist.add_css_class("flat");
        to_playlist.add_css_class("menu-item");
        let row = widgets::hbox(8);
        let l = widgets::label("Add to playlist", "");
        l.set_hexpand(true);
        row.append(&l);
        row.append(&gtk::Image::from_icon_name("pan-end-symbolic"));
        to_playlist.set_child(Some(&row));
        let pg = pages.clone();
        to_playlist.connect_clicked(move |_| pg.set_visible_child_name("playlists"));
        main.append(&to_playlist);
        main.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        match first.kind {
            Kind::File => {
                let key = first.album_key();
                main.append(&item("Show album", Box::new(move || albums::show(&key))));
                let dir = first.path.parent().map(|d| d.to_path_buf());
                main.append(&item(
                    "Open folder",
                    Box::new(move || {
                        if let Some(d) = &dir {
                            cmd::spawn(&["xdg-open", &d.to_string_lossy()]);
                        }
                    }),
                ));
            }
            Kind::Episode if !first.feed.is_empty() => {
                let feed = first.feed.clone();
                main.append(&item("Show podcast", Box::new(move || podcasts::show(&feed))));
            }
            Kind::Station => main.append(&item("Show radio", Box::new(|| window::navigate("radio")))),
            Kind::Episode => {}
        }
        if !opts.extra.is_empty() {
            main.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            for (label, f) in &opts.extra {
                let (f, idx) = (f.clone(), indexes.clone());
                main.append(&item(label, Box::new(move || f(idx.clone()))));
            }
        }
        pages.add_named(&main, Some("main"));

        // Second page: the playlists.
        let lists = widgets::vbox(1);
        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.add_css_class("menu-item");
        let row = widgets::hbox(8);
        row.append(&gtk::Image::from_icon_name("pan-start-symbolic"));
        row.append(&widgets::label("Add to playlist", "menu-heading-inline"));
        back.set_child(Some(&row));
        let pg = pages.clone();
        back.connect_clicked(move |_| pg.set_visible_child_name("main"));
        lists.append(&back);
        lists.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let p = paths.clone();
        lists.append(&item("New playlist…", Box::new(move || playlist::new_dialog(p.clone()))));
        let scroll_box = widgets::vbox(1);
        for pl in store::playlists() {
            let p = paths.clone();
            scroll_box.append(&item(&pl.name, Box::new(move || playlist::append(pl.id, p.clone()))));
        }
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .propagate_natural_height(true)
            .max_content_height(320)
            .child(&scroll_box)
            .build();
        lists.append(&scroll);
        pages.add_named(&lists, Some("playlists"));

        pop.set_child(Some(&pages));
        let anchor_weak = anchor.downgrade();
        pop.connect_closed(move |p| {
            let p = p.clone();
            let a = anchor_weak.clone();
            glib::idle_add_local_once(move || {
                if a.upgrade().is_some() {
                    p.unparent();
                }
            });
        });
        pop.popup();
    }
}

pub fn added_text(n: usize, where_: &str) -> String {
    format!("Added {} {where_}.", fmt::count(n, "song", "songs"))
}

fn apply_narrow(view: &gtk::ColumnView, narrow: bool) {
    let cols = view.columns();
    for i in 0..cols.n_items() {
        if let Some(c) = cols.item(i).and_downcast::<gtk::ColumnViewColumn>() {
            let title = c.title().map(|t| t.to_string()).unwrap_or_default();
            if [Col::Year, Col::Plays].iter().any(|col| col.hidden_when_narrow() && col.title() == title) {
                c.set_visible(!narrow);
            }
        }
    }
}

/// Hide the less important columns in a half-screen window.
pub fn set_narrow(narrow: bool) {
    TABLES.with(|t| {
        t.borrow_mut().retain(|w| w.upgrade().is_some());
        for w in t.borrow().iter() {
            if let Some(v) = w.upgrade() {
                apply_narrow(&v, narrow);
            }
        }
    });
}
