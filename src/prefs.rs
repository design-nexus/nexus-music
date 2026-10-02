//! This app's own preferences (`~/.config/nexus-music/settings.toml`).

use crate::{cmd, paths};
use gtk::glib;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    /// Follow the active Omarchy theme live.
    Omarchy,
    /// Use a bundled or custom theme.
    Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqPreset {
    pub name: String,
    pub preamp: f64,
    pub bands: [f64; 10],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: ThemeMode,
    pub theme: String,
    pub reduce_motion: bool,
    pub glow: bool,
    pub last_section: String,
    /// Folders the library is built from.
    pub library_folders: Vec<String>,
    /// Rescan when files are added, removed or renamed.
    pub watch: bool,
    pub gapless: bool,
    /// off, track or album.
    pub replaygain: String,
    /// Restore the queue and position on start.
    pub resume: bool,
    /// Desktop notification when the song changes while the window isn't focused.
    pub notify: bool,
    /// bars, line or mirror.
    pub spectrum_style: String,
    /// low, medium or high.
    pub spectrum_density: String,
    pub spectrum_peaks: bool,
    /// 0..1, cubic (what the slider shows).
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    /// off, all or one.
    pub repeat: String,
    pub eq_enabled: bool,
    /// The preset the faders came from ("" once they're edited by hand).
    pub eq_preset: String,
    pub eq_preamp: f64,
    pub eq_bands: [f64; 10],
    pub eq_custom: Vec<EqPreset>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Omarchy,
            theme: "tokyo-night".into(),
            reduce_motion: false,
            glow: true,
            last_section: "albums".into(),
            library_folders: vec![paths::music_dir().to_string_lossy().into_owned()],
            watch: true,
            gapless: true,
            replaygain: "track".into(),
            resume: true,
            notify: true,
            spectrum_style: "bars".into(),
            spectrum_density: "medium".into(),
            spectrum_peaks: true,
            volume: 0.8,
            muted: false,
            shuffle: false,
            repeat: "off".into(),
            eq_enabled: false,
            eq_preset: "flat".into(),
            eq_preamp: 0.0,
            eq_bands: [0.0; 10],
            eq_custom: Vec::new(),
        }
    }
}

thread_local! {
    static PREFS: RefCell<Prefs> = RefCell::new(load());
    static PENDING: Cell<Option<glib::SourceId>> = const { Cell::new(None) };
}

fn load() -> Prefs {
    std::fs::read_to_string(paths::prefs_file()).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
}

pub fn get() -> Prefs {
    PREFS.with(|p| p.borrow().clone())
}

/// Change the prefs now; the file is written about 450 ms after the last change,
/// so dragging a slider doesn't rewrite it on every tick.
pub fn update(change: impl FnOnce(&mut Prefs)) {
    PREFS.with(|p| change(&mut p.borrow_mut()));
    if let Some(id) = PENDING.with(|p| p.take()) {
        id.remove();
    }
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(450), || {
        PENDING.with(|p| p.set(None));
        save();
    });
    PENDING.with(|p| p.set(Some(id)));
}

/// Write any pending change now (on quit).
pub fn flush() {
    if let Some(id) = PENDING.with(|p| p.take()) {
        id.remove();
        save();
    }
}

fn save() {
    let text = PREFS.with(|p| toml::to_string_pretty(&*p.borrow()));
    if let Ok(text) = text {
        let _ = cmd::atomic_write(&paths::prefs_file(), &text);
    }
}
