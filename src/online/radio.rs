//! The Radio Browser directory (radio-browser.info): about 58,000 stations,
//! browsable by popularity, genre tag and country. No account needed.

use super::http;
use anyhow::{Result, bail};
use serde::Deserialize;
use std::sync::OnceLock;

const FALLBACK: &str = "de1.api.radio-browser.info";
/// How many stations a list asks for.
const LIMIT: usize = 200;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Station {
    #[serde(rename = "stationuuid")]
    pub uuid: String,
    pub name: String,
    pub url: String,
    pub url_resolved: String,
    pub homepage: String,
    pub favicon: String,
    pub tags: String,
    pub country: String,
    pub countrycode: String,
    pub codec: String,
    pub bitrate: u32,
    #[serde(skip)]
    pub custom: bool,
}

impl Station {
    /// The address to play.
    pub fn stream(&self) -> &str {
        if self.url_resolved.is_empty() { &self.url } else { &self.url_resolved }
    }

    /// Its tags, tidied: "jazz, smooth jazz".
    pub fn tag_list(&self) -> Vec<String> {
        self.tags.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()
    }

    /// The first tag, for grouping (empty when there are none).
    pub fn genre(&self) -> String {
        self.tag_list().into_iter().next().map(|g| title_case(&g)).unwrap_or_default()
    }

    /// "jazz, blues · France · MP3 128 kbps"
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        let tags = self.tag_list();
        if !tags.is_empty() {
            parts.push(tags.into_iter().take(3).collect::<Vec<_>>().join(", "));
        }
        if !self.country.is_empty() {
            parts.push(self.country.clone());
        }
        let mut fmt = self.codec.clone();
        if self.bitrate > 0 {
            if !fmt.is_empty() {
                fmt.push(' ');
            }
            fmt.push_str(&format!("{} kbps", self.bitrate));
        }
        if !fmt.is_empty() {
            parts.push(fmt);
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tag {
    pub name: String,
    pub stationcount: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Country {
    pub name: String,
    #[serde(rename = "iso_3166_1")]
    pub code: String,
    pub stationcount: usize,
}

#[derive(Deserialize)]
struct Server {
    name: String,
}

/// One of the directory's mirrors, picked once per run.
fn server() -> &'static str {
    static SERVER: OnceLock<String> = OnceLock::new();
    SERVER.get_or_init(|| {
        let names: Vec<String> = http::get_json::<Vec<Server>>("https://all.api.radio-browser.info/json/servers")
            .map(|v| v.into_iter().map(|s| s.name).collect())
            .unwrap_or_default();
        if names.is_empty() {
            return FALLBACK.to_string();
        }
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize;
        names[n % names.len()].clone()
    })
}

fn get<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    http::get_json(&format!("https://{}/json/{path}", server()))
}

fn stations(path: &str) -> Result<Vec<Station>> {
    let sep = if path.contains('?') { '&' } else { '?' };
    let list: Vec<Station> = get(&format!("{path}{sep}hidebroken=true&limit={LIMIT}"))?;
    Ok(dedupe(list))
}

/// The directory lists some stations more than once; keep the first of each stream.
fn dedupe(list: Vec<Station>) -> Vec<Station> {
    let mut seen = std::collections::HashSet::new();
    list.into_iter().filter(|s| !s.stream().is_empty() && seen.insert(s.stream().to_string())).collect()
}

pub fn popular() -> Result<Vec<Station>> {
    stations(&format!("stations/topclick/{LIMIT}"))
}

pub fn by_tag(tag: &str) -> Result<Vec<Station>> {
    stations(&format!("stations/bytagexact/{}?order=clickcount&reverse=true", http::encode(tag)))
}

pub fn by_country(code: &str) -> Result<Vec<Station>> {
    stations(&format!("stations/bycountrycodeexact/{}?order=clickcount&reverse=true", http::encode(code)))
}

pub fn search(text: &str) -> Result<Vec<Station>> {
    stations(&format!("stations/search?name={}&order=clickcount&reverse=true", http::encode(text)))
}

/// Tags that say nothing about the music.
const NOT_GENRES: &[&str] = &[
    "music",
    "radio",
    "fm",
    "am",
    "online",
    "online radio",
    "internet radio",
    "webradio",
    "web radio",
    "estación",
    "estacion",
    "entretenimiento",
    "local",
    "local radio",
    "community radio",
    "variety",
    "various",
    "moi merino",
    "norteamérica",
    "north america",
    "europe",
    "latin america",
    "américa",
    "noticias",
    "musica",
    "música",
    "stream",
];

/// The most used genre tags (leaving out places and filler words).
pub fn tags() -> Result<Vec<Tag>> {
    let list: Vec<Tag> = get("tags?order=stationcount&reverse=true&hidebroken=true&limit=200")?;
    let places: std::collections::HashSet<String> =
        countries().unwrap_or_default().into_iter().map(|c| c.name.to_lowercase()).collect();
    Ok(genres_only(list, &places))
}

fn genres_only(list: Vec<Tag>, places: &std::collections::HashSet<String>) -> Vec<Tag> {
    list.into_iter()
        .filter(|t| {
            let n = t.name.trim().to_lowercase();
            t.stationcount >= 20 && !n.is_empty() && !NOT_GENRES.contains(&n.as_str()) && !places.contains(&n)
        })
        .take(120)
        .collect()
}

/// Countries with stations, biggest first.
pub fn countries() -> Result<Vec<Country>> {
    let list: Vec<Country> = get("countries?order=stationcount&reverse=true&hidebroken=true")?;
    Ok(list.into_iter().filter(|c| c.stationcount > 0 && c.code.len() == 2).collect())
}

/// Tell the directory a station was played (it asks clients to).
pub fn click(uuid: &str) {
    if !uuid.is_empty() {
        http::ping(&format!("https://{}/json/url/{uuid}", server()));
    }
}

/// A station link may be a playlist (.pls, .m3u) naming the real stream.
/// Returns the stream itself.
pub fn resolve(url: &str) -> Result<String> {
    let lower = url.to_ascii_lowercase();
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    if ![".pls", ".m3u", ".m3u8"].iter().any(|e| path.ends_with(e)) {
        return Ok(url.to_string());
    }
    let text = http::get_text(url)?;
    // An HLS playlist is the stream itself; GStreamer plays it directly.
    if text.contains("#EXT-X-") {
        return Ok(url.to_string());
    }
    match first_stream(&text) {
        Some(u) => Ok(u),
        None => bail!("no stream address in that playlist"),
    }
}

/// The first http(s) address in a .pls or .m3u body.
pub fn first_stream(text: &str) -> Option<String> {
    text.lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}'))
        .filter_map(|l| {
            let v = if l.to_ascii_lowercase().starts_with("file") { l.split_once('=').map(|(_, v)| v.trim())? } else { l };
            (v.starts_with("http://") || v.starts_with("https://")).then(|| v.to_string())
        })
        .next()
}

/// The user's country from the locale (en_US.UTF-8 → US).
pub fn locale_country() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var)
            && let Some(cc) = country_of_locale(&v)
        {
            return cc;
        }
    }
    String::new()
}

fn country_of_locale(v: &str) -> Option<String> {
    let cc = v.split(['.', '@']).next()?.split('_').nth(1)?;
    (cc.len() == 2 && cc.chars().all(|c| c.is_ascii_alphabetic())).then(|| cc.to_ascii_uppercase())
}

pub fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_stations() {
        let json = r#"[{"stationuuid":"a","name":"Cool FM","url":"http://x/pls","url_resolved":"https://x/live",
            "tags":"pop, dance ,","country":"Nigeria","countrycode":"NG","codec":"MP3","bitrate":128,"votes":3,"geo_lat":null},
            {"stationuuid":"b","name":"Dup","url":"https://x/live","url_resolved":""}]"#;
        let list = dedupe(serde_json::from_str::<Vec<Station>>(json).unwrap());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].stream(), "https://x/live");
        assert_eq!(list[0].tag_list(), vec!["pop", "dance"]);
        assert_eq!(list[0].genre(), "Pop");
        assert_eq!(list[0].summary(), "pop, dance · Nigeria · MP3 128 kbps");
    }

    #[test]
    fn finds_streams_in_playlists() {
        let pls = "[playlist]\nNumberOfEntries=1\nFile1=http://ice.example.com:8000/live\nTitle1=X\n";
        assert_eq!(first_stream(pls).as_deref(), Some("http://ice.example.com:8000/live"));
        let m3u = "#EXTM3U\n#EXTINF:-1,Station\nhttps://s.example.com/a.mp3\n";
        assert_eq!(first_stream(m3u).as_deref(), Some("https://s.example.com/a.mp3"));
        assert_eq!(first_stream("nothing here"), None);
    }

    #[test]
    fn keeps_only_genres() {
        let tag = |n: &str, c| Tag { name: n.into(), stationcount: c };
        let places = ["mexico".to_string(), "méxico".to_string()].into_iter().collect();
        let kept = genres_only(
            vec![tag("pop", 5000), tag("Music", 4000), tag("México", 1900), tag("jazz", 900), tag("rare", 3)],
            &places,
        );
        assert_eq!(kept.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["pop", "jazz"]);
    }

    #[test]
    fn reads_country_from_locale() {
        assert_eq!(country_of_locale("en_US.UTF-8").as_deref(), Some("US"));
        assert_eq!(country_of_locale("de_DE@euro").as_deref(), Some("DE"));
        assert_eq!(country_of_locale("C"), None);
        assert_eq!(title_case("smooth jazz"), "Smooth Jazz");
    }
}
