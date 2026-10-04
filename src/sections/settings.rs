use crate::library::store;
use crate::widgets::{self, Page};
use crate::{fmt, paths, player, prefs, spectrum, theme, window};
use gtk::prelude::*;
use gtk::{gio, glib};
use std::path::PathBuf;

/// Pick a folder and add it to the library (the empty states use this too).
pub fn choose_folder() {
    let dialog = gtk::FileDialog::builder().title("Choose a music folder").modal(true).build();
    let start = paths::music_dir();
    if start.is_dir() {
        dialog.set_initial_folder(Some(&gio::File::for_path(start)));
    }
    dialog.select_folder(window::window().as_ref(), gio::Cancellable::NONE, |res| {
        let Ok(file) = res else { return };
        let Some(path) = file.path() else { return };
        add_folder(path);
    });
}

fn add_folder(path: PathBuf) {
    let text = path.to_string_lossy().into_owned();
    let mut already = false;
    prefs::update(|p| {
        // Drop folders that no longer exist (the default ~/Music when it was never made).
        p.library_folders.retain(|f| std::path::Path::new(f).is_dir());
        if p.library_folders.iter().any(|f| path.starts_with(f)) {
            already = true;
        } else {
            // A parent folder replaces the folders inside it.
            p.library_folders.retain(|f| !std::path::Path::new(f).starts_with(&path));
            p.library_folders.push(text.clone());
        }
    });
    prefs::flush();
    if already {
        window::toast(&format!("{} is already in your library.", paths::pretty(&path)));
        return;
    }
    store::rescan(true);
    store::reload();
    refresh_folders();
}

thread_local! {
    static FOLDERS: std::cell::RefCell<Option<gtk::Box>> = const { std::cell::RefCell::new(None) };
}

fn refresh_folders() {
    let Some(list) = FOLDERS.with(|f| f.borrow().clone()) else { return };
    while let Some(c) = list.first_child() {
        list.remove(&c);
    }
    let tracks = store::tracks();
    let folders = prefs::get().library_folders;
    for f in &folders {
        let path = PathBuf::from(f);
        let n = tracks.iter().filter(|t| t.path.starts_with(&path)).count();
        let desc = if path.is_dir() { fmt::count(n, "song", "songs") } else { "This folder doesn't exist.".to_string() };
        let f2 = f.clone();
        let remove = widgets::two_click("Remove", "Click again to remove", move || {
            prefs::update(|p| p.library_folders.retain(|x| *x != f2));
            prefs::flush();
            store::rescan(true);
            refresh_folders();
        });
        remove.set_tooltip_text(Some("Take this folder's songs out of the library. Nothing is deleted from disk."));
        let row = widgets::row(&paths::pretty(&path), &desc, Some(remove.upcast_ref()));
        if let Some(title) = row.first_child().and_then(|t| t.first_child()) {
            title.add_css_class("mono");
        }
        list.append(&row);
    }
    if folders.is_empty() {
        list.append(&widgets::row("No folders", "Add the folder your music is in.", None));
    }
}

pub fn build(page: &Page) {
    let p = prefs::get();

    // ----- Library -----
    let g = page.group("Library");
    let list = widgets::vbox(6);
    g.add(&list);
    FOLDERS.with(|f| *f.borrow_mut() = Some(list.clone()));
    refresh_folders();
    store::subscribe(&list, |c| {
        if c == store::Change::Library {
            refresh_folders();
        }
    });
    let (r, _) =
        widgets::button_row("Add a folder", "Music anywhere under it joins the library.", "Add folder…", |_| choose_folder());
    g.add(&r);
    let (scan_row, scan_btn) =
        widgets::button_row("Rescan", "Read new and changed files now.", "Rescan now", |_| store::rescan(true));
    g.add(&scan_row);
    {
        let desc =
            scan_row.first_child().and_then(|t| t.first_child()).and_then(|t| t.next_sibling()).and_downcast::<gtk::Label>();
        let b = scan_btn.clone();
        let refresh = move || {
            let busy = store::scanning();
            b.set_sensitive(busy.is_none());
            if let Some(d) = &desc {
                d.set_text(&match busy {
                    Some((done, total)) if total > 0 => format!("Reading {} of {}…", fmt::thousands(done), fmt::thousands(total)),
                    Some(_) => "Looking for music…".into(),
                    None => format!(
                        "Read new and changed files now. {} in the library.",
                        fmt::count(store::tracks().len(), "song", "songs")
                    ),
                });
            }
        };
        refresh();
        store::subscribe(&scan_btn, move |_| refresh());
    }
    let (r, _) = widgets::switch_row(
        "Watch for changes",
        "Notice music being added, removed or renamed within a minute.",
        p.watch,
        |on| prefs::update(|p| p.watch = on),
    );
    g.add(&r);

    // ----- Radio and podcasts -----
    let g = page.group("Radio and podcasts");
    g.add(&country_row());
    let (r, _) = widgets::switch_row(
        "Check for new episodes",
        "Look for new episodes of your podcasts when Music opens.",
        p.podcast_refresh,
        |on| prefs::update(|p| p.podcast_refresh = on),
    );
    g.add(&r);

    // ----- Playback -----
    let g = page.group("Playback");
    let (r, _) = widgets::switch_row(
        "Gapless playback",
        "Songs run into each other with no silence, as on live albums.",
        p.gapless,
        |on| {
            prefs::update(|p| p.gapless = on);
            player::refresh_next();
        },
    );
    g.add(&r);
    g.add(&widgets::segmented_row(
        "Volume levelling",
        "Uses ReplayGain tags to even out loudness, per song or per album.",
        widgets::opts(&[("off", "Off"), ("track", "Song"), ("album", "Album")]),
        &p.replaygain,
        |v| {
            prefs::update(|p| p.replaygain = v);
            player::apply_eq();
        },
    ));
    let (r, _) =
        widgets::switch_row("Resume where I left off", "Bring back the queue and position when Music opens.", p.resume, |on| {
            prefs::update(|p| p.resume = on)
        });
    g.add(&r);
    let (r, _) = widgets::switch_row(
        "Song notifications",
        "Show the new song in a notification while this window isn't in front.",
        p.notify,
        |on| prefs::update(|p| p.notify = on),
    );
    g.add(&r);

    // ----- Spectrum -----
    let g = page.group("Spectrum analyzer");
    g.add(&widgets::segmented_row(
        "Style",
        "How Now playing draws the music's frequencies.",
        widgets::opts(&[("bars", "Bars"), ("line", "Line"), ("mirror", "Mirror")]),
        &p.spectrum_style,
        |v| {
            prefs::update(|p| p.spectrum_style = v);
            spectrum::redraw_all();
        },
    ));
    g.add(&widgets::segmented_row(
        "Detail",
        "Fewer, wider bars or many thin ones.",
        widgets::opts(&[("low", "Low"), ("medium", "Medium"), ("high", "High")]),
        &p.spectrum_density,
        |v| {
            prefs::update(|p| p.spectrum_density = v);
            spectrum::redraw_all();
        },
    ));
    let (r, _) =
        widgets::switch_row("Peak markers", "A cap that hangs at each bar's recent high, then falls.", p.spectrum_peaks, |on| {
            prefs::update(|p| p.spectrum_peaks = on);
            spectrum::redraw_all();
        });
    g.add(&r);

    // ----- This window -----
    let g = page.group("This window");
    let p = prefs::get();
    let app_themes = theme::all();
    let options: Vec<(String, String)> = app_themes.iter().map(|t| (t.id.clone(), t.name.clone())).collect();
    let (theme_row, theme_dd) = widgets::choice_row(
        "Theme",
        "Dracula, Catppuccin, Tokyo Night, One Dark Pro and more. Add your own in <tt>~/.config/nexus-music/themes</tt>.",
        options,
        &p.theme,
        |id| {
            prefs::update(|p| {
                p.theme = id;
                p.mode = prefs::ThemeMode::Theme;
            });
            theme::apply();
        },
    );
    theme_dd.set_sensitive(p.mode == prefs::ThemeMode::Theme || !theme::omarchy_available());

    if theme::omarchy_available() {
        let dd = theme_dd.clone();
        let (r, _) = widgets::switch_row(
            "Follow Omarchy theme",
            "Match the desktop's colours and update live whenever the Omarchy theme changes.",
            p.mode == prefs::ThemeMode::Omarchy,
            move |on| {
                prefs::update(|p| p.mode = if on { prefs::ThemeMode::Omarchy } else { prefs::ThemeMode::Theme });
                dd.set_sensitive(!on);
                theme::apply();
            },
        );
        g.add(&r);
    }
    g.add(&theme_row);

    let swatches = widgets::hbox(4);
    let refresh_swatches = {
        let swatches = swatches.clone();
        move || {
            while let Some(c) = swatches.first_child() {
                swatches.remove(&c);
            }
            let pal = theme::current_palette();
            for c in [&pal.bg, &pal.surface, &pal.muted, &pal.text, &pal.accent, &pal.danger] {
                let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                s.add_css_class("swatch");
                let provider = gtk::CssProvider::new();
                provider.load_from_string(&format!("box {{ background: {c}; }}"));
                #[allow(deprecated)]
                s.style_context().add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_USER);
                swatches.append(&s);
            }
        }
    };
    refresh_swatches();
    let last = std::cell::RefCell::new(theme::current_palette());
    let weak = swatches.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        let now = theme::current_palette();
        if *last.borrow() != now {
            *last.borrow_mut() = now;
            refresh_swatches();
        }
        glib::ControlFlow::Continue
    });
    g.add(&widgets::row("Current colours", "", Some(swatches.upcast_ref())));

    let (r, _) = widgets::switch_row(
        "Colours from cover",
        "Tint Now playing with the colour of the cover that's playing.",
        p.cover_colours,
        |on| {
            prefs::update(|p| p.cover_colours = on);
            player::metadata_changed();
        },
    );
    g.add(&r);
    let (r, _) = widgets::switch_row("Glow", "Soft accent glow around focused and selected elements.", p.glow, |on| {
        prefs::update(|p| p.glow = on);
        theme::apply();
    });
    g.add(&r);
    let (r, _) =
        widgets::switch_row("Reduce motion", "Turn off transitions and animations in this window.", p.reduce_motion, |on| {
            prefs::update(|p| p.reduce_motion = on);
            theme::apply();
        });
    g.add(&r);

    // ----- Keyboard -----
    let g = page.group("Keyboard");
    for (keys, what) in window::SHORTCUTS {
        g.add(&widgets::row(what, "", Some(widgets::key_caps(keys).upcast_ref())));
    }
    g.note("Media keys work through MPRIS. From a terminal or a binding: <tt>music --play-pause</tt>, <tt>--next</tt>, <tt>--previous</tt>.");
}

/// Your country, for local stations and the podcast charts. The list comes
/// from the radio directory, so it fills in once that answers.
fn country_row() -> gtk::Box {
    let auto = crate::online::radio::locale_country();
    let auto_label = if auto.is_empty() { "Automatic".to_string() } else { format!("Automatic ({auto})") };
    let current = prefs::get().radio_country;
    let ids: std::rc::Rc<std::cell::RefCell<Vec<String>>> = std::rc::Rc::new(std::cell::RefCell::new(vec![String::new()]));
    let mut names = vec![auto_label.clone()];
    if !current.is_empty() {
        ids.borrow_mut().push(current.clone());
        names.push(current.clone());
    }
    let model = gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
    let dd = gtk::DropDown::new(Some(model), None::<gtk::Expression>);
    dd.set_valign(gtk::Align::Center);
    dd.set_enable_search(true);
    dd.set_selected(if current.is_empty() { 0 } else { 1 });
    let syncing = std::rc::Rc::new(std::cell::Cell::new(false));
    let (i, s) = (ids.clone(), syncing.clone());
    dd.connect_selected_notify(move |d| {
        if s.get() {
            return;
        }
        if let Some(id) = i.borrow().get(d.selected() as usize) {
            let id = id.clone();
            prefs::update(|p| p.radio_country = id);
        }
    });
    let d = dd.clone();
    crate::cmd::background(crate::online::radio::countries, move |res| {
        let Ok(mut list) = res else { return };
        list.sort_by_key(|c| c.name.to_lowercase());
        let current = prefs::get().radio_country;
        let mut new_ids = vec![String::new()];
        let mut labels = vec![auto_label];
        for c in list {
            new_ids.push(c.code.to_ascii_uppercase());
            labels.push(c.name);
        }
        let selected = new_ids.iter().position(|c| c.eq_ignore_ascii_case(&current)).unwrap_or(0);
        syncing.set(true);
        *ids.borrow_mut() = new_ids;
        d.set_model(Some(&gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())));
        d.set_selected(selected as u32);
        syncing.set(false);
    });
    widgets::row("Country", "For local stations and the podcast charts.", Some(dd.upcast_ref()))
}
