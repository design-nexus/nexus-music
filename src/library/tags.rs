//! Reading tags with lofty, with sensible fallbacks for untagged files.

use super::Track;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::tag::Tag;
use std::path::Path;

pub const UNKNOWN_ARTIST: &str = "Unknown artist";
pub const UNKNOWN_ALBUM: &str = "Unknown album";

/// Read one file. Untagged or unreadable files still produce a track named
/// after the file, so nothing in the music folder goes missing.
pub fn read(path: &Path) -> Track {
    let mut t = Track { path: path.to_path_buf(), ..Default::default() };
    if let Ok(meta) = std::fs::metadata(path) {
        t.size = meta.len() as i64;
        t.mtime =
            meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
    }
    if let Ok(file) = lofty::read_from_path(path) {
        let props = file.properties();
        t.duration = props.duration().as_secs_f64();
        t.bitrate = props.audio_bitrate().unwrap_or(0);
        t.sample_rate = props.sample_rate().unwrap_or(0);
        t.bit_depth = props.bit_depth().unwrap_or(0);
        if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
            fill(&mut t, tag);
        }
    }
    fallbacks(&mut t);
    t
}

fn fill(t: &mut Track, tag: &Tag) {
    let s = |v: Option<std::borrow::Cow<'_, str>>| v.map(|v| v.trim().to_string()).unwrap_or_default();
    t.title = s(tag.title());
    t.artist = s(tag.artist());
    t.album = s(tag.album());
    t.genre = s(tag.genre());
    t.album_artist = tag.get_string(ItemKey::AlbumArtist).map(|v| v.trim().to_string()).unwrap_or_default();
    t.year = tag.date().map(|d| d.year as i32).filter(|y| *y > 0);
    t.track_no = tag.track();
    t.disc_no = tag.disk();
    t.rg_track_gain = tag.get_string(ItemKey::ReplayGainTrackGain).and_then(parse_gain);
    t.rg_track_peak = tag.get_string(ItemKey::ReplayGainTrackPeak).and_then(parse_gain);
    t.rg_album_gain = tag.get_string(ItemKey::ReplayGainAlbumGain).and_then(parse_gain);
    t.rg_album_peak = tag.get_string(ItemKey::ReplayGainAlbumPeak).and_then(parse_gain);
}

/// "-6.54 dB" → -6.54
pub fn parse_gain(v: &str) -> Option<f64> {
    let v = v.trim();
    let num = v.strip_suffix("dB").or_else(|| v.strip_suffix("db")).unwrap_or(v).trim();
    num.parse::<f64>().ok().filter(|g| g.is_finite())
}

/// File name for the title, the folder for the album, "Unknown artist".
pub fn fallbacks(t: &mut Track) {
    if t.title.is_empty() {
        t.title = t.path.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_default();
    }
    if t.artist.is_empty() {
        t.artist = if t.album_artist.is_empty() { UNKNOWN_ARTIST.into() } else { t.album_artist.clone() };
    }
    if t.album.is_empty() {
        t.album = t
            .path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| UNKNOWN_ALBUM.into());
    }
}

/// The front cover embedded in a file, or any picture if there's no front.
pub fn embedded_picture(path: &Path) -> Option<Vec<u8>> {
    let file = lofty::read_from_path(path).ok()?;
    for tag in file.tags() {
        let pics = tag.pictures();
        if let Some(p) = pics.iter().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| pics.first()) {
            return Some(p.data().to_vec());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_gains() {
        assert_eq!(parse_gain("-6.54 dB"), Some(-6.54));
        assert_eq!(parse_gain("+1.20 dB"), Some(1.2));
        assert_eq!(parse_gain("0.988"), Some(0.988));
        assert_eq!(parse_gain("loud"), None);
    }

    #[test]
    fn falls_back_to_file_and_folder_names() {
        let mut t = Track { path: PathBuf::from("/music/Some Album/01_First_Song.flac"), ..Default::default() };
        fallbacks(&mut t);
        assert_eq!(t.title, "01 First Song");
        assert_eq!(t.album, "Some Album");
        assert_eq!(t.artist, UNKNOWN_ARTIST);
    }

    #[test]
    fn artist_falls_back_to_album_artist() {
        let mut t = Track { path: PathBuf::from("/a/b.mp3"), album_artist: "Band".into(), ..Default::default() };
        fallbacks(&mut t);
        assert_eq!(t.artist, "Band");
    }
}
