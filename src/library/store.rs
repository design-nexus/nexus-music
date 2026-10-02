//! The library as the UI sees it: every track in memory (on the UI thread),
//! grouped into albums, artists and genres, plus the playlists. Views subscribe
//! to changes and rebuild their models.

use super::db::{self, Playlist};
use super::scan::{self, Progress};
use super::{Track, art, tags};
use crate::{cmd, fmt, prefs, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug)]
pub struct Album {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub year: Option<i32>,
    pub art: String,
    /// In disc and track order.
    pub tracks: Vec<Rc<Track>>,
    pub duration: f64,
}

/// An artist or genre with what's filed under it.
#[derive(Debug)]
pub struct Bucket {
    pub name: String,
    pub albums: Vec<Rc<Album>>,
    pub tracks: usize,
    pub art: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Library,
    Playlists,
    Scan,
}

#[derive(Default)]
struct Lib {
    loaded: bool,
    tracks: Vec<Rc<Track>>,
    by_path: HashMap<PathBuf, Rc<Track>>,
    albums: Vec<Rc<Album>>,
    playlists: Vec<Playlist>,
    scanning: Option<(usize, usize)>,
    rescan_again: bool,
    folder_stamp: i64,
    error: Option<String>,
}

type Listener = (glib::WeakRef<gtk::Widget>, Box<dyn Fn(Change)>);

thread_local! {
    static LIB: RefCell<Lib> = RefCell::new(Lib::default());
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
}

/// Call `f` on every change while `owner` is alive.
pub fn subscribe(owner: &impl IsA<gtk::Widget>, f: impl Fn(Change) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push((owner.upcast_ref::<gtk::Widget>().downgrade(), Box::new(f))));
}

fn notify(change: Change) {
    // Take the list so listeners may subscribe or read the store freely.
    let listeners = LISTENERS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    let mut alive = Vec::with_capacity(listeners.len());
    for (w, f) in listeners {
        if w.upgrade().is_some() {
            f(change);
            alive.push((w, f));
        }
    }
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        alive.append(&mut l);
        *l = alive;
    });
}

pub fn loaded() -> bool {
    LIB.with(|l| l.borrow().loaded)
}

pub fn tracks() -> Vec<Rc<Track>> {
    LIB.with(|l| l.borrow().tracks.clone())
}

pub fn albums() -> Vec<Rc<Album>> {
    LIB.with(|l| l.borrow().albums.clone())
}

pub fn playlists() -> Vec<Playlist> {
    LIB.with(|l| l.borrow().playlists.clone())
}

pub fn scanning() -> Option<(usize, usize)> {
    LIB.with(|l| l.borrow().scanning)
}

pub fn error() -> Option<String> {
    LIB.with(|l| l.borrow().error.clone())
}

pub fn find(path: &Path) -> Option<Rc<Track>> {
    LIB.with(|l| l.borrow().by_path.get(path).cloned())
}

/// A track for any file: from the library, or read from its tags.
pub fn track_for(path: &Path) -> Rc<Track> {
    find(path).unwrap_or_else(|| {
        let mut t = tags::read(path);
        t.art = art::key_for(&t);
        if !art::file(&t.art, true).exists() {
            t.art = String::new();
        }
        Rc::new(t)
    })
}

pub fn album(key: &str) -> Option<Rc<Album>> {
    LIB.with(|l| l.borrow().albums.iter().find(|a| a.key == key).cloned())
}

pub fn album_of(t: &Track) -> Option<Rc<Album>> {
    album(&t.album_key())
}

fn bucket(albums: &[Rc<Album>], name_of: impl Fn(&Album) -> Vec<String>) -> Vec<Bucket> {
    let mut map: HashMap<String, Bucket> = HashMap::new();
    for a in albums {
        for name in name_of(a) {
            let b = map.entry(name.to_lowercase()).or_insert_with(|| Bucket {
                name: name.clone(),
                albums: vec![],
                tracks: 0,
                art: String::new(),
            });
            b.albums.push(a.clone());
            b.tracks += a.tracks.len();
            if b.art.is_empty() {
                b.art = a.art.clone();
            }
        }
    }
    let mut v: Vec<Bucket> = map.into_values().collect();
    v.sort_by_key(|b| sort_key(&b.name));
    v
}

pub fn artists() -> Vec<Bucket> {
    bucket(&albums(), |a| vec![a.artist.clone()])
}

pub fn genres() -> Vec<Bucket> {
    bucket(&albums(), |a| {
        let mut g: Vec<String> = a.tracks.iter().map(|t| t.genre.clone()).filter(|g| !g.is_empty()).collect();
        g.sort_by_key(|x| x.to_lowercase());
        g.dedup_by(|x, y| x.eq_ignore_ascii_case(y));
        if g.is_empty() { vec!["No genre".into()] } else { g }
    })
}

/// Case-insensitive, ignoring a leading "The ".
pub fn sort_key(s: &str) -> String {
    let l = s.trim().to_lowercase();
    l.strip_prefix("the ").map(str::to_string).unwrap_or(l)
}

fn group_albums(tracks: &[Rc<Track>]) -> Vec<Rc<Album>> {
    let mut map: HashMap<String, Vec<Rc<Track>>> = HashMap::new();
    for t in tracks {
        map.entry(t.album_key()).or_default().push(t.clone());
    }
    let mut albums: Vec<Rc<Album>> = map
        .into_iter()
        .map(|(key, mut ts)| {
            ts.sort_by(|a, b| {
                (a.disc_no.unwrap_or(1), a.track_no.unwrap_or(u32::MAX), &a.path).cmp(&(
                    b.disc_no.unwrap_or(1),
                    b.track_no.unwrap_or(u32::MAX),
                    &b.path,
                ))
            });
            let first = &ts[0];
            Rc::new(Album {
                key,
                title: first.album.clone(),
                artist: first.album_artist_or_artist().to_string(),
                year: ts.iter().filter_map(|t| t.year).min(),
                art: ts.iter().map(|t| t.art.clone()).find(|a| !a.is_empty()).unwrap_or_default(),
                duration: ts.iter().map(|t| t.duration).sum(),
                tracks: ts,
            })
        })
        .collect();
    albums.sort_by_key(|a| (sort_key(&a.artist), a.year.unwrap_or(0), sort_key(&a.title)));
    albums
}

/// Read the database into memory (off the main thread) and tell the views.
pub fn reload() {
    cmd::background(
        || -> anyhow::Result<(Vec<Track>, Vec<Playlist>)> {
            let conn = db::open()?;
            Ok((db::load_all(&conn)?, db::playlists(&conn)?))
        },
        |res| match res {
            Ok((tracks, playlists)) => {
                let tracks: Vec<Rc<Track>> = tracks.into_iter().map(Rc::new).collect();
                let by_path = tracks.iter().map(|t| (t.path.clone(), t.clone())).collect();
                let albums = group_albums(&tracks);
                LIB.with(|l| {
                    let mut l = l.borrow_mut();
                    l.loaded = true;
                    l.tracks = tracks;
                    l.by_path = by_path;
                    l.albums = albums;
                    l.playlists = playlists;
                    l.error = None;
                });
                notify(Change::Library);
                notify(Change::Playlists);
                crate::player::library_changed();
            }
            Err(e) => {
                LIB.with(|l| {
                    let mut l = l.borrow_mut();
                    l.loaded = true;
                    l.error = Some(e.to_string());
                });
                notify(Change::Library);
            }
        },
    );
}

pub fn roots() -> Vec<PathBuf> {
    prefs::get().library_folders.iter().map(PathBuf::from).collect()
}

/// Scan the library folders for new, changed and removed files.
pub fn rescan(announce: bool) {
    let busy = LIB.with(|l| {
        let mut l = l.borrow_mut();
        if l.scanning.is_some() {
            l.rescan_again = true;
            return true;
        }
        l.scanning = Some((0, 0));
        false
    });
    if busy {
        return;
    }
    notify(Change::Scan);
    let roots = roots();
    let (tx, rx) = async_channel::unbounded::<Progress>();
    std::thread::spawn(move || {
        scan::run(roots, |p| {
            let _ = tx.send_blocking(p);
        });
    });
    glib::spawn_future_local(async move {
        while let Ok(p) = rx.recv().await {
            match p {
                Progress::Reading { done, total } => {
                    LIB.with(|l| l.borrow_mut().scanning = Some((done, total)));
                    notify(Change::Scan);
                }
                Progress::Finished { added, updated, removed, stamp } => {
                    LIB.with(|l| l.borrow_mut().folder_stamp = stamp);
                    finish();
                    if added + updated + removed > 0 {
                        art::forget();
                        reload();
                    }
                    if announce {
                        window::toast(&summary(added, updated, removed));
                    }
                }
                Progress::Failed(e) => {
                    finish();
                    window::toast(&format!("Couldn't scan the library: {e}"));
                }
            }
        }
    });
}

fn finish() {
    let again = LIB.with(|l| {
        let mut l = l.borrow_mut();
        l.scanning = None;
        std::mem::take(&mut l.rescan_again)
    });
    notify(Change::Scan);
    if again {
        rescan(false);
    }
}

fn summary(added: usize, updated: usize, removed: usize) -> String {
    if added + updated + removed == 0 {
        return "The library is up to date.".into();
    }
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("{} added", fmt::count(added, "song", "songs")));
    }
    if updated > 0 {
        parts.push(format!("{} updated", fmt::thousands(updated)));
    }
    if removed > 0 {
        parts.push(format!("{} removed", fmt::thousands(removed)));
    }
    let mut s = parts.join(", ");
    s.push('.');
    s
}

/// Rescan shortly after files are added, removed or renamed (polled; folder
/// timestamps are cheap to read and more dependable than inotify on big trees).
pub fn start_watching() {
    glib::timeout_add_seconds_local(30, || {
        if !prefs::get().watch || scanning().is_some() {
            return glib::ControlFlow::Continue;
        }
        let last = LIB.with(|l| l.borrow().folder_stamp);
        cmd::background(
            || scan::folder_stamp(&roots()),
            move |now| {
                if now > last {
                    rescan(false);
                }
            },
        );
        glib::ControlFlow::Continue
    });
}

/// Count a play: the song was heard most of the way through.
pub fn count_play(t: &Track) {
    t.plays.set(t.plays.get() + 1);
    let path = t.path.clone();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    cmd::background(move || db::open().and_then(|c| db::count_play(&c, &path, now)), |_| {});
}

// ---------- Playlists ----------

fn playlists_changed(res: anyhow::Result<Vec<Playlist>>) {
    match res {
        Ok(p) => {
            LIB.with(|l| l.borrow_mut().playlists = p);
            notify(Change::Playlists);
        }
        Err(e) => window::toast(&format!("Couldn't save the playlist: {e}")),
    }
}

/// Run a playlist edit off the main thread, then refresh the playlist list.
pub fn edit_playlists(
    work: impl FnOnce(&rusqlite::Connection) -> anyhow::Result<()> + Send + 'static,
    done: impl FnOnce() + 'static,
) {
    cmd::background(
        move || {
            let conn = db::open()?;
            work(&conn)?;
            db::playlists(&conn)
        },
        move |res| {
            let ok = res.is_ok();
            playlists_changed(res);
            if ok {
                done();
            }
        },
    );
}

pub fn playlist_tracks(id: i64, done: impl FnOnce(Vec<Rc<Track>>) + 'static) {
    cmd::background(
        move || db::open().and_then(|c| db::playlist_paths(&c, id)).unwrap_or_default(),
        move |paths| done(paths.iter().map(|p| track_for(p)).collect()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(path: &str, album: &str, artist: &str, track: u32) -> Rc<Track> {
        Rc::new(Track {
            path: path.into(),
            album: album.into(),
            artist: artist.into(),
            track_no: Some(track),
            duration: 60.0,
            ..Default::default()
        })
    }

    #[test]
    fn groups_albums_in_track_order() {
        let albums = group_albums(&[t("/b", "X", "The Band", 2), t("/a", "X", "The Band", 1), t("/c", "Y", "Abba", 1)]);
        assert_eq!(albums.len(), 2);
        assert_eq!(albums[0].artist, "Abba");
        assert_eq!(albums[1].tracks[0].path, PathBuf::from("/a"));
        assert_eq!(albums[1].duration, 120.0);
    }

    #[test]
    fn sorts_ignoring_the() {
        assert_eq!(sort_key("The Beatles"), "beatles");
        assert_eq!(sort_key("Theatre"), "theatre");
    }

    #[test]
    fn summarises_scans() {
        assert_eq!(summary(0, 0, 0), "The library is up to date.");
        assert_eq!(summary(3, 0, 1), "3 songs added, 1 removed.");
    }
}
