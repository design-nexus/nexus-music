# Music — notes for working on this repo

- GTK4 (gtk4-rs 0.11) + Rust + GStreamer (gstreamer-rs 0.25). No libadwaita. Follows
  `~/Projects/STYLE.md`; theme, window, widgets and stylesheet started as copies of Tasks
  (`~/Projects/nexus-tasks`), the faders and segmented control from Settings. Every
  colour is a `@theme_*` token; the analyzer gets its colours from `theme::palette()`.
- Playback (`player/`): `engine.rs` wraps a `playbin` whose `audio-filter` is
  `audioconvert ! equalizer-10bands ! volume ! audioconvert ! spectrum`. The `volume`
  element carries ReplayGain × preamp; playbin's own volume is the user's (cubic).
  The bus is watched on the GTK main loop, so everything runs on the UI thread.
  Gapless: `about-to-finish` (streaming thread) swaps in `next_uri` and sets
  `switched`; the next STREAM_START advances the queue. `queue_changed()` must run
  after anything that changes what's next.
- Spectrum frames are queued with their running time and released by `current_frame()`
  only once the pipeline clock reaches them, so the bars match what's audible.
  `spectrum.rs` maps the bins to log bars (`map_bars`) and smooths them (`Smoother`);
  the tick callback stops itself when paused and idle.
- `player::mod` holds state; widgets subscribe with `player::subscribe(&widget, …)`
  (dropped with the widget) and the library with `store::subscribe`. Never hold a
  `with(...)` borrow while calling `emit`.
- Library: `scan.rs` runs on a thread, diffs (mtime, size) against SQLite and reads
  only what changed; files under a missing root are kept (unplugged drive). Covers are
  extracted once per album key into `~/.cache/nexus-music/covers/<fnv>{,-s}.jpg`.
  The UI keeps every track in memory (`store`), grouped into albums on load.
  `db.rs` opens a connection per call so it can run anywhere.
- Playlists store paths, not ids, so they survive rescans. Pages are `playlist:<id>`.
- Radio and podcasts (`online/`): a station or episode is a `Track` whose path is its
  `https://` address (`Kind::Station` / `Kind::Episode`), so the queue, playlists and
  `state.json` carry them unchanged; `store::track_for`/`find` hand such paths to
  `online`. Their details live in the `streams` table (plus `stations` for favourites
  and custom stations, `podcasts` for subscriptions) and in memory in `online::mod`.
  Items are only written to `streams` once queued, saved, played, downloaded or part of
  a subscription (`online::keep`, called by `player::play_tracks/enqueue/play_next` and
  the playlist actions). `radio.rs` is Radio Browser (a mirror is picked once per run),
  `podcast.rs` is Apple's charts/search plus RSS parsing (roxmltree), `http.rs` is
  blocking `ureq` run via `cmd::background`. Web pictures are cached like covers, keyed
  by `hash(url)` (`art::fetch`, `Cover::set_url`).
- Streams in the player: `MessageView::Tag` on a live station turns ICY "Artist - Title"
  into a copy of the current track (same path) and names bare stations from icy-name;
  `Buffering` pauses non-live streams until full. Episodes save their position every
  10 s, on pause and on quit, and resume through `pending_seek`.
- MPRIS (`mpris.rs`) runs zbus' blocking API on its own thread: it reads a shared
  snapshot, sends commands back over a channel, and emits PropertiesChanged when the
  UI thread asks. Bus name `org.mpris.MediaPlayer2.music`.
- Checks: `cargo clippy --all-targets -- -D warnings`, `cargo test`. Visual check:
  `MUSIC_SNAPSHOT=/tmp/x.png [MUSIC_SNAPSHOT_PLAY=1] music --section now-playing`
  (quit any running instance first, or run it under `dbus-run-session --` so a running
  one is left alone; it's single-instance). For tests, point
  `XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`XDG_CACHE_HOME` at scratch dirs with
  `library_folders` set, and use `MUSIC_AUDIO_SINK=fakesink` to play silently.
  `mode = "theme"`, `theme = "catppuccin-latte"` checks light mode.
