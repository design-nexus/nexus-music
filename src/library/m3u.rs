//! M3U/M3U8 playlists: read paths (absolute, relative or file:// URIs) and
//! write extended M3U with paths relative to the playlist where possible.

use super::Track;
use std::path::{Path, PathBuf};

pub fn parse(text: &str, base: &Path) -> Vec<PathBuf> {
    text.lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            if let Some(rest) = l.strip_prefix("file://") {
                return gtk::glib::filename_from_uri(&format!("file://{rest}")).ok().map(|(p, _)| p);
            }
            if l.contains("://") {
                return None; // Streams aren't part of the library.
            }
            let p = PathBuf::from(l.replace('\\', "/"));
            Some(if p.is_absolute() { p } else { base.join(p) })
        })
        .collect()
}

pub fn write(tracks: &[Track], base: &Path) -> String {
    let mut out = String::from("#EXTM3U\n");
    for t in tracks {
        out.push_str(&format!("#EXTINF:{},{} - {}\n", t.duration.round() as i64, t.artist, t.title));
        let p = t.path.strip_prefix(base).unwrap_or(&t.path);
        out.push_str(&p.to_string_lossy());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let base = Path::new("/music");
        let tracks = vec![
            Track {
                path: "/music/A/1.flac".into(),
                artist: "X".into(),
                title: "One".into(),
                duration: 61.4,
                ..Default::default()
            },
            Track {
                path: "/elsewhere/2.mp3".into(),
                artist: "Y".into(),
                title: "Two".into(),
                duration: 5.0,
                ..Default::default()
            },
        ];
        let text = write(&tracks, base);
        assert!(text.starts_with("#EXTM3U\n#EXTINF:61,X - One\nA/1.flac\n"));
        assert_eq!(parse(&text, base), vec![PathBuf::from("/music/A/1.flac"), PathBuf::from("/elsewhere/2.mp3")]);
    }

    #[test]
    fn reads_uris_and_skips_streams() {
        let text = "\u{feff}#EXTM3U\nfile:///m/a%20b.ogg\nhttp://radio/x\n\nsub\\c.mp3\n";
        assert_eq!(parse(text, Path::new("/p")), vec![PathBuf::from("/m/a b.ogg"), PathBuf::from("/p/sub/c.mp3")]);
    }
}
