//! The equalizer: a preamp and ten bands as vertical faders, with presets.

use crate::player::{self, engine};
use crate::prefs::{self, EqPreset};
use crate::widgets::{self, Page};
use crate::{eq, fmt, window};
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

struct Fader {
    scale: gtk::Scale,
    value: gtk::Label,
}

fn fader(label: &str, range: (f64, f64), value: f64, preamp: bool) -> (gtk::Box, Fader) {
    let b = widgets::vbox(6);
    b.add_css_class("fader");
    b.set_hexpand(true);
    if preamp {
        b.add_css_class("preamp");
    }
    let v = gtk::Label::new(Some(&fmt::db(value)));
    v.add_css_class("fader-value");
    v.add_css_class("mono");
    let scale = gtk::Scale::with_range(gtk::Orientation::Vertical, range.0, range.1, 0.5);
    scale.add_css_class("fader-scale");
    scale.set_inverted(true);
    scale.set_draw_value(false);
    scale.set_value(value);
    scale.set_vexpand(true);
    scale.set_halign(gtk::Align::Center);
    // Fine steps with the keyboard, snap to 0.5 dB when dragging.
    scale.set_increments(0.5, 3.0);
    let l = gtk::Label::new(Some(label));
    l.add_css_class("fader-label");
    b.append(&v);
    b.append(&scale);
    b.append(&l);
    (b, Fader { scale, value: v })
}

pub fn build(page: &Page) {
    if !player::has_eq() {
        page.body.append(&widgets::banner(
            "The equalizer needs GStreamer's <tt>equalizer-10bands</tt> element, from <b>gst-plugins-good</b>. Install it and restart Music.",
            true,
        ));
    }
    let p = prefs::get();
    let g = page.group("Equalizer");
    let panel = widgets::hbox(0);
    panel.add_css_class("eq-panel");
    if !p.eq_enabled {
        panel.add_css_class("bypassed");
    }
    let pn = panel.clone();
    let (r, _) =
        widgets::switch_row("Equalizer", "Shape the sound. When it's off, music plays untouched.", p.eq_enabled, move |on| {
            prefs::update(|p| p.eq_enabled = on);
            if on {
                pn.remove_css_class("bypassed");
            } else {
                pn.add_css_class("bypassed");
            }
            player::apply_eq();
        });
    g.add(&r);

    // ----- Faders -----
    let (pre_box, pre) = fader("Preamp", (-12.0, 12.0), p.eq_preamp, true);
    panel.append(&pre_box);
    let div = gtk::Box::new(gtk::Orientation::Vertical, 0);
    div.add_css_class("eq-divider");
    panel.append(&div);
    let mut bands = Vec::new();
    for (i, f) in engine::EQ_LABELS.iter().enumerate() {
        let (b, fd) = fader(&fmt::hz(*f), (-12.0, 12.0), p.eq_bands[i], false);
        panel.append(&b);
        bands.push(fd);
    }
    let faders: Rc<(Fader, Vec<Fader>)> = Rc::new((pre, bands));

    // ----- Presets -----
    let preset_row = widgets::hbox(8);
    let dd_holder = widgets::hbox(0);
    preset_row.append(&dd_holder);
    let save = gtk::Button::with_label("Save as preset…");
    preset_row.append(&save);
    let delete_holder = widgets::hbox(0);
    preset_row.append(&delete_holder);
    g.add(&widgets::row("Preset", "Start from a shape, then fine-tune the faders.", Some(preset_row.upcast_ref())));
    g.add(&panel);
    g.note("Boosting bands makes loud songs clip; lower the preamp by about as much as your biggest boost.");

    // Programmatic fader moves shouldn't count as hand edits.
    let syncing = Rc::new(Cell::new(false));

    let rebuild_presets: Rc<dyn Fn()> = {
        let (dd_holder, delete_holder, faders, syncing) =
            (dd_holder.clone(), delete_holder.clone(), faders.clone(), syncing.clone());
        Rc::new(move || {
            let p = prefs::get();
            let presets = eq::all(&p.eq_custom);
            let current = eq::matching(&p.eq_custom, p.eq_preamp, &p.eq_bands).unwrap_or_else(|| "manual".into());
            let mut options: Vec<(String, String)> = presets.iter().map(|(id, pr)| (id.clone(), pr.name.clone())).collect();
            if current == "manual" {
                options.push(("manual".into(), "Custom".into()));
            }
            while let Some(c) = dd_holder.first_child() {
                dd_holder.remove(&c);
            }
            let dd = widgets::dropdown(&options, &current);
            let (faders, syncing) = (faders.clone(), syncing.clone());
            dd.connect_selected_notify(move |d| {
                let Some((id, _)) = options.get(d.selected() as usize) else { return };
                let Some((_, preset)) = eq::all(&prefs::get().eq_custom).into_iter().find(|(i, _)| i == id) else { return };
                prefs::update(|p| {
                    p.eq_preamp = preset.preamp;
                    p.eq_bands = preset.bands;
                    p.eq_preset = id.clone();
                });
                syncing.set(true);
                faders.0.scale.set_value(preset.preamp);
                for (f, v) in faders.1.iter().zip(preset.bands.iter()) {
                    f.scale.set_value(*v);
                }
                syncing.set(false);
                player::apply_eq();
            });
            dd_holder.append(&dd);
            while let Some(c) = delete_holder.first_child() {
                delete_holder.remove(&c);
            }
            if let Some(name) = current.strip_prefix("custom:") {
                let name = name.to_string();
                let del = widgets::two_click("Delete", "Click again to delete", move || {
                    prefs::update(|p| p.eq_custom.retain(|c| c.name != name));
                    window::toast("Preset deleted.");
                    if let Some(f) = REBUILD.with(|r| r.borrow().clone()) {
                        f();
                    }
                });
                delete_holder.append(&del);
            }
        })
    };
    REBUILD.with(|r| *r.borrow_mut() = Some(rebuild_presets.clone()));
    rebuild_presets();

    // Hand edits: apply live; the preset list follows after a moment.
    let pending: Rc<Cell<Option<gtk::glib::SourceId>>> = Rc::default();
    let on_fader = {
        let (faders, syncing, rebuild, pending) = (faders.clone(), syncing.clone(), rebuild_presets.clone(), pending.clone());
        Rc::new(move || {
            let pre = faders.0.scale.value();
            let mut b = [0.0; 10];
            for (i, f) in faders.1.iter().enumerate() {
                b[i] = f.scale.value();
                f.value.set_text(&fmt::db(b[i]));
            }
            faders.0.value.set_text(&fmt::db(pre));
            if syncing.get() {
                return;
            }
            prefs::update(|p| {
                p.eq_preamp = pre;
                p.eq_bands = b;
            });
            player::apply_eq();
            if let Some(id) = pending.take() {
                id.remove();
            }
            let (rebuild, pending2) = (rebuild.clone(), pending.clone());
            pending.set(Some(gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(450), move || {
                pending2.set(None);
                rebuild();
            })));
        })
    };
    let f = on_fader.clone();
    faders.0.scale.connect_value_changed(move |_| f());
    for fd in &faders.1 {
        let f = on_fader.clone();
        fd.scale.connect_value_changed(move |_| f());
    }

    save.connect_clicked(move |_| {
        let rebuild = rebuild_presets.clone();
        widgets::ask_text("Save preset", "Keep these fader settings under a name.", "", "Save", move |name| {
            let p = prefs::get();
            if eq::builtin().iter().any(|(_, b)| b.name.eq_ignore_ascii_case(&name)) {
                window::toast("A built-in preset already has that name.");
                return;
            }
            prefs::update(|pr| {
                pr.eq_custom.retain(|c| c.name != name);
                pr.eq_custom.push(EqPreset { name: name.clone(), preamp: p.eq_preamp, bands: p.eq_bands });
            });
            rebuild();
            window::toast(&format!("Saved preset “{name}”."));
        });
    });
}

thread_local! {
    static REBUILD: std::cell::RefCell<Option<Rc<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}
