//! The spectrum analyzer: GStreamer's FFT magnitudes mapped onto log-spaced
//! bars, smoothed (instant attack, eased fall, peak caps that hold then drop)
//! and drawn with cairo in the theme's colours.

use crate::player::{self, State};
use crate::prefs;
use crate::theme::{self, Rgb};
use gtk::cairo;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const MIN_HZ: f64 = 40.0;
pub const MAX_HZ: f64 = 16000.0;
/// dB that maps to an empty bar, and to a full one.
const FLOOR_DB: f64 = -72.0;
const CEIL_DB: f64 = -12.0;
/// Music has less energy up high; tilt by this much per octave around 1 kHz so
/// the treble isn't flat on the floor.
const TILT_DB_PER_OCTAVE: f64 = 3.0;
const PEAK_HOLD: f64 = 0.4;
const SILENCE_DB: f64 = crate::player::engine::THRESHOLD_DB as f64 + 1.0;

/// Bar levels (0..1) for `n` log-spaced bars from one frame of magnitudes (dB).
pub fn map_bars(mags: &[f32], rate: u32, n: usize) -> Vec<f32> {
    if mags.is_empty() || n == 0 {
        return vec![0.0; n];
    }
    let nyquist = rate.max(8000) as f64 / 2.0;
    let bin_hz = nyquist / mags.len() as f64;
    let top = MAX_HZ.min(nyquist);
    let ratio = (top / MIN_HZ).ln();
    let edge = |k: usize| MIN_HZ * (ratio * k as f64 / n as f64).exp();
    let at = |hz: f64| -> f64 {
        // Linear interpolation between bin centres.
        let x = (hz / bin_hz - 0.5).max(0.0);
        let i = (x.floor() as usize).min(mags.len() - 1);
        let j = (i + 1).min(mags.len() - 1);
        let f = x - i as f64;
        mags[i] as f64 * (1.0 - f) + mags[j] as f64 * f
    };
    (0..n)
        .map(|k| {
            let (lo, hi) = (edge(k), edge(k + 1));
            let first = ((lo / bin_hz - 0.5).ceil().max(0.0)) as usize;
            let last = ((hi / bin_hz - 0.5).floor().max(0.0)) as usize;
            let db = if first <= last && first < mags.len() {
                mags[first..=last.min(mags.len() - 1)].iter().fold(f64::MIN, |m, v| m.max(*v as f64))
            } else {
                at((lo * hi).sqrt())
            };
            // At the analyzer's threshold there's nothing there; don't let the tilt lift it.
            if db <= SILENCE_DB {
                return 0.0;
            }
            let centre = (lo * hi).sqrt();
            let db = db + TILT_DB_PER_OCTAVE * (centre / 1000.0).log2();
            (((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0)) as f32
        })
        .collect()
}

/// Where a frequency sits across the plot, 0..1.
pub fn x_of(hz: f64) -> f64 {
    ((hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()).clamp(0.0, 1.0)
}

#[derive(Default)]
pub struct Smoother {
    pub levels: Vec<f32>,
    pub peaks: Vec<f32>,
    hold: Vec<f64>,
}

impl Smoother {
    pub fn resize(&mut self, n: usize) {
        if self.levels.len() != n {
            self.levels = vec![0.0; n];
            self.peaks = vec![0.0; n];
            self.hold = vec![0.0; n];
        }
    }

    /// Advance by `dt` seconds toward `targets`. Without motion, levels jump
    /// straight to the targets and peaks follow them.
    pub fn step(&mut self, targets: &[f32], dt: f64, motion: bool) {
        self.resize(targets.len());
        for (i, &t) in targets.iter().enumerate() {
            let l = &mut self.levels[i];
            if !motion {
                *l = t;
                self.peaks[i] = t;
                continue;
            }
            *l = if t >= *l { t } else { t.max(*l - (dt * (0.6 + 2.6 * *l as f64)) as f32) };
            if *l >= self.peaks[i] {
                self.peaks[i] = *l;
                self.hold[i] = PEAK_HOLD;
            } else if self.hold[i] > 0.0 {
                self.hold[i] -= dt;
            } else {
                self.peaks[i] = (self.peaks[i] - (dt * 0.8) as f32).max(*l);
            }
        }
    }

    pub fn idle(&self) -> bool {
        self.levels.iter().chain(self.peaks.iter()).all(|v| *v < 0.002)
    }
}

struct Analyzer {
    area: gtk::DrawingArea,
    compact: bool,
    smooth: RefCell<Smoother>,
    ticking: Cell<bool>,
    last_us: Cell<i64>,
    last_draw_us: Cell<i64>,
}

fn bar_geometry(width: f64, compact: bool) -> (usize, f64, f64) {
    let (bw, gap) = if compact {
        (3.0, 2.0)
    } else {
        match prefs::get().spectrum_density.as_str() {
            "low" => (12.0, 4.0),
            "high" => (3.0, 1.5),
            _ => (6.0, 2.0),
        }
    };
    let n = (((width + gap) / (bw + gap)).floor() as usize).clamp(if compact { 8 } else { 16 }, 160);
    let bw = ((width - gap * (n as f64 - 1.0)) / n as f64).max(1.0);
    (n, bw, gap)
}

thread_local! {
    static ALL: RefCell<Vec<gtk::glib::WeakRef<gtk::DrawingArea>>> = const { RefCell::new(Vec::new()) };
}

/// Redraw every analyzer (after a style setting changed).
pub fn redraw_all() {
    ALL.with(|a| {
        a.borrow_mut().retain(|w| w.upgrade().is_some());
        for w in a.borrow().iter() {
            if let Some(d) = w.upgrade() {
                d.queue_draw();
            }
        }
    });
}

const PAD_X: f64 = 12.0;
const PAD_TOP: f64 = 14.0;
const PAD_BOTTOM: f64 = 26.0;

impl Analyzer {
    fn plot(&self) -> (f64, f64, f64, f64) {
        let (w, h) = (self.area.width() as f64, self.area.height() as f64);
        if self.compact {
            (0.0, 0.0, w, h)
        } else {
            (PAD_X, PAD_TOP, (w - 2.0 * PAD_X).max(1.0), (h - PAD_TOP - PAD_BOTTOM).max(1.0))
        }
    }

    fn start(self: &Rc<Self>) {
        if self.ticking.get() || !self.area.is_mapped() {
            return;
        }
        self.ticking.set(true);
        self.last_us.set(0);
        let me = self.clone();
        self.area.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let reduce = prefs::get().reduce_motion;
            // Reduce motion: no easing, and at most 10 redraws a second.
            if reduce && now - me.last_draw_us.get() < 100_000 {
                return glib::ControlFlow::Continue;
            }
            let dt = if me.last_us.get() == 0 { 1.0 / 60.0 } else { ((now - me.last_us.get()) as f64 / 1e6).min(0.1) };
            me.last_us.set(now);
            me.last_draw_us.set(now);
            let (_, _, pw, _) = me.plot();
            let (n, _, _) = bar_geometry(pw, me.compact);
            let playing = player::state() == State::Playing;
            let targets = if playing {
                player::with_frame(|f| map_bars(&f.magnitudes, f.rate, n)).unwrap_or_else(|| me.smooth.borrow().levels.clone())
            } else {
                vec![0.0; n]
            };
            let targets = if targets.len() == n { targets } else { vec![0.0; n] };
            me.smooth.borrow_mut().step(&targets, dt, !reduce);
            me.area.queue_draw();
            if !playing && me.smooth.borrow().idle() || !me.area.is_mapped() {
                me.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn draw(&self, cr: &cairo::Context) {
        let p = theme::palette();
        let accent = Rgb::hex(&p.accent);
        let hot = accent.mix(Rgb::hex(&p.danger), 0.6);
        let text = Rgb::hex(&p.text);
        let (x0, y0, pw, ph) = self.plot();
        let style = if self.compact { "bars".to_string() } else { prefs::get().spectrum_style };
        let (n, bw, gap) = bar_geometry(pw, self.compact);
        let mut smooth = self.smooth.borrow_mut();
        smooth.resize(n);

        if !self.compact {
            self.draw_axes(cr, &p, x0, y0, pw, ph, style == "mirror");
        }

        let mid = y0 + ph / 2.0;
        let gradient = if style == "mirror" {
            let g = cairo::LinearGradient::new(0.0, y0, 0.0, y0 + ph);
            g.add_color_stop_rgb(0.0, hot.0, hot.1, hot.2);
            g.add_color_stop_rgb(0.5, accent.0, accent.1, accent.2);
            g.add_color_stop_rgb(1.0, hot.0, hot.1, hot.2);
            g
        } else {
            let g = cairo::LinearGradient::new(0.0, y0 + ph, 0.0, y0);
            g.add_color_stop_rgb(0.0, accent.0, accent.1, accent.2);
            g.add_color_stop_rgb(1.0, hot.0, hot.1, hot.2);
            g
        };

        // The shape: bars, a mirrored pair, or a line through the bar tops.
        cr.new_path();
        if style == "line" {
            cr.move_to(x0, y0 + ph);
            for (i, l) in smooth.levels.iter().enumerate() {
                let x = x0 + i as f64 * (bw + gap) + bw / 2.0;
                cr.line_to(x, y0 + ph - *l as f64 * ph);
            }
            cr.line_to(x0 + pw, y0 + ph);
            cr.close_path();
            let fill = cairo::LinearGradient::new(0.0, y0, 0.0, y0 + ph);
            fill.add_color_stop_rgba(0.0, accent.0, accent.1, accent.2, 0.24);
            fill.add_color_stop_rgba(1.0, accent.0, accent.1, accent.2, 0.04);
            let _ = cr.set_source(&fill);
            let _ = cr.fill();
            cr.new_path();
            for (i, l) in smooth.levels.iter().enumerate() {
                let x = x0 + i as f64 * (bw + gap) + bw / 2.0;
                let y = y0 + ph - *l as f64 * ph;
                if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
            }
            cr.set_line_join(cairo::LineJoin::Round);
            cr.set_line_cap(cairo::LineCap::Round);
            if prefs::get().glow {
                let glow = Rgb::hex(&p.glow);
                cr.set_source_rgba(glow.0, glow.1, glow.2, 0.18);
                cr.set_line_width(7.0);
                let _ = cr.stroke_preserve();
            }
            let _ = cr.set_source(&gradient);
            cr.set_line_width(2.5);
            let _ = cr.stroke();
        } else {
            let radius = (bw / 2.0).min(2.0);
            for (i, l) in smooth.levels.iter().enumerate() {
                let x = x0 + i as f64 * (bw + gap);
                if *l < 0.01 {
                    continue;
                }
                let h = (*l as f64 * ph).max(2.0);
                if style == "mirror" {
                    let h = h.min(ph);
                    rounded_rect(cr, x, mid - h / 2.0, bw, h, radius);
                } else {
                    rounded_rect(cr, x, y0 + ph - h, bw, h, radius);
                }
            }
            if prefs::get().glow && !self.compact {
                let glow = Rgb::hex(&p.glow);
                cr.set_source_rgba(glow.0, glow.1, glow.2, 0.12);
                cr.set_line_width(5.0);
                let _ = cr.stroke_preserve();
            }
            let _ = cr.set_source(&gradient);
            let _ = cr.fill();
        }

        // Peak caps.
        if !self.compact && prefs::get().spectrum_peaks && !prefs::get().reduce_motion {
            cr.set_source_rgba(text.0, text.1, text.2, 0.85);
            for (i, pk) in smooth.peaks.iter().enumerate() {
                if *pk < 0.01 {
                    continue;
                }
                let x = x0 + i as f64 * (bw + gap);
                let h = *pk as f64 * ph;
                match style.as_str() {
                    "mirror" => {
                        cr.rectangle(x, mid - h / 2.0 - 3.0, bw, 2.0);
                        cr.rectangle(x, mid + h / 2.0 + 1.0, bw, 2.0);
                    }
                    "line" => {
                        cr.arc(x + bw / 2.0, y0 + ph - h, 1.6, 0.0, std::f64::consts::TAU);
                        cr.close_path();
                    }
                    _ => cr.rectangle(x, y0 + ph - h - 3.0, bw, 2.0),
                }
            }
            let _ = cr.fill();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_axes(&self, cr: &cairo::Context, p: &theme::Palette, x0: f64, y0: f64, pw: f64, ph: f64, mirror: bool) {
        let border = Rgb::hex(&p.border);
        let dim = Rgb::hex(&p.dim_text);
        cr.set_line_width(1.0);
        cr.set_source_rgba(border.0, border.1, border.2, 0.4);
        let lines: &[f64] = if mirror { &[0.25, 0.5, 0.75] } else { &[0.25, 0.5, 0.75, 1.0] };
        for f in lines {
            let y = (y0 + ph * (1.0 - f)).round() + 0.5;
            cr.move_to(x0, y);
            cr.line_to(x0 + pw, y);
        }
        let _ = cr.stroke();
        cr.select_font_face("JetBrains Mono", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        cr.set_font_size(11.0);
        cr.set_source_rgb(dim.0, dim.1, dim.2);
        let mut last_right = f64::MIN;
        for hz in [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0] {
            let label = crate::fmt::hz(hz);
            let Ok(ext) = cr.text_extents(&label) else { continue };
            let x = x0 + x_of(hz) * pw - ext.width() / 2.0;
            if x < last_right + 8.0 || x + ext.width() > x0 + pw {
                continue;
            }
            cr.move_to(x, y0 + ph + 17.0);
            let _ = cr.show_text(&label);
            last_right = x + ext.width();
        }
    }
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(h / 2.0).min(w / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.close_path();
}

/// A spectrum analyzer. `compact` is the small strip in the player bar: bars
/// only, no axes or panel.
pub fn analyzer(compact: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class(if compact { "spectrum-strip" } else { "spectrum" });
    area.set_hexpand(true);
    let an = Rc::new(Analyzer {
        area: area.clone(),
        compact,
        smooth: RefCell::new(Smoother::default()),
        ticking: Cell::new(false),
        last_us: Cell::new(0),
        last_draw_us: Cell::new(0),
    });
    let a = an.clone();
    area.set_draw_func(move |_, cr, _, _| a.draw(cr));
    let a = an.clone();
    area.connect_map(move |_| {
        if player::state() == State::Playing {
            a.start();
        } else {
            a.area.queue_draw();
        }
    });
    let a = Rc::downgrade(&an);
    player::subscribe(&area, move |e| {
        if let (Some(a), player::Event::State | player::Event::Track | player::Event::Seeked) = (a.upgrade(), e) {
            a.start();
        }
    });
    area.connect_resize(|a, _, _| a.queue_draw());
    ALL.with(|a| a.borrow_mut().push(area.downgrade()));
    area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_tone_to_its_bar() {
        // A loud tone at ~1 kHz in an otherwise silent spectrum.
        let rate = 44100;
        let bands = 1024;
        let bin_hz = rate as f64 / 2.0 / bands as f64;
        let mut mags = vec![-80.0f32; bands];
        let i = (1000.0 / bin_hz) as usize;
        mags[i] = -15.0;
        let n = 32;
        let bars = map_bars(&mags, rate, n);
        let loudest = bars.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let expect = (x_of(1000.0) * n as f64) as usize;
        assert!((loudest as i64 - expect as i64).abs() <= 1, "bar {loudest}, expected {expect}");
        assert!(bars.iter().filter(|v| **v > 0.0).count() <= 2);
    }

    #[test]
    fn low_bars_interpolate_instead_of_going_empty() {
        let bars = map_bars(&vec![-30.0f32; 1024], 44100, 96);
        assert!(bars.iter().all(|v| *v > 0.0));
    }

    #[test]
    fn silence_is_empty() {
        assert!(map_bars(&vec![-80.0f32; 1024], 48000, 40).iter().all(|v| *v == 0.0));
        assert_eq!(map_bars(&[], 48000, 5), vec![0.0; 5]);
    }

    #[test]
    fn smoother_attacks_instantly_and_falls_gently() {
        let mut s = Smoother::default();
        s.step(&[1.0], 1.0 / 60.0, true);
        assert_eq!(s.levels[0], 1.0);
        s.step(&[0.0], 1.0 / 60.0, true);
        assert!(s.levels[0] > 0.9 && s.levels[0] < 1.0);
        // The peak holds, then drops.
        assert_eq!(s.peaks[0], 1.0);
        for _ in 0..20 {
            s.step(&[0.0], 1.0 / 60.0, true);
        }
        assert_eq!(s.peaks[0], 1.0);
        assert!(s.levels[0] < 0.5);
        for _ in 0..10 {
            s.step(&[0.0], 1.0 / 60.0, true);
        }
        assert!(s.peaks[0] < 1.0 && s.peaks[0] > s.levels[0]);
        for _ in 0..120 {
            s.step(&[0.0], 1.0 / 60.0, true);
        }
        assert!(s.idle());
    }

    #[test]
    fn without_motion_levels_jump() {
        let mut s = Smoother::default();
        s.step(&[0.7, 0.2], 0.1, false);
        s.step(&[0.1, 0.0], 0.1, false);
        assert_eq!(s.levels, vec![0.1, 0.0]);
        assert_eq!(s.peaks, vec![0.1, 0.0]);
    }
}
