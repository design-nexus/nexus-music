//! Playback state on the UI thread: the queue, the engine and what's playing.
//! Widgets subscribe to [`Event`]s instead of polling.

pub mod engine;
pub mod queue;

use crate::library::{Kind, Track, store};
use crate::{cmd, online, paths, prefs, window};
use engine::{Engine, Frame};
use gtk::glib;
use gtk::prelude::*;
use queue::{Queue, Repeat};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A different song is loaded (or none).
    Track,
    State,
    /// The regular position tick.
    Position,
    /// The position jumped (seek).
    Seeked,
    Queue,
    /// Volume, shuffle or repeat changed.
    Options,
}

struct Player {
    engine: Option<Engine>,
    error: Option<String>,
    queue: Queue,
    current: Option<Rc<Track>>,
    state: State,
    position: f64,
    duration: f64,
    heard: f64,
    counted: bool,
    failures: usize,
    pending_seek: Option<f64>,
    last_tick: Instant,
    /// Network buffering in progress (percent).
    buffering: Option<i32>,
    /// When an episode's position was last saved.
    saved_progress: f64,
}

type Callback = Rc<dyn Fn(Event)>;
type Listener = (glib::WeakRef<gtk::Widget>, Callback);

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static GLOBAL: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    static SAVE_PENDING: Cell<Option<glib::SourceId>> = const { Cell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Player) -> R) -> Option<R> {
    PLAYER.with(|p| p.borrow_mut().as_mut().map(f))
}

/// Call `f` on every event while `owner` is alive.
pub fn subscribe(owner: &impl IsA<gtk::Widget>, f: impl Fn(Event) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push((owner.upcast_ref::<gtk::Widget>().downgrade(), Rc::new(f))));
}

/// Call `f` on every event for the life of the app (MPRIS, notifications).
pub fn subscribe_global(f: impl Fn(Event) + 'static) {
    GLOBAL.with(|g| g.borrow_mut().push(Rc::new(f)));
}

fn emit(e: Event) {
    let fs: Vec<Callback> = LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(w, _)| w.upgrade().is_some());
        l.iter().map(|(_, f)| f.clone()).collect()
    });
    let globals: Vec<Callback> = GLOBAL.with(|g| g.borrow().clone());
    for f in fs.iter().chain(globals.iter()) {
        f(e);
    }
}

pub fn linear_volume(v: f64) -> f64 {
    v.clamp(0.0, 1.0).powi(3)
}

pub fn init() {
    let p = prefs::get();
    let (engine, error) = match Engine::new() {
        Ok(mut e) => {
            e.watch(on_message);
            e.set_volume(linear_volume(p.volume), p.muted);
            (Some(e), None)
        }
        Err(err) => (None, Some(err.to_string())),
    };
    let mut queue = Queue::default();
    queue.repeat = Repeat::from_id(&p.repeat);
    queue.set_shuffle(p.shuffle);
    PLAYER.with(|cell| {
        *cell.borrow_mut() = Some(Player {
            engine,
            error,
            queue,
            current: None,
            state: State::Stopped,
            position: 0.0,
            duration: 0.0,
            heard: 0.0,
            counted: false,
            failures: 0,
            pending_seek: None,
            last_tick: Instant::now(),
            buffering: None,
            saved_progress: 0.0,
        })
    });
    apply_eq();
    if p.resume {
        restore();
    }
    glib::timeout_add_local(std::time::Duration::from_millis(250), || {
        tick();
        glib::ControlFlow::Continue
    });
}

// ---------- Reading ----------

pub fn engine_error() -> Option<String> {
    with(|p| p.error.clone()).flatten()
}

pub fn has_spectrum() -> bool {
    with(|p| p.engine.as_ref().is_some_and(|e| e.has_spectrum)).unwrap_or(false)
}

pub fn has_eq() -> bool {
    with(|p| p.engine.as_ref().is_some_and(|e| e.has_eq())).unwrap_or(false)
}

pub fn current() -> Option<Rc<Track>> {
    with(|p| p.current.clone()).flatten()
}

pub fn state() -> State {
    with(|p| p.state).unwrap_or(State::Stopped)
}

pub fn position() -> f64 {
    with(|p| p.position).unwrap_or(0.0)
}

pub fn duration() -> f64 {
    with(|p| p.duration).unwrap_or(0.0)
}

/// A radio station is playing: no length, no seeking.
pub fn is_live() -> bool {
    current().is_some_and(|t| t.is_live())
}

/// Percent while a stream is filling its buffer.
pub fn buffering() -> Option<i32> {
    with(|p| p.buffering).flatten()
}

/// A station's logo arrived or its song changed: tell the views.
pub fn metadata_changed() {
    emit(Event::Track);
}

pub fn queue_items() -> (Vec<PathBuf>, Option<usize>) {
    with(|p| (p.queue.items.clone(), p.queue.cursor)).unwrap_or_default()
}

pub fn upcoming(n: usize) -> Vec<PathBuf> {
    with(|p| {
        let start = p.queue.cursor.map_or(0, |c| c + 1);
        p.queue.items.iter().skip(start).take(n).cloned().collect()
    })
    .unwrap_or_default()
}

pub fn shuffle() -> bool {
    with(|p| p.queue.shuffled()).unwrap_or(false)
}

pub fn repeat() -> Repeat {
    with(|p| p.queue.repeat).unwrap_or(Repeat::Off)
}

pub fn can_next() -> bool {
    with(|p| p.queue.cursor.is_some_and(|c| c + 1 < p.queue.items.len() || p.queue.repeat != Repeat::Off)).unwrap_or(false)
}

pub fn can_previous() -> bool {
    with(|p| p.current.is_some()).unwrap_or(false)
}

/// Run `f` on the spectrum frame that's audible right now.
pub fn with_frame<R>(f: impl FnOnce(&Frame) -> R) -> Option<R> {
    with(|p| p.engine.as_mut().and_then(|e| e.current_frame().map(f))).flatten()
}

// ---------- Engine events ----------

fn on_message(msg: &gst::Message) {
    use gst::MessageView;
    match msg.view() {
        MessageView::Element(e) => {
            if let Some(s) = e.structure()
                && s.name() == "spectrum"
            {
                with(|p| {
                    if let Some(engine) = p.engine.as_mut() {
                        engine.push_spectrum(s);
                    }
                });
            }
        }
        MessageView::Tag(t) => stream_title(&t.tags()),
        MessageView::Buffering(b) => buffering_changed(b.percent()),
        MessageView::Eos(_) => finished(),
        MessageView::Error(err) => {
            let name = current().map(|t| t.title.clone()).unwrap_or_default();
            if current().is_some_and(|t| t.is_remote()) {
                window::toast(&format!("Couldn't connect to “{name}”: {}", err.error()));
            } else {
                window::toast(&format!("Couldn't play “{name}”: {}", err.error()));
            }
            with(|p| p.buffering = None);
            let skip = with(|p| {
                p.failures += 1;
                p.failures < p.queue.items.len()
            })
            .unwrap_or(false);
            if skip && with(|p| p.queue.advance(true)).flatten().is_some() {
                load_current(true);
            } else {
                stop();
            }
        }
        MessageView::StreamStart(_) => {
            let switched = with(|p| p.engine.as_ref().is_some_and(|e| e.take_switched())).unwrap_or(false);
            if switched {
                // Gapless: the engine already moved on; catch the queue up.
                with(|p| {
                    p.queue.advance(false);
                    p.failures = 0;
                });
                adopt_current();
                emit(Event::Track);
                queue_changed();
            }
        }
        MessageView::AsyncDone(_) => {
            if let Some(secs) = with(|p| p.pending_seek.take()).flatten() {
                seek(secs);
            }
        }
        MessageView::DurationChanged(_) => {
            with(|p| {
                if let Some(d) = p.engine.as_ref().and_then(|e| e.duration()) {
                    p.duration = d;
                }
            });
        }
        _ => {}
    }
}

/// A station names the song it's playing (ICY "Artist - Title"): show it in
/// place of the station's own name, which moves to the album line.
fn stream_title(tags: &gst::TagList) {
    let Some(cur) = current().filter(|t| t.is_live()) else { return };
    // A station opened from a bare address names itself (icy-name).
    if let Some(name) = tags.get::<gst::tags::Organization>().map(|v| v.get().trim().to_string())
        && online::name_station(&cur.path, &name)
    {
        with(|p| p.current = Some(store::track_for(&cur.path)));
        emit(Event::Track);
    }
    let Some(text) = tags.get::<gst::tags::Title>().map(|v| v.get().trim().to_string()) else { return };
    let cur = current().unwrap_or(cur);
    let station = store::track_for(&cur.path);
    if text.is_empty() || text == station.title || text == cur.title {
        return;
    }
    let (artist, title) = match text.split_once(" - ") {
        Some((a, t)) if !a.trim().is_empty() && !t.trim().is_empty() => (a.trim().to_string(), t.trim().to_string()),
        _ => (station.title.clone(), text),
    };
    let shown = Track { title, artist, album: station.title.clone(), ..(*station).clone() };
    with(|p| p.current = Some(Rc::new(shown)));
    emit(Event::Track);
    notify_song();
}

/// Streams over the network pause while their buffer fills (live ones can't).
fn buffering_changed(percent: i32) {
    let live = is_live();
    let changed = with(|p| {
        let before = p.buffering;
        p.buffering = (percent < 100).then_some(percent);
        if !live && let Some(e) = p.engine.as_ref() {
            match (before.is_some(), p.buffering.is_some(), p.state) {
                (false, true, State::Playing) => e.pause(),
                (true, false, State::Playing) => e.play(),
                _ => {}
            }
        }
        before.is_some() != p.buffering.is_some()
    })
    .unwrap_or(false);
    if changed {
        emit(Event::State);
    }
}

/// The current song ended on its own.
fn finished() {
    if let Some(t) = current().filter(|t| t.kind == Kind::Episode) {
        online::set_progress(&t.path, 0.0, true);
    }
    if with(|p| p.queue.advance(false)).flatten().is_some() {
        load_current(true);
    } else {
        with(|p| {
            if let Some(e) = p.engine.as_mut() {
                e.stop();
            }
            p.state = State::Stopped;
            p.position = 0.0;
        });
        emit(Event::State);
        emit(Event::Seeked);
    }
}

/// Point `current` at the queue's song and reset the per-song counters.
fn adopt_current() {
    let path = with(|p| p.queue.current().cloned()).flatten();
    let track = path.map(|p| store::track_for(&p));
    with(|p| {
        p.duration = track.as_ref().map_or(0.0, |t| t.duration);
        p.current = track;
        p.position = 0.0;
        p.heard = 0.0;
        p.counted = false;
        p.buffering = None;
        p.saved_progress = 0.0;
    });
    apply_gain();
    schedule_save();
}

fn load_current(play: bool) {
    // A resume seek only applies to the song it was saved for.
    with(|p| p.pending_seek = None);
    adopt_current();
    let uri = current().map(|t| t.file_uri());
    with(|p| {
        let Some(engine) = p.engine.as_mut() else { return };
        match &uri {
            Some(u) => {
                engine.load(u);
                if play {
                    engine.play();
                    p.state = State::Playing;
                } else {
                    engine.pause();
                    p.state = State::Paused;
                }
            }
            None => {
                engine.stop();
                p.state = State::Stopped;
            }
        }
        p.last_tick = Instant::now();
    });
    if let Some(t) = current() {
        match t.kind {
            Kind::Episode => {
                // Pick up an episode where it was left.
                let pr = online::progress(&t.path);
                if !pr.played && pr.position > 5.0 {
                    with(|p| {
                        p.pending_seek = Some(pr.position);
                        p.position = pr.position;
                    });
                }
            }
            Kind::Station if play => online::radio::click(&online::info(&t.path).guid),
            _ => {}
        }
    }
    queue_changed();
    emit(Event::Track);
    emit(Event::State);
    emit(Event::Seeked);
    if play {
        notify_song();
    }
}

fn tick() {
    let mut count: Option<Rc<Track>> = None;
    let mut progress: Option<(PathBuf, f64, bool)> = None;
    let playing = with(|p| {
        let now = Instant::now();
        let dt = now.duration_since(p.last_tick).as_secs_f64();
        p.last_tick = now;
        if p.state != State::Playing {
            return false;
        }
        if let Some(pos) = p.engine.as_ref().and_then(|e| e.position()) {
            p.position = pos;
        }
        p.heard += dt;
        if let Some(t) = p.current.as_ref().filter(|t| t.kind == Kind::Episode)
            && p.pending_seek.is_none()
            && (p.position - p.saved_progress).abs() >= 10.0
        {
            p.saved_progress = p.position;
            let played = p.duration > 0.0 && p.position >= p.duration * 0.95;
            progress = Some((t.path.clone(), if played { 0.0 } else { p.position }, played));
        }
        if !p.counted && p.heard >= (p.duration * 0.5).clamp(10.0, 240.0) {
            p.counted = true;
            count = p.current.clone();
        }
        true
    })
    .unwrap_or(false);
    if let Some(t) = count {
        store::count_play(&t);
    }
    if let Some((path, pos, played)) = progress {
        online::set_progress(&path, pos, played);
    }
    if playing {
        emit(Event::Position);
    }
}

// ---------- Commands ----------

pub fn play_tracks(paths: Vec<PathBuf>, start: usize) {
    if paths.is_empty() {
        return;
    }
    online::keep(&paths);
    with(|p| {
        p.queue.set(paths, start);
        p.failures = 0;
    });
    load_current(true);
}

/// Shuffle a list into a fresh queue.
pub fn shuffle_tracks(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    set_shuffle(true);
    let start = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize)
        % paths.len();
    play_tracks(paths, start);
}

pub fn enqueue(paths: Vec<PathBuf>) {
    online::keep(&paths);
    with(|p| p.queue.append(&paths));
    queue_changed();
}

pub fn play_next(paths: Vec<PathBuf>) {
    online::keep(&paths);
    with(|p| p.queue.insert_next(&paths));
    queue_changed();
}

pub fn play() {
    match state() {
        State::Playing => {}
        State::Paused => {
            with(|p| {
                if let Some(e) = p.engine.as_ref() {
                    e.play();
                }
                p.state = State::Playing;
                p.last_tick = Instant::now();
            });
            emit(Event::State);
        }
        State::Stopped => {
            if with(|p| p.queue.current().is_some()).unwrap_or(false) {
                load_current(true);
            } else {
                // Nothing queued: play the whole library.
                let all: Vec<PathBuf> = store::tracks().iter().map(|t| t.path.clone()).collect();
                if shuffle() { shuffle_tracks(all) } else { play_tracks(all, 0) }
            }
        }
    }
}

pub fn pause() {
    if state() != State::Playing {
        return;
    }
    with(|p| {
        if let Some(e) = p.engine.as_ref() {
            e.pause();
        }
        p.state = State::Paused;
    });
    save_episode_progress();
    emit(Event::State);
    schedule_save();
}

fn save_episode_progress() {
    if let Some(t) = current().filter(|t| t.kind == Kind::Episode) {
        let (pos, dur) = (position(), duration());
        if pos > 5.0 && (dur <= 0.0 || pos < dur * 0.95) {
            online::set_progress(&t.path, pos, false);
        }
    }
}

pub fn toggle() {
    if state() == State::Playing { pause() } else { play() }
}

pub fn stop() {
    with(|p| {
        if let Some(e) = p.engine.as_mut() {
            e.stop();
        }
        p.state = State::Stopped;
        p.position = 0.0;
    });
    emit(Event::State);
    emit(Event::Seeked);
}

pub fn next() {
    let playing = state() != State::Paused;
    if with(|p| p.queue.advance(true)).flatten().is_some() {
        load_current(playing);
    }
}

pub fn previous() {
    if position() > 3.0 {
        seek(0.0);
        return;
    }
    let playing = state() != State::Paused;
    if with(|p| p.queue.previous()).flatten().is_some() {
        load_current(playing);
    }
}

pub fn seek(secs: f64) {
    if is_live() {
        return;
    }
    let secs = secs.clamp(0.0, duration().max(0.0));
    with(|p| {
        if let Some(e) = p.engine.as_mut() {
            e.seek(secs);
        }
        p.position = secs;
    });
    emit(Event::Seeked);
}

pub fn seek_by(delta: f64) {
    if current().is_some() {
        seek(position() + delta);
    }
}

pub fn jump(index: usize) {
    if with(|p| p.queue.jump(index)).flatten().is_some() {
        load_current(true);
    }
}

pub fn remove(index: usize) {
    let was_current = with(|p| p.queue.cursor == Some(index)).unwrap_or(false);
    with(|p| p.queue.remove(index));
    if was_current {
        if with(|p| p.queue.current().is_some()).unwrap_or(false) {
            load_current(state() == State::Playing);
        } else {
            stop();
            with(|p| p.current = None);
            emit(Event::Track);
        }
    }
    queue_changed();
}

/// Move several queue items together to just before `to`.
pub fn move_items(indexes: &[usize], to: usize) {
    with(|p| p.queue.move_items(indexes, to));
    queue_changed();
}

/// The queue as it is now, for undoing an edit.
pub struct Snapshot {
    items: Vec<PathBuf>,
    cursor: Option<usize>,
    original: Option<Vec<PathBuf>>,
    position: f64,
}

pub fn snapshot() -> Snapshot {
    let position = position();
    with(|p| Snapshot { items: p.queue.items.clone(), cursor: p.queue.cursor, original: p.queue.original().cloned(), position })
        .unwrap_or(Snapshot { items: Vec::new(), cursor: None, original: None, position: 0.0 })
}

/// Put back a queue from [`snapshot`]. If the current song changed, the old one
/// comes back paused where it was.
pub fn restore_snapshot(s: Snapshot) {
    let before = current().map(|t| t.path.clone());
    with(|p| p.queue.restore(s.items, s.cursor, s.original));
    let after = with(|p| p.queue.current().cloned()).flatten();
    if before != after && after.is_some() {
        load_current(false);
        if s.position > 1.0 && !is_live() {
            with(|p| {
                p.pending_seek = Some(s.position);
                p.position = s.position;
            });
            emit(Event::Seeked);
        }
    }
    queue_changed();
}

pub fn clear() {
    stop();
    with(|p| {
        p.queue.clear();
        p.current = None;
    });
    emit(Event::Track);
    queue_changed();
}

pub fn set_volume(v: f64) {
    let v = v.clamp(0.0, 1.0);
    prefs::update(|p| p.volume = v);
    let muted = prefs::get().muted;
    with(|p| {
        if let Some(e) = p.engine.as_ref() {
            e.set_volume(linear_volume(v), muted);
        }
    });
    emit(Event::Options);
}

pub fn set_muted(muted: bool) {
    prefs::update(|p| p.muted = muted);
    let v = prefs::get().volume;
    with(|p| {
        if let Some(e) = p.engine.as_ref() {
            e.set_volume(linear_volume(v), muted);
        }
    });
    emit(Event::Options);
}

pub fn set_shuffle(on: bool) {
    prefs::update(|p| p.shuffle = on);
    with(|p| p.queue.set_shuffle(on));
    queue_changed();
    emit(Event::Options);
}

pub fn set_repeat(r: Repeat) {
    prefs::update(|p| p.repeat = r.id().to_string());
    with(|p| p.queue.repeat = r);
    queue_changed();
    emit(Event::Options);
}

pub fn cycle_repeat() {
    set_repeat(match repeat() {
        Repeat::Off => Repeat::All,
        Repeat::All => Repeat::One,
        Repeat::One => Repeat::Off,
    });
}

/// Tell the engine what comes next (for gapless), save, and tell the views.
fn queue_changed() {
    let gapless = prefs::get().gapless;
    with(|p| {
        let next = if gapless { p.queue.peek_next().map(|n| store::track_for(n).file_uri()) } else { None };
        if let Some(e) = p.engine.as_ref() {
            e.set_next(next);
        }
    });
    schedule_save();
    emit(Event::Queue);
}

// ---------- Sound ----------

/// ReplayGain for a track as a linear factor, never louder than its peak allows.
pub fn replaygain_factor(t: &Track, mode: &str) -> f64 {
    let (gain, peak) = match mode {
        "track" => (t.rg_track_gain.or(t.rg_album_gain), t.rg_track_peak.or(t.rg_album_peak)),
        "album" => (t.rg_album_gain.or(t.rg_track_gain), t.rg_album_peak.or(t.rg_track_peak)),
        _ => return 1.0,
    };
    let Some(gain) = gain else { return 1.0 };
    let mut f = 10f64.powf(gain / 20.0);
    if let Some(peak) = peak.filter(|p| *p > 0.0) {
        f = f.min(1.0 / peak);
    }
    f
}

fn apply_gain() {
    let p = prefs::get();
    let rg = current().map_or(1.0, |t| replaygain_factor(&t, &p.replaygain));
    let preamp = if p.eq_enabled { 10f64.powf(p.eq_preamp / 20.0) } else { 1.0 };
    with(|pl| {
        if let Some(e) = pl.engine.as_ref() {
            e.set_gain(rg * preamp);
        }
    });
}

/// Push the equalizer and gain settings to the engine.
pub fn apply_eq() {
    let p = prefs::get();
    let bands = if p.eq_enabled { p.eq_bands } else { [0.0; 10] };
    with(|pl| {
        if let Some(e) = pl.engine.as_ref() {
            e.set_eq(&bands);
        }
    });
    apply_gain();
}

/// The library was (re)loaded: pick up the library's copy of the current song
/// (its id, cover and play count) if it was loaded from tags before.
pub fn library_changed() {
    let changed = with(|p| {
        // A station's song title isn't in the library; leave it be.
        let cur = p.current.as_ref().filter(|t| !t.is_live())?;
        let fresh = store::find(&cur.path)?;
        if Rc::ptr_eq(cur, &fresh) {
            return None;
        }
        p.current = Some(fresh);
        Some(())
    })
    .flatten()
    .is_some();
    if changed {
        emit(Event::Track);
    }
}

/// After the gapless setting changes.
pub fn refresh_next() {
    queue_changed();
}

// ---------- Notifications ----------

fn notify_song() {
    if !prefs::get().notify || window::is_active() {
        return;
    }
    let Some(t) = current() else { return };
    if !cmd::present("notify-send") {
        return;
    }
    let icon = if t.art.is_empty() {
        "audio-x-generic".to_string()
    } else {
        crate::library::art::file(&t.art, true).to_string_lossy().into_owned()
    };
    cmd::spawn(&[
        "notify-send",
        "--app-name=Music",
        &format!("--icon={icon}"),
        "--hint=string:x-canonical-private-synchronous:nexus-music",
        "--expire-time=4000",
        &t.title,
        &format!("{} — {}", t.artist, t.album),
    ]);
}

// ---------- Saved state ----------

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    items: Vec<PathBuf>,
    cursor: Option<usize>,
    original: Option<Vec<PathBuf>>,
    position: f64,
}

fn schedule_save() {
    if let Some(id) = SAVE_PENDING.with(|s| s.take()) {
        id.remove();
    }
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(1000), || {
        SAVE_PENDING.with(|s| s.set(None));
        save();
    });
    SAVE_PENDING.with(|s| s.set(Some(id)));
}

/// Write the queue and position now (also called on quit).
pub fn save() {
    save_episode_progress();
    let saved = with(|p| Saved {
        items: p.queue.items.clone(),
        cursor: p.queue.cursor,
        original: p.queue.original().cloned(),
        position: p.position,
    });
    if let Some(s) = saved
        && let Ok(text) = serde_json::to_string(&s)
    {
        let _ = cmd::atomic_write(&paths::state_file(), &text);
    }
}

fn restore() {
    let Some(saved) = std::fs::read_to_string(paths::state_file()).ok().and_then(|t| serde_json::from_str::<Saved>(&t).ok())
    else {
        return;
    };
    let shuffled = prefs::get().shuffle;
    with(|p| {
        p.queue.restore(saved.items, saved.cursor, if shuffled { saved.original } else { None });
        if shuffled && p.queue.original().is_none() {
            p.queue.set_shuffle(true);
        }
    });
    if with(|p| p.queue.current().is_some_and(|c| online::is_remote(c) || c.exists())).unwrap_or(false) {
        load_current(false);
        // A station picks up live; an episode resumes from its own progress.
        if saved.position > 1.0 && !current().is_some_and(|t| t.is_remote()) {
            with(|p| {
                p.pending_seek = Some(saved.position);
                p.position = saved.position;
            });
            emit(Event::Seeked);
        }
    } else {
        emit(Event::Queue);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaygain_respects_mode_and_peak() {
        let t = Track {
            rg_track_gain: Some(-6.0),
            rg_track_peak: Some(0.5),
            rg_album_gain: Some(6.0),
            rg_album_peak: Some(0.9),
            ..Default::default()
        };
        assert_eq!(replaygain_factor(&t, "off"), 1.0);
        assert!((replaygain_factor(&t, "track") - 0.501).abs() < 0.01);
        // +6 dB would be ×1.995, but the peak (0.9) caps it at ×1.11.
        assert!((replaygain_factor(&t, "album") - 1.0 / 0.9).abs() < 1e-9);
        assert_eq!(replaygain_factor(&Track::default(), "track"), 1.0);
    }

    #[test]
    fn volume_is_cubic() {
        assert_eq!(linear_volume(1.0), 1.0);
        assert!((linear_volume(0.5) - 0.125).abs() < 1e-9);
        assert_eq!(linear_volume(2.0), 1.0);
    }
}
