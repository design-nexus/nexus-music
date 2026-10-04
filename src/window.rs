//! The main window: navigation sidebar with library search, a stack of pages
//! built the first time they're shown, and the player bar underneath.

use crate::library::store;
use crate::sections::{self, Section};
use crate::widgets;
use crate::{player, playerbar, prefs, theme};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: gtk::Box,
    nav_items: HashMap<String, gtk::Button>,
    playlist_box: gtk::Box,
    search: gtk::SearchEntry,
    /// The search shown from the icon-only sidebar.
    pop_search: gtk::SearchEntry,
    pages: HashMap<String, gtk::Widget>,
    sections: Vec<Section>,
    current: String,
    /// Where Esc in the search goes back to.
    before_search: String,
    overlay: gtk::Overlay,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A playlist asked for before the playlists finished loading.
    static PENDING: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if let Some(ui) = ui() {
        let window = ui.borrow().window.clone();
        if let Some(s) = section {
            navigate(s);
        }
        window.present();
        return;
    }
    theme::install();
    install_icons();
    build(app);
    let start = section.map(String::from).unwrap_or_else(|| prefs::get().last_section);
    navigate(&start);
    // Developer aid: MUSIC_SNAPSHOT=/path.png renders the window to a PNG
    // (invisibly) and quits, so layouts can be checked without a visible window.
    if let Some(out) = std::env::var_os("MUSIC_SNAPSHOT") {
        snapshot_and_quit(app, std::path::PathBuf::from(out));
        return;
    }
    if let Some(ui) = ui() {
        ui.borrow().window.present();
    }
    if prefs::take_broken() {
        toast("Your settings file couldn't be read, so defaults are in use. The old file is kept as settings.toml.bak.");
    }
}

/// Our own symbolic icons, for things the icon theme has no glyph for.
/// They're written to the cache once and added to the icon search path.
fn install_icons() {
    const ICONS: &[(&str, &str)] = &[
        ("music-note-symbolic.svg", include_str!("../data/icons/music-note-symbolic.svg")),
        ("music-playlist-symbolic.svg", include_str!("../data/icons/music-playlist-symbolic.svg")),
        ("music-queue-symbolic.svg", include_str!("../data/icons/music-queue-symbolic.svg")),
        ("music-equalizer-symbolic.svg", include_str!("../data/icons/music-equalizer-symbolic.svg")),
        ("music-genre-symbolic.svg", include_str!("../data/icons/music-genre-symbolic.svg")),
        ("music-spectrum-symbolic.svg", include_str!("../data/icons/music-spectrum-symbolic.svg")),
        ("music-radio-symbolic.svg", include_str!("../data/icons/music-radio-symbolic.svg")),
        ("music-podcast-symbolic.svg", include_str!("../data/icons/music-podcast-symbolic.svg")),
    ];
    let dir = crate::paths::cache_dir().join("icons");
    for (name, svg) in ICONS {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*svg) {
            let _ = crate::cmd::atomic_write(&path, svg);
        }
    }
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}

fn nav_button(icon: &str, title: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let l = widgets::label(title, "nav-label");
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&l);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    (button, l)
}

fn build(app: &gtk::Application) {
    let window =
        gtk::ApplicationWindow::builder().application(app).title("Music").default_width(1180).default_height(820).build();
    window.add_css_class("music-window");
    // No client-side titlebar: Hyprland manages the window.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some("io.github.design_nexus.Music"));

    let sections = sections::all();

    // ----- Sidebar -----
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);
    let heading = widgets::label("MUSIC", "menu-heading");
    heading.add_css_class("compact-hide");
    heading.set_hexpand(true);
    nav.append(&nav_head(&heading));

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search library"));
    search.add_css_class("settings-search");
    search.add_css_class("compact-hide");
    nav.append(&search);
    let (search_pop, pop_search) = compact_search(&nav);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut nav_items = HashMap::new();
    let playlist_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut last_group = "";
    for s in sections.iter().filter(|s| s.nav) {
        if s.group != last_group {
            let g = widgets::label(&s.group.to_uppercase(), "nav-group");
            g.add_css_class("compact-hide");
            list.append(&g);
            last_group = s.group;
        }
        let (button, label) = nav_button(s.icon, s.title, s.description);
        label.add_css_class("compact-hide");
        let id = s.id;
        button.connect_clicked(move |_| navigate(id));
        list.append(&button);
        nav_items.insert(s.id.to_string(), button);
        if s.id == "queue" {
            list.append(&playlist_box);
            let (new_button, label) = nav_button("list-add-symbolic", "New playlist", "Make a playlist, or import one");
            label.add_css_class("compact-hide");
            new_button.add_css_class("nav-add");
            new_button.connect_clicked(|_| sections::playlist::new_dialog(Vec::new()));
            list.append(&new_button);
        }
    }
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls with wheel, trackpad and keyboard; no visible scrollbar.
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    footer.add_css_class("nav-footer");
    footer.add_css_class("compact-hide");
    let version = widgets::label(concat!("Music ", env!("CARGO_PKG_VERSION")), "dim");
    version.set_hexpand(true);
    footer.append(&version);
    let scan_label = widgets::label("", "nav-readout");
    scan_label.add_css_class("mono");
    footer.append(&scan_label);
    nav.append(&footer);
    {
        let l = scan_label.clone();
        store::subscribe(&scan_label, move |c| {
            if c == store::Change::Scan {
                l.set_text(&match store::scanning() {
                    Some((done, total)) if total > 0 => format!("{}%", done * 100 / total),
                    Some(_) => "Scanning".into(),
                    None => String::new(),
                });
            }
        });
    }

    // ----- Content -----
    let stack = gtk::Stack::new();
    stack.add_css_class("settings-content");
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.append(&stack);
    content.append(&playerbar::build());

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&nav);
    body.append(&content);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&body));
    window.set_child(Some(&overlay));

    install_keys(&window, &search, &nav, &search_pop, &pop_search);
    search.connect_search_changed(|e| on_search(&e.text()));
    search.connect_activate(|_| sections::search::focus_results());
    search.connect_stop_search(|e| e.set_text(""));

    // Narrow windows (a tiled half-screen) get an icon-only sidebar.
    let apply_width = {
        let nav = nav.clone();
        move |w: &gtk::ApplicationWindow| {
            let width = if w.width() > 0 { w.width() } else { w.default_width() };
            let narrow = width > 0 && width < 980;
            if narrow == NARROW.with(|n| n.get()) && nav.has_css_class("sized") {
                return;
            }
            nav.add_css_class("sized");
            set_narrow(narrow);
            apply_compact(&nav, narrow || prefs::get().sidebar_collapsed);
        }
    };
    let aw = apply_width.clone();
    window.connect_default_width_notify(move |w| aw(w));
    let aw = apply_width.clone();
    window.connect_realize(move |w| aw(w));
    // Tiled windows are resized by the compositor; watch the real size too.
    let w2 = window.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
        apply_width(&w2);
        glib::ControlFlow::Continue
    });

    let ui = Ui {
        window,
        stack,
        nav,
        nav_items,
        playlist_box: playlist_box.clone(),
        search,
        pop_search,
        pages: HashMap::new(),
        sections,
        current: String::new(),
        before_search: String::new(),
        overlay,
    };
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(RefCell::new(ui))));

    store::subscribe(&playlist_box, |c| {
        if c == store::Change::Playlists {
            refresh_playlists();
        }
    });
    refresh_playlists();
}

/// The icon-only sidebar has no room for the search field: a search button
/// opens one in a popover instead.
fn compact_search(nav: &gtk::Box) -> (gtk::Popover, gtk::SearchEntry) {
    let (button, label) = nav_button("system-search-symbolic", "Search", "Search the library (Ctrl+F)");
    label.add_css_class("compact-hide");
    button.add_css_class("compact-show");
    button.set_visible(false);
    let pop = gtk::Popover::new();
    pop.set_parent(&button);
    pop.set_position(gtk::PositionType::Right);
    pop.add_css_class("search-popover");
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some("Search library"));
    entry.add_css_class("settings-search");
    entry.set_width_chars(28);
    pop.set_child(Some(&entry));
    entry.connect_search_changed(|e| on_search(&e.text()));
    let p = pop.clone();
    entry.connect_activate(move |_| {
        p.popdown();
        sections::search::focus_results();
    });
    let p = pop.clone();
    entry.connect_stop_search(move |e| {
        e.set_text("");
        p.popdown();
    });
    let (p, e) = (pop.clone(), entry.clone());
    button.connect_clicked(move |_| {
        p.popup();
        e.grab_focus();
    });
    nav.append(&button);
    (pop, entry)
}

/// The button that collapses the sidebar to icons, beside the app heading.
fn nav_head(heading: &gtk::Label) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("nav-head");
    row.append(heading);
    let button = gtk::Button::from_icon_name("sidebar-show-symbolic");
    button.add_css_class("nav-collapse");
    button.set_tooltip_text(Some("Collapse or expand the sidebar (Ctrl+B)"));
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(|_| toggle_sidebar());
    row.append(&button);
    row
}

/// The sidebar shows only icons: hide the labels (everything marked
/// `compact-hide`), show what's marked `compact-show`, centre the icons and the toggle.
fn apply_compact(nav: &gtk::Box, compact: bool) {
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }
    set_compact_hidden(nav, compact);
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    let Some(ui) = ui() else { return };
    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn set_compact_hidden(root: &gtk::Box, compact: bool) {
    fn walk(w: &gtk::Widget, compact: bool) {
        if w.has_css_class("compact-hide") {
            w.set_visible(!compact);
        }
        if w.has_css_class("compact-show") {
            w.set_visible(compact);
        }
        // Icon-only: centre the icon in its pill, and the toggle in the column.
        if w.has_css_class("nav-item")
            && let Some(content) = w.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
        }
        if w.has_css_class("nav-collapse") {
            w.set_halign(if compact { gtk::Align::Center } else { gtk::Align::End });
            w.set_hexpand(compact);
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, compact);
            child = c.next_sibling();
        }
    }
    walk(root.upcast_ref(), compact);
}

fn install_keys(
    window: &gtk::ApplicationWindow,
    search: &gtk::SearchEntry,
    nav: &gtk::Box,
    search_pop: &gtk::Popover,
    pop_search: &gtk::SearchEntry,
) {
    // Capture phase: these work wherever focus is, except while typing.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s2 = search.clone();
    let w2 = window.clone();
    let (nav, search_pop, pop_search) = (nav.clone(), search_pop.clone(), pop_search.clone());
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let typing = gtk::prelude::GtkWindowExt::focus(&w2).is_some_and(|f| {
            f.is::<gtk::Text>() || f.ancestor(gtk::Entry::static_type()).is_some() || f.is::<gtk::SearchEntry>()
        });
        match key {
            gdk::Key::f if ctrl => {
                if nav.has_css_class("compact") {
                    search_pop.popup();
                    pop_search.grab_focus();
                } else {
                    s2.grab_focus();
                }
                glib::Propagation::Stop
            }
            gdk::Key::b if ctrl => {
                toggle_sidebar();
                glib::Propagation::Stop
            }
            gdk::Key::q | gdk::Key::w if ctrl => {
                w2.close();
                glib::Propagation::Stop
            }
            gdk::Key::space if !typing => {
                player::toggle();
                glib::Propagation::Stop
            }
            gdk::Key::Left if ctrl && !typing => {
                player::previous();
                glib::Propagation::Stop
            }
            gdk::Key::Right if ctrl && !typing => {
                player::next();
                glib::Propagation::Stop
            }
            gdk::Key::Escape if !s2.text().is_empty() => {
                s2.set_text("");
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);

    // Bubble phase: plain arrows seek only when nothing focused used them.
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, mods| {
        if !mods.is_empty() {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Left => {
                player::seek_by(-5.0);
                glib::Propagation::Stop
            }
            gdk::Key::Right => {
                player::seek_by(5.0);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);
}

fn on_search(text: &str) {
    let Some(ui) = ui() else { return };
    let q = text.trim().to_string();
    if q.is_empty() {
        let back = std::mem::take(&mut ui.borrow_mut().before_search);
        if ui.borrow().current == "search" {
            navigate(if back.is_empty() { "albums" } else { &back });
        }
        return;
    }
    if ui.borrow().current != "search" {
        let cur = ui.borrow().current.clone();
        ui.borrow_mut().before_search = cur;
    }
    navigate("search");
    sections::search::set_query(&q);
}

fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    playerbar::set_narrow(narrow);
    crate::tracklist::set_narrow(narrow);
    let Some(ui) = ui() else { return };
    for page in ui.borrow().pages.values() {
        mark_page(page, narrow);
    }
}

pub fn narrow() -> bool {
    NARROW.with(|n| n.get())
}

fn mark_page(page: &gtk::Widget, narrow: bool) {
    let body = page
        .downcast_ref::<gtk::ScrolledWindow>()
        .and_then(|s| s.child())
        .and_then(|v| v.first_child())
        .unwrap_or_else(|| page.clone());
    if narrow {
        body.add_css_class("narrow");
    } else {
        body.remove_css_class("narrow");
    }
}

fn ensure_built(id: &str) -> bool {
    let Some(ui) = ui() else { return false };
    if ui.borrow().pages.contains_key(id) {
        return true;
    }
    let page: gtk::Widget = if let Some(pid) = id.strip_prefix("playlist:").and_then(|p| p.parse::<i64>().ok()) {
        if !store::playlists().iter().any(|p| p.id == pid) {
            return false;
        }
        sections::playlist::build(pid)
    } else {
        let section = {
            let u = ui.borrow();
            u.sections.iter().find(|s| s.id == id).map(|s| (s.id, s.title, s.description, (s.files)(), s.build, s.fill))
        };
        let Some((sid, title, description, files, build, fill)) = section else { return false };
        let page = widgets::page(sid, title, description, &files);
        if fill {
            page.fill();
        }
        build(&page);
        page.root.upcast()
    };
    mark_page(&page, narrow());
    let stack = ui.borrow().stack.clone();
    stack.add_named(&page, Some(id));
    ui.borrow_mut().pages.insert(id.to_string(), page);
    true
}

pub fn navigate(id: &str) {
    let Some(ui) = ui() else { return };
    if id.starts_with("playlist:") && !store::loaded() {
        PENDING.with(|p| *p.borrow_mut() = Some(id.to_string()));
    }
    let id = if ensure_built(id) { id.to_string() } else { "albums".to_string() };
    if !ensure_built(&id) {
        return;
    }
    let mut u = ui.borrow_mut();
    if let Some(prev) = u.nav_items.get(&u.current) {
        prev.remove_css_class("active");
    }
    if let Some(b) = u.nav_items.get(&id) {
        b.add_css_class("active");
    }
    u.stack.set_visible_child_name(&id);
    u.current = id.clone();
    let search = u.search.clone();
    let pop_search = u.pop_search.clone();
    drop(u);
    if id != "search" {
        if !search.text().is_empty() || !pop_search.text().is_empty() {
            ui.borrow_mut().before_search.clear();
            search.set_text("");
            pop_search.set_text("");
        }
        prefs::update(|p| p.last_section = id);
    }
}

/// Drop a page so it's rebuilt next time (a deleted playlist).
pub fn remove_page(id: &str) {
    let Some(ui) = ui() else { return };
    let old = ui.borrow_mut().pages.remove(id);
    if let Some(old) = old {
        ui.borrow().stack.remove(&old);
    }
    if ui.borrow().current == id {
        navigate("queue");
    }
}

fn refresh_playlists() {
    let Some(ui) = ui() else { return };
    let bx = ui.borrow().playlist_box.clone();
    while let Some(c) = bx.first_child() {
        bx.remove(&c);
    }
    ui.borrow_mut().nav_items.retain(|k, _| !k.starts_with("playlist:"));
    let current = ui.borrow().current.clone();
    for p in store::playlists() {
        let (button, label) = nav_button("music-playlist-symbolic", &p.name, &p.name);
        label.add_css_class("compact-hide");
        // Icon-only, every playlist would look the same: show its initial instead.
        if let Some(content) = button.child().and_downcast::<gtk::Box>() {
            if let Some(icon) = content.first_child() {
                icon.add_css_class("compact-hide");
            }
            let initial = widgets::label(&initial_of(&p.name), "nav-initial");
            initial.add_css_class("compact-show");
            initial.set_visible(false);
            content.prepend(&initial);
        }
        let id = format!("playlist:{}", p.id);
        if id == current {
            button.add_css_class("active");
        }
        let target = id.clone();
        button.connect_clicked(move |_| navigate(&target));
        sections::playlist::accept_drops(&button, p.id);
        bx.append(&button);
        ui.borrow_mut().nav_items.insert(id, button);
    }
    let nav = ui.borrow().nav.clone();
    set_compact_hidden(&nav, nav.has_css_class("compact"));
    if let Some(id) = PENDING.with(|p| p.borrow_mut().take()) {
        navigate(&id);
    }
}

/// The first letter or digit of a name, for the icon-only sidebar.
fn initial_of(name: &str) -> String {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "#".into())
}

/// Show a short message at the bottom of the window.
pub fn toast(message: &str) {
    show_toast(message, None);
}

/// A message with a button (Undo), shown a little longer.
pub fn toast_action(message: &str, label: &str, action: impl Fn() + 'static) {
    show_toast(message, Some((label, Box::new(action))));
}

thread_local! {
    /// The toast showing now; a new one replaces it.
    static TOAST: RefCell<Option<gtk::Box>> = const { RefCell::new(None) };
}

type ToastAction<'a> = Option<(&'a str, Box<dyn Fn()>)>;

fn show_toast(message: &str, action: ToastAction<'_>) {
    let Some(ui) = ui() else {
        eprintln!("music: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    if let Some(old) = TOAST.with(|t| t.borrow_mut().take())
        && old.parent().is_some()
    {
        overlay.remove_overlay(&old);
    }
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(70);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bx.add_css_class("toast");
    bx.append(&label);
    let has_action = action.is_some();
    if let Some((text, f)) = action {
        let b = gtk::Button::with_label(text);
        b.add_css_class("flat");
        b.add_css_class("toast-action");
        b.set_valign(gtk::Align::Center);
        let (o, w) = (overlay.clone(), bx.downgrade());
        b.connect_clicked(move |_| {
            f();
            if let Some(w) = w.upgrade()
                && w.parent().is_some()
            {
                o.remove_overlay(&w);
            }
        });
        bx.append(&b);
    }
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    overlay.add_overlay(&bx);
    TOAST.with(|t| *t.borrow_mut() = Some(bx.clone()));
    let ms = if has_action { 6000 } else { 3500 };
    glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
        if bx.parent().is_some() {
            overlay.remove_overlay(&bx);
        }
    });
}

pub fn current() -> String {
    ui().map(|u| u.borrow().current.clone()).unwrap_or_default()
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

pub fn is_active() -> bool {
    window().is_some_and(|w| w.is_active() && w.is_visible())
}

fn snapshot_and_quit(app: &gtk::Application, out: std::path::PathBuf) {
    let Some(ui) = ui() else { return };
    let window = ui.borrow().window.clone();
    window.set_opacity(0.01);
    // A distinct title lets a window rule float it at a set size for screenshots.
    window.set_title(Some("Music snapshot"));
    window.set_default_size(
        std::env::var("MUSIC_SNAPSHOT_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1180),
        std::env::var("MUSIC_SNAPSHOT_H").ok().and_then(|v| v.parse().ok()).unwrap_or(820),
    );
    window.present();
    if std::env::var_os("MUSIC_SNAPSHOT_MAX").is_some() {
        window.maximize();
    }
    let app = app.clone();
    let delay: u64 = std::env::var("MUSIC_SNAPSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(2500);
    if std::env::var_os("MUSIC_SNAPSHOT_PLAY").is_some() {
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), player::play);
    }
    // MUSIC_SNAPSHOT_ALBUM=<part of a title> opens that album first.
    if let Ok(want) = std::env::var("MUSIC_SNAPSHOT_ALBUM") {
        glib::timeout_add_local_once(std::time::Duration::from_millis(delay.saturating_sub(800)), move || {
            if let Some(a) = store::albums().into_iter().find(|a| a.title.contains(&want)) {
                sections::albums::show(&a.key);
            }
        });
    }
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        if let Some(child) = window.child() {
            let paintable = gtk::WidgetPaintable::new(Some(&child));
            let (w, h) = (child.width(), child.height());
            let snapshot = gtk::Snapshot::new();
            snapshot.append_color(&gdk::RGBA::BLACK, &gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
            paintable.snapshot(&snapshot, w as f64, h as f64);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
                let texture = renderer.render_texture(node, None);
                match texture.save_to_png(&out) {
                    Ok(()) => println!("snapshot {w}x{h} -> {}", out.display()),
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
        }
        player::save();
        app.quit();
    });
}
