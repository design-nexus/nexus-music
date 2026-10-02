//! Now playing: the cover, the song, a full-size spectrum analyzer and what's
//! up next.

use crate::library::art::Cover;
use crate::library::store;
use crate::player::{self, Event, State, engine};
use crate::widgets::{self, Page};
use crate::{fmt, prefs, spectrum};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

const UP_NEXT: usize = 5;

fn link(class: &str) -> (gtk::Button, gtk::Label) {
    let b = gtk::Button::new();
    b.add_css_class("flat");
    b.add_css_class("link-button");
    b.add_css_class(class);
    b.set_halign(gtk::Align::Start);
    let l = widgets::label("", "");
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.set_child(Some(&l));
    (b, l)
}

pub fn build(page: &Page) {
    if let Some(e) = player::engine_error() {
        page.body.append(&widgets::banner(&format!("Playback isn't available: {}", super::glib_escape(&e)), true));
    } else {
        for (el, what) in engine::missing() {
            page.body.append(&widgets::banner(
                &format!("The {what} needs GStreamer's <tt>{el}</tt> element, from <b>gst-plugins-good</b>. Install it and restart Music."),
                true,
            ));
        }
    }

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    let empty = widgets::empty_state(
        "music-spectrum-symbolic",
        "Nothing playing",
        "Pick an album or a song, or press <b>Space</b> to play your whole library.",
        Some(("Play everything", Box::new(player::play))),
    );
    stack.add_named(&empty, Some("empty"));

    let content = widgets::vbox(22);
    // ----- Hero -----
    let hero = widgets::hbox(24);
    hero.add_css_class("now-hero");
    let cover = Cover::new(200, false);
    hero.append(&cover.root);
    let text = widgets::vbox(4);
    text.set_valign(gtk::Align::End);
    text.set_hexpand(true);
    let state = widgets::label("", "group-title");
    text.append(&state);
    let title = widgets::label("", "now-title");
    title.set_wrap(true);
    title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title.set_lines(2);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);
    let (artist_btn, artist_lbl) = link("now-artist");
    text.append(&artist_btn);
    let (album_btn, album_lbl) = link("now-album");
    text.append(&album_btn);
    let meta = widgets::label("", "dim");
    meta.add_css_class("mono");
    meta.add_css_class("detail-meta");
    text.append(&meta);
    hero.append(&text);
    content.append(&hero);

    // ----- Analyzer -----
    let an_card = widgets::vbox(10);
    an_card.set_vexpand(true);
    let an_head = widgets::hbox(12);
    let an_title = widgets::label("SPECTRUM", "group-title");
    an_title.set_hexpand(true);
    an_title.set_valign(gtk::Align::Center);
    an_head.append(&an_title);
    an_head.append(&widgets::segmented(
        &widgets::opts(&[("bars", "Bars"), ("line", "Line"), ("mirror", "Mirror")]),
        &prefs::get().spectrum_style,
        |v| {
            prefs::update(|p| p.spectrum_style = v);
            spectrum::redraw_all();
        },
    ));
    an_card.append(&an_head);
    let analyzer = spectrum::analyzer(false);
    analyzer.set_vexpand(true);
    analyzer.set_size_request(-1, 200);
    an_card.append(&analyzer);
    an_card.set_visible(player::has_spectrum());
    content.append(&an_card);

    // ----- Up next -----
    let next_group = widgets::vbox(6);
    next_group.append(&widgets::label("UP NEXT", "group-title"));
    let next_list = widgets::vbox(2);
    next_group.append(&next_list);
    content.append(&next_group);
    stack.add_named(&content, Some("playing"));
    page.body.append(&stack);

    let artist_name: Rc<RefCell<String>> = Rc::default();
    let album_key: Rc<RefCell<String>> = Rc::default();
    let n = artist_name.clone();
    artist_btn.connect_clicked(move |_| super::artists::show(&n.borrow()));
    let k = album_key.clone();
    album_btn.connect_clicked(move |_| super::albums::show(&k.borrow()));

    let refresh_next = move || {
        while let Some(c) = next_list.first_child() {
            next_list.remove(&c);
        }
        let (_, cursor) = player::queue_items();
        let upcoming = player::upcoming(UP_NEXT);
        next_group.set_visible(!upcoming.is_empty());
        let start = cursor.map_or(0, |c| c + 1);
        for (i, p) in upcoming.iter().enumerate() {
            let t = store::track_for(p);
            let b = gtk::Button::new();
            b.add_css_class("flat");
            b.add_css_class("next-row");
            let row = widgets::hbox(12);
            let num = widgets::label(&(i + 1).to_string(), "cell-dim");
            num.add_css_class("mono");
            num.set_width_chars(2);
            num.set_xalign(1.0);
            row.append(&num);
            let tt = widgets::label(&t.title, "");
            tt.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&tt);
            let ar = widgets::label(&t.artist, "dim");
            ar.set_ellipsize(gtk::pango::EllipsizeMode::End);
            ar.set_hexpand(true);
            row.append(&ar);
            let d = widgets::label(&fmt::time(t.duration), "cell-dim");
            d.add_css_class("mono");
            row.append(&d);
            b.set_child(Some(&row));
            let idx = start + i;
            b.set_tooltip_text(Some("Play now"));
            b.connect_clicked(move |_| player::jump(idx));
            next_list.append(&b);
        }
    };

    let refresh = {
        let stack = stack.clone();
        let refresh_next = refresh_next.clone();
        move |e: Event| match e {
            Event::Track => {
                match player::current() {
                    Some(t) => {
                        stack.set_visible_child_name("playing");
                        cover.set_key(&t.art);
                        title.set_text(&t.title);
                        artist_lbl.set_text(&t.artist);
                        *artist_name.borrow_mut() = t.album_artist_or_artist().to_string();
                        album_lbl.set_text(&match t.year {
                            Some(y) => format!("{} · {y}", t.album),
                            None => t.album.clone(),
                        });
                        *album_key.borrow_mut() = t.album_key();
                        album_btn.set_sensitive(store::album_of(&t).is_some());
                        artist_btn.set_sensitive(store::find(&t.path).is_some());
                        let mut m = vec![fmt::time(t.duration)];
                        if let Some(ext) = t.path.extension() {
                            m.push(ext.to_string_lossy().to_uppercase());
                        }
                        let plays = t.plays.get();
                        if plays > 0 {
                            m.push(fmt::count(plays as usize, "play", "plays"));
                        }
                        meta.set_text(&m.join(" · "));
                    }
                    None => stack.set_visible_child_name("empty"),
                }
                refresh_next();
            }
            Event::State => state.set_text(match player::state() {
                State::Playing => "PLAYING",
                State::Paused => "PAUSED",
                State::Stopped => "STOPPED",
            }),
            Event::Queue => refresh_next(),
            _ => {}
        }
    };
    for e in [Event::Track, Event::State] {
        refresh(e);
    }
    player::subscribe(&page.body, refresh);
}
