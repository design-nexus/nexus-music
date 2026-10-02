//! The bar under every page: what's playing, transport, seek, volume and a
//! small spectrum strip.

use crate::library::art::Cover;
use crate::player::{self, Event, State, queue::Repeat};
use crate::{fmt, prefs, spectrum, widgets, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static NARROW_PARTS: RefCell<Vec<gtk::Widget>> = const { RefCell::new(Vec::new()) };
}

pub fn set_narrow(narrow: bool) {
    NARROW_PARTS.with(|p| {
        for w in p.borrow().iter() {
            w.set_visible(!narrow);
        }
    });
}

fn volume_icon(v: f64, muted: bool) -> &'static str {
    if muted || v <= 0.001 {
        "audio-volume-muted-symbolic"
    } else if v < 0.34 {
        "audio-volume-low-symbolic"
    } else if v < 0.67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

pub fn build() -> gtk::Box {
    let bar = widgets::hbox(16);
    bar.add_css_class("player-bar");

    // ----- Now playing -----
    let info = widgets::hbox(12);
    info.add_css_class("player-info");
    info.set_size_request(220, -1);
    let cover = Cover::new(52, true);
    cover.root.set_valign(gtk::Align::Center);
    info.append(&cover.root);
    let text = widgets::vbox(2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let title = widgets::label("Nothing playing", "player-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(28);
    let artist = widgets::label("", "player-artist");
    artist.set_ellipsize(gtk::pango::EllipsizeMode::End);
    artist.set_max_width_chars(28);
    text.append(&title);
    text.append(&artist);
    info.append(&text);
    let click = gtk::GestureClick::new();
    click.connect_released(|_, _, _, _| window::navigate("now-playing"));
    info.add_controller(click);
    info.set_cursor_from_name(Some("pointer"));
    bar.append(&info);

    // ----- Transport + seek -----
    let center = widgets::vbox(2);
    center.set_hexpand(true);
    center.set_size_request(260, -1);
    center.set_valign(gtk::Align::Center);
    let controls = widgets::hbox(6);
    controls.set_halign(gtk::Align::Center);
    let shuffle = gtk::ToggleButton::new();
    shuffle.set_icon_name("media-playlist-shuffle-symbolic");
    shuffle.add_css_class("flat");
    shuffle.add_css_class("icon-button");
    shuffle.add_css_class("transport-toggle");
    shuffle.set_tooltip_text(Some("Shuffle"));
    let prev = widgets::icon_button("media-skip-backward-symbolic", "Previous (Ctrl+←)");
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.add_css_class("play-button");
    play.set_tooltip_text(Some("Play (Space)"));
    let next = widgets::icon_button("media-skip-forward-symbolic", "Next (Ctrl+→)");
    let repeat = gtk::ToggleButton::new();
    repeat.set_icon_name("media-playlist-repeat-symbolic");
    repeat.add_css_class("flat");
    repeat.add_css_class("icon-button");
    repeat.add_css_class("transport-toggle");
    for w in [shuffle.upcast_ref::<gtk::Widget>(), prev.upcast_ref(), play.upcast_ref(), next.upcast_ref(), repeat.upcast_ref()] {
        w.set_focus_on_click(false);
        controls.append(w);
    }
    center.append(&controls);

    let seek_row = widgets::hbox(10);
    let pos = widgets::label("0:00", "time-readout");
    pos.add_css_class("mono");
    pos.set_xalign(1.0);
    pos.set_width_chars(5);
    let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
    seek.set_draw_value(false);
    seek.set_hexpand(true);
    seek.add_css_class("seek");
    seek.set_focus_on_click(false);
    let dur = widgets::label("0:00", "time-readout");
    dur.add_css_class("mono");
    dur.set_width_chars(5);
    seek_row.append(&pos);
    seek_row.append(&seek);
    seek_row.append(&dur);
    center.append(&seek_row);
    bar.append(&center);

    // ----- Spectrum strip + volume -----
    let right = widgets::hbox(8);
    right.set_valign(gtk::Align::Center);
    right.set_halign(gtk::Align::End);
    let strip = spectrum::analyzer(true);
    strip.set_size_request(110, 34);
    strip.set_hexpand(false);
    strip.set_valign(gtk::Align::Center);
    strip.set_tooltip_text(Some("Open Now playing"));
    let sclick = gtk::GestureClick::new();
    sclick.connect_released(|_, _, _, _| window::navigate("now-playing"));
    strip.add_controller(sclick);
    right.append(&strip);
    let p = prefs::get();
    let mute = widgets::icon_button(volume_icon(p.volume, p.muted), "Mute");
    mute.set_focus_on_click(false);
    right.append(&mute);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    volume.set_draw_value(false);
    volume.set_value(p.volume);
    volume.set_size_request(100, -1);
    volume.add_css_class("volume");
    volume.set_focus_on_click(false);
    volume.set_tooltip_text(Some(&format!("{}%", (p.volume * 100.0).round())));
    right.append(&volume);
    bar.append(&right);
    NARROW_PARTS.with(|n| {
        let mut n = n.borrow_mut();
        n.push(strip.clone().upcast());
        n.push(volume.clone().upcast());
    });

    // ----- Wiring -----
    play.connect_clicked(|_| player::toggle());
    prev.connect_clicked(|_| player::previous());
    next.connect_clicked(|_| player::next());
    let syncing = Rc::new(Cell::new(false));
    let s = syncing.clone();
    shuffle.connect_toggled(move |b| {
        if !s.get() {
            player::set_shuffle(b.is_active());
        }
    });
    let s = syncing.clone();
    repeat.connect_toggled(move |_| {
        if !s.get() {
            player::cycle_repeat();
        }
    });
    mute.connect_clicked(|_| player::set_muted(!prefs::get().muted));
    volume.connect_change_value(|s, _, v| {
        let v = v.clamp(0.0, 1.0);
        player::set_volume(v);
        if prefs::get().muted && v > 0.0 {
            player::set_muted(false);
        }
        s.set_tooltip_text(Some(&format!("{}%", (v * 100.0).round())));
        glib::Propagation::Proceed
    });
    // Scroll on the volume icon nudges the volume.
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(|_, _, dy| {
        player::set_volume((prefs::get().volume - dy * 0.05).clamp(0.0, 1.0));
        glib::Propagation::Stop
    });
    mute.add_controller(scroll);

    // While the user drags the seek bar, don't move it under them.
    let dragging: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let d = dragging.clone();
    let pos_label = pos.clone();
    seek.connect_change_value(move |_, _, v| {
        let gen_ = d.get().wrapping_add(1);
        d.set(gen_);
        pos_label.set_text(&fmt::time(v));
        player::seek(v);
        let d2 = d.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(350), move || {
            if d2.get() == gen_ {
                d2.set(0);
            }
        });
        glib::Propagation::Proceed
    });

    let refresh = {
        let (cover, title, artist, play, shuffle, repeat, seek, pos, dur, mute, volume, syncing, prev, next) = (
            cover.clone(),
            title.clone(),
            artist.clone(),
            play.clone(),
            shuffle.clone(),
            repeat.clone(),
            seek.clone(),
            pos.clone(),
            dur.clone(),
            mute.clone(),
            volume.clone(),
            syncing.clone(),
            prev.clone(),
            next.clone(),
        );
        let dragging = dragging.clone();
        move |e: Event| match e {
            Event::Track => {
                match player::current() {
                    Some(t) => {
                        title.set_text(&t.title);
                        artist.set_text(&format!("{} — {}", t.artist, t.album));
                        title.set_tooltip_text(Some(&t.title));
                        cover.set_key(&t.art);
                    }
                    None => {
                        title.set_text("Nothing playing");
                        artist.set_text("");
                        cover.set_key("");
                    }
                }
                let d = player::duration();
                seek.set_range(0.0, d.max(1.0));
                dur.set_text(&fmt::time(d));
                prev.set_sensitive(player::can_previous());
                next.set_sensitive(player::can_next());
            }
            Event::State => {
                let playing = player::state() == State::Playing;
                play.set_icon_name(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
                play.set_tooltip_text(Some(if playing { "Pause (Space)" } else { "Play (Space)" }));
            }
            Event::Position | Event::Seeked => {
                if dragging.get() != 0 && e == Event::Position {
                    return;
                }
                let d = player::duration();
                if (seek.adjustment().upper() - d.max(1.0)).abs() > 0.5 {
                    seek.set_range(0.0, d.max(1.0));
                    dur.set_text(&fmt::time(d));
                }
                let p = player::position();
                seek.set_value(p);
                pos.set_text(&fmt::time(p));
            }
            Event::Queue => {
                prev.set_sensitive(player::can_previous());
                next.set_sensitive(player::can_next());
            }
            Event::Options => {
                syncing.set(true);
                shuffle.set_active(player::shuffle());
                let r = player::repeat();
                repeat.set_active(r != Repeat::Off);
                repeat.set_icon_name(if r == Repeat::One {
                    "media-playlist-repeat-song-symbolic"
                } else {
                    "media-playlist-repeat-symbolic"
                });
                repeat.set_tooltip_text(Some(match r {
                    Repeat::Off => "Repeat: off",
                    Repeat::All => "Repeat: all",
                    Repeat::One => "Repeat: this song",
                }));
                syncing.set(false);
                let p = prefs::get();
                mute.set_icon_name(volume_icon(p.volume, p.muted));
                mute.set_tooltip_text(Some(if p.muted { "Unmute" } else { "Mute" }));
                if (volume.value() - p.volume).abs() > 0.005 {
                    volume.set_value(p.volume);
                }
            }
        }
    };
    for e in [Event::Track, Event::State, Event::Seeked, Event::Queue, Event::Options] {
        refresh(e);
    }
    player::subscribe(&bar, refresh);
    bar
}
