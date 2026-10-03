//! Podcasts: Apple's directory (top charts, categories and search, no key
//! needed) to find shows, and the shows' own RSS feeds for their episodes.

use super::http;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;

/// Apple's podcast categories (genre ids).
pub const CATEGORIES: &[(u32, &str)] = &[
    (1301, "Arts"),
    (1321, "Business"),
    (1303, "Comedy"),
    (1304, "Education"),
    (1483, "Fiction"),
    (1511, "Government"),
    (1512, "Health & Fitness"),
    (1487, "History"),
    (1305, "Kids & Family"),
    (1502, "Leisure"),
    (1310, "Music"),
    (1489, "News"),
    (1314, "Religion & Spirituality"),
    (1533, "Science"),
    (1324, "Society & Culture"),
    (1545, "Sports"),
    (1318, "Technology"),
    (1488, "True Crime"),
    (1309, "TV & Film"),
];

/// A show as the directory lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Show {
    pub apple_id: String,
    pub title: String,
    pub author: String,
    pub art_url: String,
    /// Empty for chart entries until looked up.
    pub feed: String,
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Feed {
    pub title: String,
    pub author: String,
    pub art_url: String,
    pub description: String,
    pub episodes: Vec<Episode>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Episode {
    pub guid: String,
    pub title: String,
    /// The audio file.
    pub url: String,
    pub mime: String,
    /// Seconds; 0 when the feed doesn't say.
    pub duration: f64,
    /// Unix time; 0 when unknown.
    pub published: i64,
    pub description: String,
}

fn store(country: &str) -> String {
    let c = country.to_ascii_lowercase();
    if c.len() == 2 { c } else { "us".into() }
}

/// Apple's top shows, overall or in one category.
pub fn top(country: &str, genre: Option<u32>) -> Result<Vec<Show>> {
    let genre = genre.map(|g| format!("/genre={g}")).unwrap_or_default();
    let v: Value = http::get_json(&format!("https://itunes.apple.com/{}/rss/toppodcasts/limit=100{genre}/json", store(country)))?;
    Ok(parse_chart(&v))
}

fn parse_chart(v: &Value) -> Vec<Show> {
    let entries = match &v["feed"]["entry"] {
        Value::Array(a) => a.clone(),
        e @ Value::Object(_) => vec![e.clone()],
        _ => Vec::new(),
    };
    let label = |e: &Value, k: &str| e[k]["label"].as_str().unwrap_or_default().trim().to_string();
    entries
        .iter()
        .filter_map(|e| {
            let apple_id = e["id"]["attributes"]["im:id"].as_str()?.to_string();
            let art = e["im:image"].as_array().and_then(|a| a.last()).and_then(|i| i["label"].as_str()).unwrap_or_default();
            Some(Show {
                apple_id,
                title: label(e, "im:name"),
                author: label(e, "im:artist"),
                art_url: bigger_art(art),
                feed: String::new(),
                summary: label(e, "summary"),
            })
        })
        .collect()
}

/// Apple's thumbnails take their size from the file name; ask for 600 px.
fn bigger_art(url: &str) -> String {
    match url.rfind('/') {
        Some(i) if url[i..].contains("x") && url[i..].contains("bb") => format!("{}/600x600bb.jpg", &url[..i]),
        _ => url.to_string(),
    }
}

#[derive(Deserialize)]
struct Search {
    results: Vec<SearchResult>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct SearchResult {
    collection_id: u64,
    collection_name: String,
    artist_name: String,
    feed_url: String,
    artwork_url600: String,
    artwork_url100: String,
    primary_genre_name: String,
}

impl From<SearchResult> for Show {
    fn from(r: SearchResult) -> Show {
        Show {
            apple_id: r.collection_id.to_string(),
            title: r.collection_name,
            author: r.artist_name,
            art_url: if r.artwork_url600.is_empty() { r.artwork_url100 } else { r.artwork_url600 },
            feed: r.feed_url,
            summary: r.primary_genre_name,
        }
    }
}

pub fn search(country: &str, term: &str) -> Result<Vec<Show>> {
    let s: Search = http::get_json(&format!(
        "https://itunes.apple.com/search?media=podcast&entity=podcast&limit=60&country={}&term={}",
        store(country),
        http::encode(term)
    ))?;
    Ok(s.results.into_iter().filter(|r| !r.feed_url.is_empty()).map(Show::from).collect())
}

/// A chart entry's feed address.
pub fn lookup(apple_id: &str) -> Result<String> {
    let s: Search = http::get_json(&format!("https://itunes.apple.com/lookup?entity=podcast&id={}", http::encode(apple_id)))?;
    match s.results.into_iter().map(|r| r.feed_url).find(|f| !f.is_empty()) {
        Some(f) => Ok(f),
        None => bail!("this show has no public feed"),
    }
}

pub fn fetch(feed: &str) -> Result<Feed> {
    let text = http::get_text(feed)?;
    parse_feed(&text).with_context(|| format!("{} isn't a podcast feed", http::host(feed)))
}

// ---------- RSS ----------

const ITUNES: &str = "http://www.itunes.com/dtds/podcast-1.0.dtd";

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() != Some(ITUNES))
}

fn itunes<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name && c.tag_name().namespace() == Some(ITUNES))
}

fn text_of(n: Option<roxmltree::Node>) -> String {
    n.map(|n| n.children().filter_map(|c| c.text()).collect::<String>().trim().to_string()).unwrap_or_default()
}

pub fn parse_feed(xml: &str) -> Result<Feed> {
    let doc = roxmltree::Document::parse_with_options(xml, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })?;
    let Some(channel) = doc.descendants().find(|n| n.has_tag_name("channel")) else { bail!("no channel") };
    let art_url = itunes(channel, "image")
        .and_then(|i| i.attribute("href"))
        .map(str::to_string)
        .unwrap_or_else(|| text_of(child(channel, "image").and_then(|i| child(i, "url"))));
    let mut description = text_of(child(channel, "description"));
    if description.is_empty() {
        description = text_of(itunes(channel, "summary"));
    }
    let mut episodes: Vec<Episode> = channel
        .children()
        .filter(|n| n.has_tag_name("item"))
        .filter_map(|item| {
            let enclosure = child(item, "enclosure")?;
            let url = enclosure.attribute("url")?.trim().to_string();
            if url.is_empty() {
                return None;
            }
            let mut desc = text_of(itunes(item, "summary"));
            if desc.is_empty() {
                desc = text_of(child(item, "description"));
            }
            let guid = text_of(child(item, "guid"));
            Some(Episode {
                guid: if guid.is_empty() { url.clone() } else { guid },
                title: text_of(child(item, "title")),
                url,
                mime: enclosure.attribute("type").unwrap_or_default().to_string(),
                duration: parse_duration(&text_of(itunes(item, "duration"))),
                published: parse_date(&text_of(child(item, "pubDate"))).unwrap_or(0),
                description: plain_text(&desc),
            })
        })
        .collect();
    episodes.sort_by_key(|e| std::cmp::Reverse(e.published));
    Ok(Feed {
        title: text_of(child(channel, "title")),
        author: text_of(itunes(channel, "author")),
        art_url,
        description: plain_text(&description),
        episodes,
    })
}

/// `itunes:duration`: seconds, mm:ss or hh:mm:ss.
pub fn parse_duration(s: &str) -> f64 {
    let mut total = 0.0;
    for part in s.trim().split(':') {
        let Ok(v) = part.trim().parse::<f64>() else { return 0.0 };
        total = total * 60.0 + v;
    }
    total
}

/// An RFC 2822 date ("Wed, 25 Sep 2026 14:00:00 +0000") as Unix time.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = s.split_once(',').map_or(s, |(_, rest)| rest).trim();
    let mut it = s.split_whitespace();
    let day: i64 = it.next()?.parse().ok()?;
    let month = it.next()?.get(..3)?.to_ascii_lowercase();
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| *m == month)? as i64
        + 1;
    let mut year: i64 = it.next()?.parse().ok()?;
    if year < 100 {
        year += if year < 70 { 2000 } else { 1900 };
    }
    let time = it.next().unwrap_or("00:00:00");
    let mut hms = time.split(':').map(|v| v.parse::<i64>().unwrap_or(0));
    let (h, m, sec) = (hms.next().unwrap_or(0), hms.next().unwrap_or(0), hms.next().unwrap_or(0));
    let offset = match it.next().unwrap_or("+0000") {
        z if z.starts_with('+') || z.starts_with('-') => {
            let n: i64 = z[1..].parse().ok()?;
            let mins = (n / 100) * 60 + n % 100;
            if z.starts_with('-') { -mins } else { mins }
        }
        "EDT" => -240,
        "EST" | "CDT" => -300,
        "CST" | "MDT" => -360,
        "MST" | "PDT" => -420,
        "PST" => -480,
        _ => 0,
    };
    Some(days_from_civil(year, month, day) * 86400 + h * 3600 + m * 60 + sec - offset * 60)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Show notes come as HTML: keep the words, one blank line between paragraphs.
pub fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut tag = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match (in_tag, c) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                let name = tag.trim_start_matches('/').split([' ', '/']).next().unwrap_or("").to_ascii_lowercase();
                if matches!(name.as_str(), "p" | "br" | "div" | "li" | "h1" | "h2" | "h3" | "h4") {
                    out.push('\n');
                }
            }
            (true, c) => tag.push(c),
            (false, c) => out.push(c),
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#8217;", "’")
        .replace("&rsquo;", "’")
        .replace("&ldquo;", "“")
        .replace("&rdquo;", "”")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…");
    let mut paras: Vec<String> = Vec::new();
    for line in out.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() {
            paras.push(line);
        }
    }
    paras.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
<channel>
  <title>Show &amp; Tell</title>
  <description><![CDATA[<p>A show.</p><p>Second&nbsp;para</p>]]></description>
  <itunes:author>Someone</itunes:author>
  <itunes:image href="https://img.example.com/show.jpg"/>
  <image><url>https://img.example.com/rss.jpg</url></image>
  <item>
    <title>Older</title>
    <guid>ep-1</guid>
    <pubDate>Mon, 01 Jan 2024 00:00:00 +0000</pubDate>
    <enclosure url="https://cdn.example.com/1.mp3" type="audio/mpeg" length="100"/>
    <itunes:duration>22:05</itunes:duration>
  </item>
  <item>
    <title>Newer</title>
    <pubDate>Tue, 02 Jan 2024 10:30:00 -0500</pubDate>
    <enclosure url="https://cdn.example.com/2.mp3" type="audio/mpeg"/>
    <itunes:duration>3725</itunes:duration>
    <description>Plain notes</description>
  </item>
  <item><title>No audio</title></item>
</channel>
</rss>"#;

    #[test]
    fn parses_a_feed() {
        let f = parse_feed(FEED).unwrap();
        assert_eq!(f.title, "Show & Tell");
        assert_eq!(f.author, "Someone");
        assert_eq!(f.art_url, "https://img.example.com/show.jpg");
        assert_eq!(f.description, "A show.\n\nSecond para");
        assert_eq!(f.episodes.len(), 2);
        assert_eq!(f.episodes[0].title, "Newer");
        assert_eq!(f.episodes[0].guid, "https://cdn.example.com/2.mp3");
        assert_eq!(f.episodes[0].duration, 3725.0);
        assert_eq!(f.episodes[0].description, "Plain notes");
        assert_eq!(f.episodes[1].guid, "ep-1");
        assert_eq!(f.episodes[1].duration, 1325.0);
        assert_eq!(f.episodes[1].published, 1704067200);
    }

    #[test]
    fn falls_back_to_the_rss_image() {
        let f = parse_feed(&FEED.replace(r#"<itunes:image href="https://img.example.com/show.jpg"/>"#, "")).unwrap();
        assert_eq!(f.art_url, "https://img.example.com/rss.jpg");
    }

    #[test]
    fn parses_durations_and_dates() {
        assert_eq!(parse_duration("1:02:03"), 3723.0);
        assert_eq!(parse_duration("90"), 90.0);
        assert_eq!(parse_duration(""), 0.0);
        assert_eq!(parse_duration("n/a"), 0.0);
        assert_eq!(parse_date("Tue, 02 Jan 2024 10:30:00 -0500"), Some(1704209400));
        assert_eq!(parse_date("02 Jan 2024 15:30:00 GMT"), Some(1704209400));
        assert_eq!(parse_date("Tue, 02 Jan 2024 07:30:00 PST"), Some(1704209400));
        assert_eq!(parse_date("garbage"), None);
    }

    #[test]
    fn reads_charts_and_search() {
        let chart = serde_json::json!({"feed": {"entry": {
            "im:name": {"label": "The Daily"}, "im:artist": {"label": "NYT"},
            "im:image": [{"label": "https://a/x.jpg/55x55bb.png"}, {"label": "https://a/x.jpg/170x170bb.png"}],
            "summary": {"label": "News"}, "id": {"attributes": {"im:id": "1200361736"}}}}});
        let shows = parse_chart(&chart);
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].apple_id, "1200361736");
        assert_eq!(shows[0].art_url, "https://a/x.jpg/600x600bb.jpg");
        let s: Search = serde_json::from_str(
            r#"{"resultCount":1,"results":[{"collectionId":152249110,"collectionName":"Radiolab","artistName":"WNYC",
            "feedUrl":"https://feeds.example.com/r","artworkUrl600":"https://a/600.jpg","trackPrice":0.0}]}"#,
        )
        .unwrap();
        let show: Show = s.results.into_iter().next().unwrap().into();
        assert_eq!(show.feed, "https://feeds.example.com/r");
        assert_eq!(show.apple_id, "152249110");
    }
}
