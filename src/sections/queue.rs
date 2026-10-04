//! Queue: what's playing now and next. Double-click jumps; the menu removes.

use crate::library::store;
use crate::tracklist::{Col, Options, TrackTable};
use crate::widgets::{self, Page};
use crate::{fmt, player, window};
use gtk::prelude::*;
use std::rc::Rc;

pub fn build(page: &Page) {
    let toolbar = widgets::hbox(10);
    toolbar.add_css_class("toolbar");
    let summary = widgets::label("", "dim");
    summary.add_css_class("mono");
    summary.set_hexpand(true);
    toolbar.append(&summary);
    let save = widgets::labeled_button("music-playlist-symbolic", "Save as playlist");
    save.connect_clicked(|_| {
        let (items, _) = player::queue_items();
        if !items.is_empty() {
            super::playlist::new_dialog(items);
        }
    });
    toolbar.append(&save);
    let clear = gtk::Button::with_label("Clear");
    clear.set_tooltip_text(Some("Empty the queue"));
    clear.connect_clicked(|_| {
        let before = std::cell::RefCell::new(Some(player::snapshot()));
        player::clear();
        window::toast_action("Cleared the queue.", "Undo", move || {
            if let Some(s) = before.borrow_mut().take() {
                        player::restore_snapshot(s);
                    }
        });
    });
    toolbar.append(&clear);
    page.body.append(&toolbar);

    let table = TrackTable::new(Options {
        cols: &[Col::Num, Col::Title, Col::Artist, Col::Album, Col::Time],
        sortable: false,
        positions: true,
        on_activate: Some(Rc::new(player::jump)),
        extra: vec![(
            "Remove from queue",
            Rc::new(|mut idx: Vec<usize>| {
                let before = std::cell::RefCell::new(Some(player::snapshot()));
                // Highest first, so earlier removals don't shift later ones.
                idx.sort_unstable_by(|a, b| b.cmp(a));
                let n = idx.len();
                for i in idx {
                    player::remove(i);
                }
                window::toast_action(&format!("Removed {} from the queue.", fmt::count(n, "song", "songs")), "Undo", move || {
                    if let Some(s) = before.borrow_mut().take() {
                        player::restore_snapshot(s);
                    }
                });
            }),
        )],
        reorder: Some(Rc::new(|rows: Vec<usize>, to: usize| player::move_items(&rows, to))),
        ..Default::default()
    });
    let empty = widgets::empty_state(
        "music-queue-symbolic",
        "The queue is empty",
        "Play an album or a playlist, or pick songs and choose <b>Add to queue</b>.",
        Some((
            "Play everything",
            Box::new(|| {
                let all: Vec<_> = store::tracks().iter().map(|t| t.path.clone()).collect();
                player::shuffle_tracks(all);
            }),
        )),
    );
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&table.root, Some("table"));
    stack.add_named(&empty, Some("empty"));
    page.body.append(&stack);

    let refresh = {
        let (table, stack) = (table.clone(), stack.clone());
        move || {
            let (items, cursor) = player::queue_items();
            let tracks: Vec<_> = items.iter().map(|p| store::track_for(p)).collect();
            let secs: f64 = tracks.iter().map(|t| t.duration).sum();
            let left: f64 = cursor.map_or(secs, |c| tracks.iter().skip(c).map(|t| t.duration).sum());
            summary.set_text(&if tracks.is_empty() {
                String::new()
            } else {
                format!("{} · {} · {} left", fmt::count(tracks.len(), "song", "songs"), fmt::total(secs), fmt::total(left))
            });
            table.set(&tracks);
            save.set_sensitive(!tracks.is_empty());
            clear.set_sensitive(!tracks.is_empty());
            stack.set_visible_child_name(if tracks.is_empty() { "empty" } else { "table" });
            if let Some(c) = cursor
                && window::current() == "queue"
                && (c as u32) < table.len()
            {
                table.view.scroll_to(c as u32, None, gtk::ListScrollFlags::NONE, None);
            }
        }
    };
    refresh();
    let r = refresh.clone();
    player::subscribe(&page.body, move |e| {
        if matches!(e, player::Event::Queue | player::Event::Track) {
            r();
        }
    });
}
