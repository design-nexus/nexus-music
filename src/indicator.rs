//! The "now playing" mark in song tables: three small bars that move with
//! the music's bass, mids and treble, and rest while paused.

use crate::player::{self, State};
use crate::{prefs, spectrum};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const BARS: usize = 3;
/// Heights while paused, or with Reduce motion on.
const REST: [f64; BARS] = [0.45, 0.8, 0.6];

#[derive(Clone)]
pub struct Indicator {
    pub area: gtk::DrawingArea,
    levels: Rc<RefCell<[f64; BARS]>>,
    tick: Rc<Cell<Option<gtk::TickCallbackId>>>,
}

impl Indicator {
    pub fn new() -> Indicator {
        let area = gtk::DrawingArea::new();
        area.set_content_width(14);
        area.set_content_height(14);
        area.set_halign(gtk::Align::Center);
        area.set_valign(gtk::Align::Center);
        area.add_css_class("accent-text");
        let levels: Rc<RefCell<[f64; BARS]>> = Rc::new(RefCell::new(REST));
        let l = levels.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let c = a.color();
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
            let gap = 2.0;
            let bw = (w as f64 - gap * (BARS as f64 - 1.0)) / BARS as f64;
            for (i, v) in l.borrow().iter().enumerate() {
                let bh = (h as f64 * v.clamp(0.15, 1.0)).round();
                let x = i as f64 * (bw + gap);
                cr.rectangle(x, h as f64 - bh, bw, bh);
            }
            let _ = cr.fill();
        });
        Indicator { area, levels, tick: Rc::default() }
    }

    /// Show it (on the playing row) or hide it; it animates only while shown.
    pub fn set_on(&self, on: bool) {
        self.area.set_opacity(if on { 1.0 } else { 0.0 });
        if !on {
            if let Some(id) = self.tick.take() {
                id.remove();
            }
            return;
        }
        if let Some(id) = self.tick.take() {
            self.tick.set(Some(id));
            return;
        }
        let levels = self.levels.clone();
        let id = self.area.add_tick_callback(move |a, _| {
            let next = if player::state() == State::Playing && !prefs::get().reduce_motion {
                player::with_frame(|f| spectrum::map_bars(&f.magnitudes, f.rate, BARS)).map(|v| {
                    let mut out = [0.0; BARS];
                    for (o, x) in out.iter_mut().zip(v) {
                        *o = x as f64;
                    }
                    out
                })
            } else {
                Some(REST)
            };
            if let Some(next) = next {
                let mut l = levels.borrow_mut();
                // Rise at once, fall gently.
                for (cur, n) in l.iter_mut().zip(next) {
                    *cur = if n > *cur { n } else { *cur * 0.85 + n * 0.15 };
                }
                drop(l);
                a.queue_draw();
            }
            gtk::glib::ControlFlow::Continue
        });
        self.tick.set(Some(id));
    }
}
