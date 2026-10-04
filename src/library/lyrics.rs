//! Lyrics: an `.lrc` file next to the song, or the lyrics in its tags.
//! LRC lines carry times (`[01:23.45]`), so they can follow the music.

use lofty::prelude::*;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    /// (seconds, text) in time order when synced; times are 0 otherwise.
    pub lines: Vec<(f64, String)>,
    pub synced: bool,
}

/// Look for lyrics: `<song>.lrc` first, then the file's own tags.
pub fn read(song: &Path) -> Option<Lyrics> {
    let lrc = song.with_extension("lrc");
    if let Ok(text) = std::fs::read_to_string(&lrc)
        && let Some(l) = parse(&text)
    {
        return Some(l);
    }
    let file = lofty::read_from_path(song).ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    parse(tag.get_string(ItemKey::Lyrics)?)
}

/// `mm:ss`, `mm:ss.xx` or `mm:ss:xx` → seconds.
fn stamp(s: &str) -> Option<f64> {
    let (m, rest) = s.split_once(':')?;
    let rest = rest.replacen(':', ".", 1);
    let m: f64 = m.trim().parse().ok()?;
    let sec: f64 = rest.trim().parse().ok()?;
    (m >= 0.0 && sec >= 0.0).then_some(m * 60.0 + sec)
}

/// LRC (several stamps on a line, an `[offset:ms]` tag) or plain text.
pub fn parse(text: &str) -> Option<Lyrics> {
    let mut offset = 0.0;
    let mut timed: Vec<(f64, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    for raw in text.lines() {
        let mut line = raw.trim();
        let mut stamps = Vec::new();
        while let Some(rest) = line.strip_prefix('[') {
            let Some((inside, after)) = rest.split_once(']') else { break };
            if let Some(t) = stamp(inside) {
                stamps.push(t);
            } else if let Some(ms) = inside.strip_prefix("offset:") {
                // A positive offset shows the lyrics sooner.
                offset = ms.trim().parse::<f64>().unwrap_or(0.0) / 1000.0;
            } else if !inside.contains(':') {
                break;
            }
            // Other tags ([ar:], [ti:]…) are skipped.
            line = after.trim_start();
        }
        if stamps.is_empty() {
            if !raw.trim_start().starts_with('[') {
                plain.push(raw.trim_end().to_string());
            }
        } else {
            for t in stamps {
                timed.push((t, line.to_string()));
            }
        }
    }
    if !timed.is_empty() {
        for l in &mut timed {
            l.0 = (l.0 - offset).max(0.0);
        }
        timed.sort_by(|a, b| a.0.total_cmp(&b.0));
        return Some(Lyrics { lines: timed, synced: true });
    }
    // Trim blank lines at either end of plain lyrics.
    while plain.last().is_some_and(|l| l.trim().is_empty()) {
        plain.pop();
    }
    let start = plain.iter().position(|l| !l.trim().is_empty())?;
    Some(Lyrics { lines: plain[start..].iter().map(|l| (0.0, l.clone())).collect(), synced: false })
}

/// The line playing at `secs` in synced lyrics.
pub fn line_at(l: &Lyrics, secs: f64) -> Option<usize> {
    if !l.synced {
        return None;
    }
    l.lines.iter().rposition(|(t, _)| *t <= secs + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc() {
        let l = parse("[ar:Someone]\n[00:01.00]One\n[00:10.00][00:20.50]Chorus\n[00:15]Two\n").unwrap();
        assert!(l.synced);
        let lines: Vec<_> = l.lines.iter().map(|(t, s)| (*t, s.as_str())).collect();
        assert_eq!(lines, [(1.0, "One"), (10.0, "Chorus"), (15.0, "Two"), (20.5, "Chorus")]);
        assert_eq!(line_at(&l, 0.5), None);
        assert_eq!(line_at(&l, 12.0), Some(1));
        assert_eq!(line_at(&l, 99.0), Some(3));
    }

    #[test]
    fn applies_offset() {
        let l = parse("[offset:+500]\n[00:02.00]Hi").unwrap();
        assert_eq!(l.lines[0].0, 1.5);
    }

    #[test]
    fn plain_text() {
        let l = parse("\nFirst\n\nSecond\n\n").unwrap();
        assert!(!l.synced);
        assert_eq!(l.lines.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(), ["First", "", "Second"]);
        assert_eq!(parse("  \n"), None);
    }
}
