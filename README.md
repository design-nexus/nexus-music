# Music

A music player for [Omarchy](https://omarchy.org). It plays your music library and
shows it as a live spectrum. It takes its colours from your Omarchy theme and fits a
half-screen tile.

## What it does

- **Library:** scans your music folders (MP3, FLAC, Ogg Vorbis, Opus, AAC/M4A, WAV,
  AIFF, APE, WavPack, Musepack and more) into a local database. Later scans only read
  new or changed files, and it picks up music added, removed or renamed within a minute.
  - **Songs:** one sortable table of everything.
  - **Albums:** a cover grid. Covers come from the files' embedded art or a
    `cover.jpg` / `folder.jpg` beside them.
  - **Artists** and **Genres**, each opening into their albums.
  - **Folders:** your music as it's laid out on disk.
  - **Search** (<kbd>Ctrl</kbd>+<kbd>F</kbd>) across songs, albums and artists.
- **Radio:** about 58,000 stations from the [Radio Browser](https://www.radio-browser.info)
  directory, no account needed.
  - Browse the most popular stations, or by genre or country, or search by name.
  - Star stations to keep them as favourites, grouped by genre or by country.
  - Add your own stations by their stream address or a `.pls` / `.m3u` link.
  - Shows the song a station is playing, when the station sends it.
- **Podcasts:** Apple's top shows, its 19 categories and search, with no account or key.
  - Subscribe from the directory or by a show's RSS feed address. New episodes are
    checked for when Music opens.
  - Episodes pick up where you left off, and show how much is left.
  - Download episodes to play offline.
- Stations and episodes go in the queue and in playlists alongside your songs, and
  pick up again after a restart.
- **Now playing:** the cover and song, a full-width spectrum analyzer and what's up
  next.
  - The analyzer draws 40 Hz–16 kHz on a log scale, in sync with what you hear.
  - Styles: bars, line or mirrored, with peak markers.
  - A small version sits in the player bar on every page.
- **Playback:** gapless, shuffle, repeat (all or one song), and ReplayGain volume
  levelling by song or album. It remembers the queue and position between sessions.
- **Queue and playlists:**
  - Play next or add to the queue from any song's right-click menu.
  - Make playlists from songs or from the queue, or drag songs onto a playlist in the
    sidebar.
  - Import and export M3U.
- **Equalizer:** ten bands and a preamp, with presets and your own saved ones.
- **Media keys and the bar:** MPRIS, so media keys, `playerctl` and status bars control
  it and show the song and cover.
- **Notifications** when the song changes while the window is in the background.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/design-nexus/nexus-music/main/install.sh | bash
```

This installs GTK 4 and GStreamer if they're missing, builds with Cargo, and installs
`music` to `~/.local/bin` along with a launcher entry. Add `-s -- --default` after
`bash` to also make it the default app for audio files.

To remove it, run the same line with `uninstall.sh` in place of `install.sh`. Add
`-s -- --purge` to also remove its settings, library database, playlists and downloaded
episodes. Your music files are never touched.

## Usage

```
music [OPTIONS] [FILES…]
```

| Option | What |
| --- | --- |
| `FILES…` | Play these songs, folders or `.m3u` playlists, or a stream's web address |
| `--enqueue` | Add `FILES` to the queue instead |
| `--section ID` | Open a page: `now-playing`, `songs`, `albums`, `artists`, `genres`, `folders`, `radio`, `podcasts`, `queue`, `equalizer`, `settings` |
| `--toggle` | Close the window if it's open, otherwise open it |
| `--play-pause`, `--play`, `--pause`, `--stop`, `--next`, `--previous` | Control playback without raising the window |

| Key | What |
| --- | --- |
| <kbd>Space</kbd> | Play or pause |
| <kbd>Ctrl</kbd>+<kbd>←</kbd> / <kbd>→</kbd> | Previous / next song |
| <kbd>←</kbd> / <kbd>→</kbd> | Back / forward 5 seconds |
| <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search the library |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Close |

## Files

| Path | What |
| --- | --- |
| `~/.config/nexus-music/settings.toml` | Preferences: library folders, playback, equalizer, theme |
| `~/.config/nexus-music/themes/*.toml` | Your own themes |
| `~/.local/share/nexus-music/library.db` | The library, play counts, playlists, favourite stations and podcast subscriptions |
| `~/.local/share/nexus-music/podcasts/` | Downloaded episodes |
| `~/.local/share/nexus-music/state.json` | The queue and position, for resuming |
| `~/.cache/nexus-music/covers/` | Album covers, station logos and podcast art, scaled |

## License

MIT
