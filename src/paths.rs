//! Well-known locations. Every path honours the XDG overrides so the whole app
//! can be pointed at a scratch copy of `~/.config` for testing.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config"))
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".cache"))
        .join("nexus-music")
}

pub fn state_home() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/state"))
}

/// `~/.config/nexus-music`: everything this app owns lives here.
pub fn app_dir() -> PathBuf {
    config_home().join("nexus-music")
}

pub fn prefs_file() -> PathBuf {
    app_dir().join("settings.toml")
}

/// The play queue and position, so playback can resume.
pub fn state_file() -> PathBuf {
    data_dir().join("state.json")
}

pub fn custom_themes_dir() -> PathBuf {
    app_dir().join("themes")
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local/share"))
        .join("nexus-music")
}

/// The library database.
pub fn library_db() -> PathBuf {
    data_dir().join("library.db")
}

/// Downloaded podcast episodes.
pub fn podcasts_dir() -> PathBuf {
    data_dir().join("podcasts")
}

/// Album covers, extracted and scaled once.
pub fn covers_dir() -> PathBuf {
    cache_dir().join("covers")
}

/// The user's music folder (`XDG_MUSIC_DIR`), or `~/Music`.
pub fn music_dir() -> PathBuf {
    let dirs = config_home().join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(dirs) {
        for line in text.lines() {
            if let Some(v) = line.trim().strip_prefix("XDG_MUSIC_DIR=") {
                let v = v.trim_matches('"').replace("$HOME", &home().to_string_lossy());
                if !v.is_empty() {
                    return PathBuf::from(v);
                }
            }
        }
    }
    home().join("Music")
}

pub fn omarchy_theme_dir() -> PathBuf {
    state_home().join("omarchy/current/theme")
}

pub fn omarchy_colors() -> PathBuf {
    omarchy_theme_dir().join("colors.toml")
}

/// Replace `$HOME` with `~` for display.
pub fn pretty(path: &std::path::Path) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
