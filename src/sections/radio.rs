//! Radio: your favourite and own stations (grouped by genre or country), and
//! the Radio Browser directory by popularity, genre, country or name.

use crate::library::art::Cover;
use crate::online::radio::{self, Station};
use crate::online::{self, Change};
use crate::views::{self, Loader, NameList};
use crate::widgets::{self, Page};
use crate::{cmd, fmt, player, prefs, window};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::rc::Rc;

const TABS: &[(&str, &str)] = &[
    ("favourites", "Favorites"),
    ("popular", "Popular"),
    ("genres", "Genres"),
    ("countries", "Countries"),
    ("search", "Search"),
];

fn is_playing(s: &Station) -> bool {
    player::current().is_some_and(|t| t.path.as_os_str() == s.stream())
}

// ---------- A station row ----------

#[derive(Clone)]
struct StationRow {
    root: gtk::Box,
    cover: Cover,
    name: gtk::Label,
    meta: gtk::Label,
    star: gtk::Button,
    station: Rc<RefCell<Station>>,
}

impl StationRow {
    fn new() -> StationRow {
        let root = widgets::hbox(12);
        root.add_css_class("bucket-row");
        root.add_css_class("station-row");
        let cover = Cover::new(40, true);
        cover.set_placeholder("music-radio-symbolic");
        root.append(&cover.root);
        let text = widgets::vbox(1);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let name = widgets::label("", "bucket-name");
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let meta = widgets::label("", "dim");
        meta.add_css_class("bucket-meta");
        meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&name);
        text.append(&meta);
        root.append(&text);
        let star = widgets::icon_button("non-starred-symbolic", "Add to favorites");
        star.add_css_class("star-button");
        star.set_focus_on_click(false);
        root.append(&star);
        let station: Rc<RefCell<Station>> = Rc::default();
        let s = station.clone();
        star.connect_clicked(move |_| {
            let st = s.borrow().clone();
            online::set_favourite(&st, !online::is_favourite(st.stream()));
        });
        let s = station.clone();
        views::on_context(&root, move |w, x, y| menu(w, x, y, &s.borrow()));
        StationRow { root, cover, name, meta, star, station }
    }

    fn bind(&self, s: &Station) {
        *self.station.borrow_mut() = s.clone();
        self.cover.set_url(&s.favicon);
        self.name.set_text(s.name.trim());
        self.name.set_tooltip_text(Some(s.name.trim()));
        if is_playing(s) {
            self.name.add_css_class("accent-text");
        } else {
            self.name.remove_css_class("accent-text");
        }
        let meta = s.summary();
        self.meta.set_text(if meta.is_empty() { "Internet radio" } else { &meta });
        let fav = online::is_favourite(s.stream());
        self.star.set_icon_name(if fav { "starred-symbolic" } else { "non-starred-symbolic" });
        self.star.set_tooltip_text(Some(if fav { "Remove from favorites" } else { "Add to favorites" }));
        if fav {
            self.star.add_css_class("accent-text");
        } else {
            self.star.remove_css_class("accent-text");
        }
    }
}

fn menu(anchor: &gtk::Widget, x: f64, y: f64, s: &Station) {
    let paths = online::prepare_stations(std::slice::from_ref(s));
    let mut items: Vec<(String, Box<dyn Fn()>)> = Vec::new();
    let st = s.clone();
    items.push(("Play".into(), Box::new(move || online::play_station(&st))));
    let p = paths.clone();
    items.push((
        "Play next".into(),
        Box::new(move || {
            player::play_next(p.clone());
            window::toast("The station plays next.");
        }),
    ));
    let p = paths.clone();
    items.push((
        "Add to queue".into(),
        Box::new(move || {
            player::enqueue(p.clone());
            window::toast("Added the station to the queue.");
        }),
    ));
    items.push(("-".into(), Box::new(|| {})));
    let fav = online::is_favourite(s.stream());
    let st = s.clone();
    items.push((
        if fav { "Remove from favorites" } else { "Add to favorites" }.into(),
        Box::new(move || online::set_favourite(&st, !fav)),
    ));
    let url = s.stream().to_string();
    let a = anchor.clone();
    items.push((
        "Copy stream address".into(),
        Box::new(move || {
            a.clipboard().set_text(&url);
            window::toast("Copied the stream address.");
        }),
    ));
    if !s.homepage.is_empty() {
        let home = s.homepage.clone();
        items.push(("Open website".into(), Box::new(move || cmd::spawn(&["xdg-open", &home]))));
    }
    if s.custom {
        items.push(("-".into(), Box::new(|| {})));
        let st = s.clone();
        items.push(("Edit station…".into(), Box::new(move || station_dialog(Some(st.clone())))));
        let st = s.clone();
        items.push((
            "Remove station".into(),
            Box::new(move || {
                online::set_favourite(&st, false);
                window::toast(&format!("Removed {}.", st.name));
            }),
        ));
    }
    views::popup_menu(anchor, x, y, items, Some(paths));
}

// ---------- A list of stations from the directory ----------

#[derive(Clone)]
struct StationList {
    root: gtk::Box,
    store: gio::ListStore,
}

impl StationList {
    fn new() -> StationList {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let row = StationRow::new();
            item.set_child(Some(&row.root));
            unsafe { item.set_data("row", row) };
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(obj) = item.item() else { return };
            let Some(b) = obj.downcast_ref::<glib::BoxedAnyObject>() else { return };
            if let Some(row) = unsafe { item.data::<StationRow>("row") } {
                unsafe { row.as_ref() }.bind(&b.borrow::<Station>());
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
            if let Some(obj) = s.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                let st = obj.borrow::<Station>().clone();
                online::play_station(&st);
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
        let list = StationList { root, store };
        let l = list.clone();
        online::subscribe(&list.root, move |c| {
            if c == Change::Stations {
                l.rebind();
            }
        });
        let l = list.clone();
        player::subscribe(&list.root, move |e| {
            if e == player::Event::Track {
                l.rebind();
            }
        });
        list
    }

    fn set(&self, stations: Vec<Station>) {
        let objs: Vec<glib::BoxedAnyObject> = stations.into_iter().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &objs);
    }

    fn rebind(&self) {
        let n = self.store.n_items();
        self.store.items_changed(0, n, n);
    }
}

type Work = std::sync::Arc<dyn Fn() -> anyhow::Result<Vec<Station>> + Send + Sync>;

/// Load stations off the main thread into a list.
fn load_into(loader: &Loader, list: &StationList, work: impl Fn() -> anyhow::Result<Vec<Station>> + Send + Sync + 'static) {
    load_work(loader, list, std::sync::Arc::new(work));
}

fn load_work(loader: &Loader, list: &StationList, work: Work) {
    loader.loading();
    let (l, li) = (loader.clone(), list.clone());
    let w = work.clone();
    cmd::background(
        move || w(),
        move |res| match res {
            Ok(stations) if stations.is_empty() => {
                l.show_empty(&widgets::empty_state("music-radio-symbolic", "No stations", "Nothing was found here.", None))
            }
            Ok(stations) => {
                li.set(stations);
                l.show();
            }
            Err(e) => {
                let (l2, li2) = (l.clone(), li.clone());
                l.failed(&e, move || load_work(&l2, &li2, work.clone()));
            }
        },
    );
}

// ---------- Favourites ----------

fn favourites_view() -> gtk::Widget {
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    let groups = widgets::vbox(18);
    groups.add_css_class("station-groups");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&groups)
        .vexpand(true)
        .build();
    stack.add_named(&scroll, Some("list"));
    let empty = widgets::empty_state(
        "starred-symbolic",
        "No favorite stations yet",
        "Star a station to keep it here, or add one of your own with <b>Add station</b>.",
        Some(("Browse popular stations", Box::new(|| set_tab("popular")))),
    );
    stack.add_named(&empty, Some("empty"));

    let refresh = {
        let stack = stack.clone();
        move || {
            while let Some(c) = groups.first_child() {
                groups.remove(&c);
            }
            let stations = online::favourites();
            if stations.is_empty() {
                stack.set_visible_child_name("empty");
                return;
            }
            stack.set_visible_child_name("list");
            let by_country = prefs::get().radio_group == "country";
            let mut grouped: BTreeMap<(bool, String), Vec<Station>> = BTreeMap::new();
            for s in stations {
                let key = if by_country { s.country.trim().to_string() } else { s.genre() };
                // Stations with no genre or country go last.
                grouped.entry((key.is_empty(), key.to_lowercase())).or_default().push(s);
            }
            for ((other, _), list) in grouped {
                let g = widgets::vbox(6);
                let first = &list[0];
                let title = if other {
                    "Other".to_string()
                } else if by_country {
                    first.country.trim().to_string()
                } else {
                    first.genre()
                };
                g.append(&widgets::label(&format!("{} · {}", title.to_uppercase(), list.len()), "group-title"));
                let lb = gtk::ListBox::new();
                lb.add_css_class("table-card");
                lb.set_selection_mode(gtk::SelectionMode::None);
                let rows: Rc<Vec<Station>> = Rc::new(list.clone());
                for s in rows.iter() {
                    let row = StationRow::new();
                    row.bind(s);
                    lb.append(&row.root);
                }
                let r = rows.clone();
                lb.connect_row_activated(move |_, row| {
                    if let Some(s) = r.get(row.index() as usize) {
                        online::play_station(s);
                    }
                });
                g.append(&lb);
                groups.append(&g);
            }
        }
    };
    refresh();
    let r = refresh.clone();
    online::subscribe(&stack, move |c| {
        if c == Change::Stations {
            r();
        }
    });
    let r = refresh.clone();
    player::subscribe(&stack, move |e| {
        if e == player::Event::Track {
            r();
        }
    });
    FAVOURITES_REFRESH.with(|f| *f.borrow_mut() = Some(Rc::new(refresh)));
    stack.upcast()
}

// ---------- Genres and countries ----------

type Names = Box<dyn Fn() -> anyhow::Result<Vec<views::Name>> + Send + Sync>;
type ByName = fn(&str) -> anyhow::Result<Vec<Station>>;

/// A list of names (genres, countries) that opens into their stations.
fn browse_view(icon: &'static str, back: &'static str, names: Names, stations: ByName) -> (gtk::Widget, Rc<dyn Fn()>) {
    let inner = gtk::Stack::new();
    inner.set_transition_type(gtk::StackTransitionType::Crossfade);
    inner.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });
    let list = StationList::new();
    let detail_loader = Loader::new(&list.root);
    let detail = widgets::vbox(10);
    let title = widgets::label("", "detail-title");
    title.set_xalign(0.0);
    let i = inner.clone();
    detail.append(&views::back_button(back, move || i.set_visible_child_name("names")));
    detail.append(&title);
    detail.append(&detail_loader.root);
    let (dl, li, t, i) = (detail_loader.clone(), list.clone(), title.clone(), inner.clone());
    let name_list = NameList::new(icon, move |id, name| {
        t.set_text(&name);
        i.set_visible_child_name("detail");
        li.set(Vec::new());
        load_into(&dl, &li, move || stations(&id));
    });
    let names_loader = Loader::new(&name_list.root);
    inner.add_named(&names_loader.root, Some("names"));
    inner.add_named(&detail, Some("detail"));
    let names = std::sync::Arc::new(names);
    let load: Rc<dyn Fn()> = Rc::new({
        let (nl, list) = (names_loader.clone(), name_list.clone());
        move || load_names(&nl, &list, names.clone())
    });
    (inner.upcast(), load)
}

fn load_names(loader: &Loader, list: &NameList, names: std::sync::Arc<Names>) {
    loader.loading();
    let (l, li) = (loader.clone(), list.clone());
    let n = names.clone();
    cmd::background(
        move || n(),
        move |res| match res {
            Ok(v) => {
                li.set(v);
                l.show();
            }
            Err(e) => {
                let (l2, li2) = (l.clone(), li.clone());
                l.failed(&e, move || load_names(&l2, &li2, names.clone()));
            }
        },
    );
}

fn genre_names() -> anyhow::Result<Vec<views::Name>> {
    Ok(radio::tags()?
        .into_iter()
        .map(|t| (t.name.clone(), radio::title_case(&t.name), fmt::count(t.stationcount, "station", "stations")))
        .collect())
}

/// Countries, with yours first.
fn country_names(mine: String) -> anyhow::Result<Vec<views::Name>> {
    let mut list = radio::countries()?;
    if let Some(i) = list.iter().position(|c| c.code.eq_ignore_ascii_case(&mine)) {
        let c = list.remove(i);
        list.insert(0, c);
    }
    Ok(list.into_iter().map(|c| (c.code, c.name, fmt::count(c.stationcount, "station", "stations"))).collect())
}

pub fn country() -> String {
    let c = prefs::get().radio_country;
    if c.is_empty() { radio::locale_country() } else { c }
}

// ---------- Search ----------

fn search_view() -> (gtk::Widget, gtk::SearchEntry) {
    let b = widgets::vbox(10);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Search stations by name"));
    entry.add_css_class("settings-search");
    entry.add_css_class("page-search");
    b.append(&entry);
    let list = StationList::new();
    let loader = Loader::new(&list.root);
    loader.show_empty(&widgets::empty_state("system-search-symbolic", "Find a station", "Type part of its name.", None));
    b.append(&loader.root);
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    entry.connect_search_changed(move |e| {
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let q = e.text().trim().to_string();
        if q.chars().count() < 2 {
            return;
        }
        let (l, li, p) = (loader.clone(), list.clone(), pending.clone());
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(350), move || {
            p.borrow_mut().take();
            load_into(&l, &li, move || radio::search(&q));
        });
        *pending.borrow_mut() = Some(id);
    });
    (b.upcast(), entry)
}

// ---------- Add or edit a station ----------

pub fn station_dialog(existing: Option<Station>) {
    let editing = existing.is_some();
    let s = existing.clone().unwrap_or_default();
    let (dialog, card) = widgets::dialog(if editing { "Edit station" } else { "Add station" }, 460);
    let d = widgets::label(
        "A stream address, or a link to a <tt>.pls</tt> or <tt>.m3u</tt> playlist for one. It joins your favorites.",
        "dim",
    );
    d.set_use_markup(true);
    d.set_wrap(true);
    card.append(&d);
    let field = |label: &str, value: &str, placeholder: &str| {
        let b = widgets::vbox(4);
        b.append(&widgets::label(label, "field-label"));
        let e = gtk::Entry::new();
        e.set_text(value);
        e.set_placeholder_text(Some(placeholder));
        b.append(&e);
        card.append(&b);
        e
    };
    let name = field("Name", &s.name, "My station");
    let url = field("Stream address", if editing { s.stream() } else { "" }, "https://…");
    let tags = field("Genre", &s.tags, "jazz, blues");
    let country = field("Country", &s.country, "");
    let home = field("Website", &s.homepage, "Optional");
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    buttons.set_margin_top(6);
    let cancel = gtk::Button::with_label("Not now");
    let ok = gtk::Button::with_label(if editing { "Save" } else { "Add station" });
    ok.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&ok);
    card.append(&buttons);
    let dl = dialog.clone();
    cancel.connect_clicked(move |_| dl.close());
    let dl = dialog.clone();
    ok.connect_clicked(move |b| {
        let n = name.text().trim().to_string();
        let u = url.text().trim().to_string();
        name.remove_css_class("error");
        url.remove_css_class("error");
        if n.is_empty() {
            name.add_css_class("error");
            return;
        }
        if !(u.starts_with("http://") || u.starts_with("https://")) {
            url.add_css_class("error");
            return;
        }
        let station = Station {
            name: n,
            url: u,
            tags: tags.text().trim().to_string(),
            country: country.text().trim().to_string(),
            homepage: home.text().trim().to_string(),
            ..existing.clone().unwrap_or_default()
        };
        b.set_sensitive(false);
        let (dl, b) = (dl.clone(), b.clone());
        let old = existing.as_ref().map(|s| s.stream().to_string());
        online::save_custom(station, old, move || dl.close());
        // A failure leaves the dialog open to fix the address.
        glib::timeout_add_local_once(std::time::Duration::from_secs(12), move || b.set_sensitive(true));
    });
    dialog.present();
}

// ---------- The page ----------

thread_local! {
    static TABS_STACK: RefCell<Option<(gtk::Stack, gtk::Box)>> = const { RefCell::new(None) };
    static FAVOURITES_REFRESH: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

fn set_tab(id: &str) {
    TABS_STACK.with(|t| {
        if let Some((_, seg)) = t.borrow().as_ref() {
            let mut c = seg.first_child();
            let mut i = 0;
            while let Some(w) = c {
                if let Some(b) = w.downcast_ref::<gtk::ToggleButton>()
                    && TABS.get(i).is_some_and(|(t, _)| *t == id)
                {
                    b.set_active(true);
                }
                c = w.next_sibling();
                i += 1;
            }
        }
    });
}

pub fn build(page: &Page) {
    let p = prefs::get();
    let tab = if TABS.iter().any(|(t, _)| *t == p.radio_tab) { p.radio_tab.clone() } else { "favourites".into() };

    let toolbar = widgets::hbox(12);
    toolbar.add_css_class("list-toolbar");
    let tabs = gtk::Stack::new();
    tabs.set_vexpand(true);
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_transition_duration(if p.reduce_motion { 0 } else { 140 });

    tabs.add_named(&favourites_view(), Some("favourites"));
    let popular = StationList::new();
    let popular_loader = Loader::new(&popular.root);
    tabs.add_named(&popular_loader.root, Some("popular"));
    let (genres, load_genres) = browse_view("music-genre-symbolic", "Genres", Box::new(genre_names), radio::by_tag);
    tabs.add_named(&genres, Some("genres"));
    let mine = country();
    let (countries, load_countries) =
        browse_view("mark-location-symbolic", "Countries", Box::new(move || country_names(mine.clone())), radio::by_country);
    tabs.add_named(&countries, Some("countries"));
    let (search, entry) = search_view();
    tabs.add_named(&search, Some("search"));

    let group = widgets::segmented(&widgets::opts(&[("genre", "By genre"), ("country", "By country")]), &p.radio_group, |v| {
        prefs::update(|p| p.radio_group = v);
        if let Some(f) = FAVOURITES_REFRESH.with(|f| f.borrow().clone()) {
            f();
        }
    });

    let loaded: Rc<RefCell<HashSet<String>>> = Rc::default();
    let show = {
        let (tabs, group, loaded, entry) = (tabs.clone(), group.clone(), loaded.clone(), entry.clone());
        let (popular, popular_loader) = (popular.clone(), popular_loader.clone());
        move |id: &str| {
            tabs.set_visible_child_name(id);
            group.set_visible(id == "favourites");
            if id == "search" {
                entry.grab_focus();
            }
            if !loaded.borrow_mut().insert(id.to_string()) {
                return;
            }
            match id {
                "popular" => load_into(&popular_loader, &popular, radio::popular),
                "genres" => load_genres(),
                "countries" => load_countries(),
                _ => {}
            }
        }
    };
    let s = show.clone();
    let opts: Vec<(String, String)> = TABS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let seg = widgets::segmented(&opts, &tab, move |id| {
        s(&id);
        prefs::update(|p| p.radio_tab = id);
    });
    toolbar.append(&seg);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    toolbar.append(&spacer);
    toolbar.append(&group);
    let add = widgets::labeled_button("list-add-symbolic", "Add station");
    add.connect_clicked(|_| station_dialog(None));
    toolbar.append(&add);

    page.body.append(&toolbar);
    page.body.append(&tabs);
    TABS_STACK.with(|t| *t.borrow_mut() = Some((tabs.clone(), seg.clone())));
    show(&tab);
}
