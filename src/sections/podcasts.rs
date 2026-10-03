//! Podcasts: your subscriptions, Apple's charts and categories, search, and a
//! show's episodes to stream, download or queue.

use crate::library::Track;
use crate::library::art::Cover;
use crate::online::podcast::{self, Feed, Show};
use crate::online::{self, Change};
use crate::views::{self, CoverGrid, CoverItem, Loader, NameList};
use crate::widgets::{self, Page};
use crate::{cmd, fmt, player, prefs, window};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Episodes shown at first, and how many more each "Show more" adds.
const PAGE: usize = 100;

const TABS: &[(&str, &str)] =
    &[("subscribed", "Subscribed"), ("top", "Top shows"), ("categories", "Categories"), ("search", "Search")];

struct Ui {
    outer: gtk::Stack,
    detail: gtk::Box,
    seg: gtk::Box,
    /// The feed the detail view shows.
    open: String,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn country() -> String {
    super::radio::country()
}

// ---------- Show grids ----------

/// A grid of shows from the directory; opening one shows its episodes.
fn show_grid() -> (CoverGrid, Rc<RefCell<Vec<Show>>>) {
    let shows: Rc<RefCell<Vec<Show>>> = Rc::default();
    let s = shows.clone();
    let grid = CoverGrid::new(move |id| {
        let show = id.parse::<usize>().ok().and_then(|i| s.borrow().get(i).cloned());
        if let Some(show) = show {
            open_show(show);
        }
    });
    (grid, shows)
}

fn set_shows(grid: &CoverGrid, store: &Rc<RefCell<Vec<Show>>>, shows: Vec<Show>) {
    grid.set(
        shows
            .iter()
            .enumerate()
            .map(|(i, s)| CoverItem {
                id: i.to_string(),
                title: s.title.clone(),
                subtitle: s.author.clone(),
                art_url: s.art_url.clone(),
                badge: String::new(),
            })
            .collect(),
    );
    *store.borrow_mut() = shows;
}

fn load_shows(
    loader: &Loader,
    grid: &CoverGrid,
    store: &Rc<RefCell<Vec<Show>>>,
    work: std::sync::Arc<dyn Fn() -> anyhow::Result<Vec<Show>> + Send + Sync>,
) {
    loader.loading();
    let (l, g, s) = (loader.clone(), grid.clone(), store.clone());
    let w = work.clone();
    cmd::background(
        move || w(),
        move |res| match res {
            Ok(shows) if shows.is_empty() => {
                l.show_empty(&widgets::empty_state("music-podcast-symbolic", "No shows", "Nothing was found.", None))
            }
            Ok(shows) => {
                set_shows(&g, &s, shows);
                l.show();
            }
            Err(e) => {
                let (l2, g2, s2) = (l.clone(), g.clone(), s.clone());
                l.failed(&e, move || load_shows(&l2, &g2, &s2, work.clone()));
            }
        },
    );
}

fn subscribed_view() -> gtk::Widget {
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    let grid = CoverGrid::new(|feed| open_feed(&feed, None));
    stack.add_named(&grid.root, Some("grid"));
    let empty = widgets::empty_state(
        "music-podcast-symbolic",
        "No podcasts yet",
        "Subscribe to a show to keep it here and see its new episodes.",
        Some(("Browse top shows", Box::new(|| set_tab("top")))),
    );
    stack.add_named(&empty, Some("empty"));
    let refresh = {
        let stack = stack.clone();
        move || {
            let list = online::podcasts();
            stack.set_visible_child_name(if list.is_empty() { "empty" } else { "grid" });
            grid.set(
                list.iter()
                    .map(|p| {
                        let n = online::new_count(p);
                        CoverItem {
                            id: p.feed.clone(),
                            title: p.title.clone(),
                            subtitle: if online::refreshing(&p.feed) { "Checking…".into() } else { p.author.clone() },
                            art_url: p.art_url.clone(),
                            badge: if n > 0 { format!("{n} new") } else { String::new() },
                        }
                    })
                    .collect(),
            );
        }
    };
    refresh();
    online::subscribe(&stack, move |c| {
        if matches!(c, Change::Podcasts | Change::Episodes) {
            refresh();
        }
    });
    stack.upcast()
}

fn categories_view() -> gtk::Widget {
    let inner = gtk::Stack::new();
    inner.set_transition_type(gtk::StackTransitionType::Crossfade);
    inner.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });
    let (grid, shows) = show_grid();
    let loader = Loader::new(&grid.root);
    let detail = widgets::vbox(10);
    let title = widgets::label("", "detail-title");
    title.set_xalign(0.0);
    let i = inner.clone();
    detail.append(&views::back_button("Categories", move || i.set_visible_child_name("names")));
    detail.append(&title);
    detail.append(&loader.root);
    let (t, i) = (title.clone(), inner.clone());
    let names = NameList::new("music-podcast-symbolic", move |id, name| {
        t.set_text(&name);
        i.set_visible_child_name("detail");
        let genre = id.parse::<u32>().ok();
        let cc = country();
        load_shows(&loader, &grid, &shows, std::sync::Arc::new(move || podcast::top(&cc, genre)));
    });
    names.set(podcast::CATEGORIES.iter().map(|(id, name)| (id.to_string(), name.to_string(), String::new())).collect());
    inner.add_named(&names.root, Some("names"));
    inner.add_named(&detail, Some("detail"));
    inner.upcast()
}

fn search_view() -> (gtk::Widget, gtk::SearchEntry) {
    let b = widgets::vbox(10);
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Search podcasts"));
    entry.add_css_class("settings-search");
    entry.add_css_class("page-search");
    b.append(&entry);
    let (grid, shows) = show_grid();
    let loader = Loader::new(&grid.root);
    loader.show_empty(&widgets::empty_state("system-search-symbolic", "Find a podcast", "Search by its name or its host.", None));
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
        let (l, g, s, p) = (loader.clone(), grid.clone(), shows.clone(), pending.clone());
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
            p.borrow_mut().take();
            let cc = country();
            load_shows(&l, &g, &s, std::sync::Arc::new(move || podcast::search(&cc, &q)));
        });
        *pending.borrow_mut() = Some(id);
    });
    (b.upcast(), entry)
}

// ---------- Episodes ----------

fn episode_meta(t: &Track) -> String {
    let mut m = Vec::new();
    let published = online::info(&t.path).published;
    if published > 0 {
        m.push(fmt::date(published));
    }
    let pr = online::progress(&t.path);
    if pr.played {
        m.push("Played".into());
    } else if pr.position > 0.0 && t.duration > 0.0 {
        m.push(format!("{} left", fmt::total((t.duration - pr.position).max(60.0))));
    } else if t.duration > 0.0 {
        m.push(fmt::total(t.duration));
    }
    match online::download_state(&t.path) {
        Some((done, total)) if total > 0 => m.push(format!("Downloading {}%", done * 100 / total)),
        Some(_) => m.push("Downloading…".into()),
        None if t.download.is_some() => m.push("Downloaded".into()),
        None => {}
    }
    m.join(" · ")
}

#[derive(Clone)]
struct EpisodeRow {
    root: gtk::Box,
    title: gtk::Label,
    meta: gtk::Label,
    bar: gtk::ProgressBar,
    download: gtk::Button,
    track: Rc<RefCell<Option<Rc<Track>>>>,
}

impl EpisodeRow {
    fn new() -> EpisodeRow {
        let root = widgets::hbox(12);
        root.add_css_class("bucket-row");
        root.add_css_class("episode-row");
        let play = widgets::icon_button("media-playback-start-symbolic", "Play");
        play.set_focus_on_click(false);
        root.append(&play);
        let text = widgets::vbox(3);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let title = widgets::label("", "bucket-name");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let meta = widgets::label("", "dim");
        meta.add_css_class("bucket-meta");
        meta.add_css_class("mono");
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("episode-progress");
        text.append(&title);
        text.append(&meta);
        text.append(&bar);
        root.append(&text);
        let download = widgets::icon_button("folder-download-symbolic", "Download");
        download.set_focus_on_click(false);
        root.append(&download);
        let track: Rc<RefCell<Option<Rc<Track>>>> = Rc::default();
        let t = track.clone();
        play.connect_clicked(move |_| {
            if let Some(t) = t.borrow().clone() {
                play_episode(&t);
            }
        });
        let t = track.clone();
        download.connect_clicked(move |_| {
            let Some(t) = t.borrow().clone() else { return };
            if online::download_state(&t.path).is_some() {
                return;
            }
            if t.download.is_some() {
                online::delete_download(&t.path);
            } else {
                online::download(&t.path);
            }
        });
        let t = track.clone();
        views::on_context(&root, move |w, x, y| {
            if let Some(t) = t.borrow().clone() {
                episode_menu(w, x, y, &t);
            }
        });
        EpisodeRow { root, title, meta, bar, download, track }
    }

    fn bind(&self, t: &Rc<Track>) {
        *self.track.borrow_mut() = Some(t.clone());
        self.title.set_text(&t.title);
        let playing = player::current().is_some_and(|c| c.path == t.path);
        if playing {
            self.title.add_css_class("accent-text");
        } else {
            self.title.remove_css_class("accent-text");
        }
        self.meta.set_text(&episode_meta(t));
        let pr = online::progress(&t.path);
        if pr.played {
            self.root.add_css_class("played");
        } else {
            self.root.remove_css_class("played");
        }
        let fraction = if !pr.played && pr.position > 0.0 && t.duration > 0.0 { pr.position / t.duration } else { 0.0 };
        self.bar.set_visible(fraction > 0.0);
        self.bar.set_fraction(fraction.clamp(0.0, 1.0));
        let notes = online::info(&t.path).description;
        let notes: String = notes.chars().take(600).collect();
        self.root.set_tooltip_text((!notes.is_empty()).then_some(notes.as_str()));
        let (icon, tip) = match online::download_state(&t.path) {
            Some(_) => ("content-loading-symbolic", "Downloading…"),
            None if t.download.is_some() => ("user-trash-symbolic", "Delete the download"),
            None => ("folder-download-symbolic", "Download to play offline"),
        };
        self.download.set_icon_name(icon);
        self.download.set_tooltip_text(Some(tip));
    }
}

fn play_episode(t: &Track) {
    player::play_tracks(vec![t.path.clone()], 0);
}

fn episode_menu(anchor: &gtk::Widget, x: f64, y: f64, t: &Rc<Track>) {
    let paths = vec![t.path.clone()];
    let mut items: Vec<(String, Box<dyn Fn()>)> = Vec::new();
    let tt = t.clone();
    items.push(("Play".into(), Box::new(move || play_episode(&tt))));
    let p = paths.clone();
    items.push((
        "Play next".into(),
        Box::new(move || {
            player::play_next(p.clone());
            window::toast("The episode plays next.");
        }),
    ));
    let p = paths.clone();
    items.push((
        "Add to queue".into(),
        Box::new(move || {
            player::enqueue(p.clone());
            window::toast("Added the episode to the queue.");
        }),
    ));
    items.push(("-".into(), Box::new(|| {})));
    let played = online::progress(&t.path).played;
    let path = t.path.clone();
    items.push((
        if played { "Mark as not played" } else { "Mark as played" }.into(),
        Box::new(move || online::set_progress(&path, 0.0, !played)),
    ));
    let path = t.path.clone();
    if online::download_state(&t.path).is_none() {
        if t.download.is_some() {
            items.push(("Delete download".into(), Box::new(move || online::delete_download(&path))));
        } else {
            items.push(("Download".into(), Box::new(move || online::download(&path))));
        }
    }
    let url = t.path.to_string_lossy().into_owned();
    let a = anchor.clone();
    items.push((
        "Copy episode address".into(),
        Box::new(move || {
            a.clipboard().set_text(&url);
            window::toast("Copied the episode address.");
        }),
    ));
    views::popup_menu(anchor, x, y, items, Some(paths));
}

#[derive(Clone)]
struct EpisodeList {
    root: gtk::Box,
    store: gio::ListStore,
    all: Rc<RefCell<Vec<Rc<Track>>>>,
    shown: Rc<std::cell::Cell<usize>>,
    more: gtk::Button,
}

impl EpisodeList {
    fn new() -> EpisodeList {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let row = EpisodeRow::new();
            item.set_child(Some(&row.root));
            unsafe { item.set_data("row", row) };
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().expect("list item");
            let Some(obj) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
            if let Some(row) = unsafe { item.data::<EpisodeRow>("row") } {
                unsafe { row.as_ref() }.bind(&obj.borrow::<Rc<Track>>());
            }
        });
        let model = gtk::SingleSelection::new(Some(store.clone()));
        model.set_autoselect(false);
        model.set_can_unselect(true);
        let list = gtk::ListView::new(Some(model), Some(factory));
        list.add_css_class("bucket-list");
        let s = store.clone();
        list.connect_activate(move |_, pos| {
            if let Some(obj) = s.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                let t = obj.borrow::<Rc<Track>>().clone();
                play_episode(&t);
            }
        });
        // The detail page scrolls as a whole, so the list is paged instead.
        let card = widgets::vbox(0);
        card.add_css_class("table-card");
        card.set_overflow(gtk::Overflow::Hidden);
        list.set_vexpand(false);
        card.append(&list);
        let root = widgets::vbox(10);
        root.append(&card);
        let more = gtk::Button::with_label("");
        more.set_halign(gtk::Align::Center);
        root.append(&more);
        let l = EpisodeList { root, store, all: Rc::default(), shown: Rc::new(std::cell::Cell::new(PAGE)), more };
        let me = l.clone();
        l.more.connect_clicked(move |_| {
            me.shown.set(me.shown.get() + PAGE);
            me.render();
        });
        l
    }

    fn set(&self, tracks: Vec<Rc<Track>>) {
        *self.all.borrow_mut() = tracks;
        self.render();
    }

    fn render(&self) {
        let all = self.all.borrow();
        let n = all.len().min(self.shown.get());
        let objs: Vec<glib::BoxedAnyObject> = all[..n].iter().cloned().map(glib::BoxedAnyObject::new).collect();
        self.store.splice(0, self.store.n_items(), &objs);
        let rest = all.len() - n;
        self.more.set_visible(rest > 0);
        self.more.set_label(&format!("Show {} more", rest.min(PAGE)));
    }

    fn rebind(&self) {
        let n = self.store.n_items();
        self.store.items_changed(0, n, n);
    }
}

// ---------- A show ----------

/// What the header shows, from whichever source we have.
#[derive(Default, Clone)]
struct Header {
    title: String,
    author: String,
    art_url: String,
    description: String,
    apple_id: String,
}

fn header_for(feed: &str, show: Option<&Show>, loaded: Option<&Feed>) -> Header {
    if let Some(f) = loaded {
        return Header {
            title: f.title.clone(),
            author: f.author.clone(),
            art_url: f.art_url.clone(),
            description: f.description.clone(),
            apple_id: show.map(|s| s.apple_id.clone()).unwrap_or_default(),
        };
    }
    if let Some(p) = online::podcast(feed) {
        return Header { title: p.title, author: p.author, art_url: p.art_url, description: p.description, apple_id: p.apple_id };
    }
    show.map(|s| Header {
        title: s.title.clone(),
        author: s.author.clone(),
        art_url: s.art_url.clone(),
        description: s.summary.clone(),
        apple_id: s.apple_id.clone(),
    })
    .unwrap_or_default()
}

/// A chart entry has no feed address yet; look it up first.
fn open_show(show: Show) {
    if !show.feed.is_empty() {
        open_feed(&show.feed.clone(), Some(show));
        return;
    }
    let Some(detail) = begin_detail("") else { return };
    let spinner = gtk::Spinner::new();
    spinner.set_spinning(true);
    spinner.set_size_request(32, 32);
    spinner.set_vexpand(true);
    detail.append(&spinner);
    let id = show.apple_id.clone();
    cmd::background(
        move || podcast::lookup(&id),
        move |res| match res {
            Ok(feed) => open_feed(&feed, Some(Show { feed: feed.clone(), ..show })),
            Err(e) => {
                window::toast(&format!("Couldn't open {}: {e:#}", show.title));
                back();
            }
        },
    );
}

/// Show a podcast's page (from anywhere, e.g. Now playing).
pub fn show(feed: &str) {
    if feed.is_empty() {
        return;
    }
    window::navigate("podcasts");
    open_feed(feed, None);
}

/// Switch to the detail view, emptied. Returns its box.
fn begin_detail(feed: &str) -> Option<gtk::Box> {
    UI.with(|u| {
        let mut u = u.borrow_mut();
        let u = u.as_mut()?;
        u.open = feed.to_string();
        while let Some(c) = u.detail.first_child() {
            u.detail.remove(&c);
        }
        u.outer.set_visible_child_name("detail");
        let detail = u.detail.clone();
        detail.append(&views::back_button("Podcasts", back));
        Some(detail)
    })
}

fn back() {
    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.outer.set_visible_child_name("browse");
            u.open.clear();
        }
    });
}

fn open_feed(feed: &str, show: Option<Show>) {
    let Some(detail) = begin_detail(feed) else { return };
    let feed = feed.to_string();
    let loaded: Rc<RefCell<Option<Feed>>> = Rc::default();
    let h = header_for(&feed, show.as_ref(), None);

    let cover = Cover::new(160, false);
    cover.set_placeholder("music-podcast-symbolic");
    cover.set_url(&h.art_url);
    let author = widgets::label(&h.author, "dim");
    author.set_xalign(0.0);
    let actions = widgets::hbox(8);
    let play_latest = widgets::labeled_button("media-playback-start-symbolic", "Play latest");
    play_latest.add_css_class("suggested-action");
    let subscribe = gtk::Button::with_label("");
    let refresh = widgets::icon_button("view-refresh-symbolic", "Check for new episodes");
    actions.append(&play_latest);
    actions.append(&subscribe);
    actions.append(&refresh);
    let (header, title) = views::detail_header(Some(&cover), "Podcast", &h.title, author.upcast_ref(), "", &actions);
    detail.append(&header);
    let about = widgets::label(&h.description, "dim");
    about.add_css_class("podcast-about");
    about.set_wrap(true);
    about.set_lines(4);
    about.set_ellipsize(gtk::pango::EllipsizeMode::End);
    about.set_xalign(0.0);
    about.set_visible(!h.description.is_empty());
    detail.append(&about);
    let count = widgets::label("", "group-title");
    detail.append(&count);
    let list = EpisodeList::new();
    let loader = Loader::new(&list.root);
    loader.root.set_vexpand(false);
    detail.append(&loader.root);

    let sync = {
        let (feed, subscribe, refresh, count, list, loader, play_latest) =
            (feed.clone(), subscribe.clone(), refresh.clone(), count.clone(), list.clone(), loader.clone(), play_latest.clone());
        let loaded = loaded.clone();
        move || {
            let subscribed = online::is_subscribed(&feed);
            subscribe.set_label(if subscribed { "Unsubscribe" } else { "Subscribe" });
            subscribe.set_sensitive(subscribed || loaded.borrow().is_some());
            refresh.set_visible(subscribed);
            refresh.set_sensitive(!online::refreshing(&feed));
            let episodes = online::episodes(&feed);
            play_latest.set_sensitive(!episodes.is_empty());
            count.set_text(&fmt::count(episodes.len(), "episode", "episodes").to_uppercase());
            count.set_visible(!episodes.is_empty());
            if !episodes.is_empty() {
                list.set(episodes);
                loader.show();
            }
        }
    };
    sync();

    let ctx = Rc::new(Fetch {
        feed: feed.clone(),
        show: show.clone(),
        loaded: loaded.clone(),
        loader: loader.clone(),
        sync: Rc::new(sync.clone()),
        title: title.clone(),
        author: author.clone(),
        about: about.clone(),
        cover: cover.clone(),
    });
    let fetch = {
        let ctx = ctx.clone();
        move || fetch_feed(ctx.clone())
    };
    fetch();

    let (fd, ld, sh) = (feed.clone(), loaded.clone(), show.clone());
    subscribe.connect_clicked(move |_| {
        if online::is_subscribed(&fd) {
            online::unsubscribe(&fd);
        } else if let Some(f) = ld.borrow().as_ref() {
            online::subscribe_feed(&fd, &sh.as_ref().map(|s| s.apple_id.clone()).unwrap_or(h.apple_id.clone()), f);
        }
    });
    refresh.connect_clicked(move |_| fetch());
    let fd = feed.clone();
    play_latest.connect_clicked(move |_| {
        if let Some(t) = online::episodes(&fd).first() {
            play_episode(t);
        }
    });

    let (s, l) = (sync.clone(), list.clone());
    online::subscribe(&detail_marker(&detail), move |c| match c {
        Change::Downloads => l.rebind(),
        _ => s(),
    });
    let l = list.clone();
    player::subscribe(&list.root, move |e| {
        if e == player::Event::Track {
            l.rebind();
        }
    });
}

/// What a feed fetch updates on the detail view.
struct Fetch {
    feed: String,
    show: Option<Show>,
    loaded: Rc<RefCell<Option<Feed>>>,
    loader: Loader,
    sync: Rc<dyn Fn()>,
    title: gtk::Label,
    author: gtk::Label,
    about: gtk::Label,
    cover: Cover,
}

fn fetch_feed(ctx: Rc<Fetch>) {
    if online::episodes(&ctx.feed).is_empty() {
        ctx.loader.loading();
    }
    let feed = ctx.feed.clone();
    online::load_feed(&feed, move |res| {
        // The view may have moved on to another show.
        if UI.with(|u| u.borrow().as_ref().map(|u| u.open.clone())) != Some(ctx.feed.clone()) {
            return;
        }
        match res {
            Ok(f) => {
                let h = header_for(&ctx.feed, ctx.show.as_ref(), Some(&f));
                ctx.title.set_text(&h.title);
                ctx.author.set_text(&h.author);
                ctx.about.set_text(&h.description);
                ctx.about.set_visible(!h.description.is_empty());
                ctx.cover.set_url(&h.art_url);
                let empty = f.episodes.is_empty();
                *ctx.loaded.borrow_mut() = Some(f);
                (ctx.sync)();
                if empty {
                    ctx.loader.show_empty(&widgets::empty_state(
                        "music-podcast-symbolic",
                        "No episodes",
                        "This show has no episodes to play yet.",
                        None,
                    ));
                }
            }
            Err(e) if online::episodes(&ctx.feed).is_empty() => {
                let c = ctx.clone();
                ctx.loader.failed(&e, move || fetch_feed(c.clone()));
            }
            Err(e) => window::toast(&format!("Couldn't check for new episodes: {e:#}")),
        }
    });
}

/// A widget that lives exactly as long as this detail view (its subscriptions
/// end when the view is rebuilt for another show).
fn detail_marker(detail: &gtk::Box) -> gtk::Widget {
    let m = widgets::hbox(0);
    detail.append(&m);
    m.upcast()
}

// ---------- The page ----------

fn set_tab(id: &str) {
    UI.with(|u| {
        if let Some(u) = u.borrow().as_ref() {
            let mut c = u.seg.first_child();
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
            u.outer.set_visible_child_name("browse");
        }
    });
}

fn add_by_url() {
    widgets::ask_text(
        "Add a podcast",
        "The address of the show's RSS feed. You'll be subscribed to it.",
        "",
        "Subscribe",
        |url| {
            online::subscribe_url(&url, |feed| {
                if let Some(f) = feed {
                    open_feed(&f, None);
                }
            })
        },
    );
}

pub fn build(page: &Page) {
    let p = prefs::get();
    let tab = if TABS.iter().any(|(t, _)| *t == p.podcast_tab) { p.podcast_tab.clone() } else { "subscribed".into() };

    let outer = gtk::Stack::new();
    outer.set_vexpand(true);
    outer.set_transition_type(gtk::StackTransitionType::Crossfade);
    outer.set_transition_duration(if p.reduce_motion { 0 } else { 160 });

    let browse = widgets::vbox(0);
    let toolbar = widgets::hbox(12);
    toolbar.add_css_class("list-toolbar");
    let tabs = gtk::Stack::new();
    tabs.set_vexpand(true);
    tabs.set_transition_type(gtk::StackTransitionType::Crossfade);
    tabs.set_transition_duration(if p.reduce_motion { 0 } else { 140 });
    tabs.add_named(&subscribed_view(), Some("subscribed"));
    let (top, top_shows) = show_grid();
    let top_loader = Loader::new(&top.root);
    tabs.add_named(&top_loader.root, Some("top"));
    tabs.add_named(&categories_view(), Some("categories"));
    let (search, entry) = search_view();
    tabs.add_named(&search, Some("search"));

    let refresh = widgets::icon_button("view-refresh-symbolic", "Check for new episodes");
    refresh.connect_clicked(|_| online::refresh_all(true));
    let loaded: Rc<RefCell<HashSet<String>>> = Rc::default();
    let show_tab = {
        let (tabs, refresh, entry) = (tabs.clone(), refresh.clone(), entry.clone());
        move |id: &str| {
            tabs.set_visible_child_name(id);
            refresh.set_visible(id == "subscribed");
            if id == "search" {
                entry.grab_focus();
            }
            if id == "top" && loaded.borrow_mut().insert(id.to_string()) {
                let cc = country();
                load_shows(&top_loader, &top, &top_shows, std::sync::Arc::new(move || podcast::top(&cc, None)));
            }
        }
    };
    let s = show_tab.clone();
    let opts: Vec<(String, String)> = TABS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let seg = widgets::segmented(&opts, &tab, move |id| {
        s(&id);
        prefs::update(|p| p.podcast_tab = id);
    });
    toolbar.append(&seg);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    toolbar.append(&spacer);
    toolbar.append(&refresh);
    let add = widgets::labeled_button("list-add-symbolic", "Add by address");
    add.set_tooltip_text(Some("Subscribe to a show by its RSS feed"));
    add.connect_clicked(|_| add_by_url());
    toolbar.append(&add);
    browse.append(&toolbar);
    browse.append(&tabs);
    outer.add_named(&browse, Some("browse"));

    let detail = widgets::vbox(14);
    detail.add_css_class("podcast-detail");
    let detail_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&detail)
        .vexpand(true)
        .build();
    outer.add_named(&detail_scroll, Some("detail"));
    page.body.append(&outer);
    UI.with(|u| *u.borrow_mut() = Some(Ui { outer, detail, seg, open: String::new() }));
    show_tab(&tab);
}
