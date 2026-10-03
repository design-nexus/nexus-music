//! Blocking HTTP for the radio and podcast directories. Every call here runs
//! off the UI thread (see `cmd::background`).

use anyhow::{Context, Result, bail};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = concat!("nexus-music/", env!("CARGO_PKG_VERSION"));
/// Feeds and directory answers are small; anything bigger is a mistake.
const MAX_TEXT: u64 = 32 * 1024 * 1024;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(20)))
            .build()
            .into()
    })
}

pub fn get_text(url: &str) -> Result<String> {
    let mut res = agent().get(url).call().with_context(|| format!("couldn't reach {}", host(url)))?;
    Ok(res.body_mut().with_config().limit(MAX_TEXT).read_to_string()?)
}

pub fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut res = agent().get(url).call().with_context(|| format!("couldn't reach {}", host(url)))?;
    Ok(res.body_mut().with_config().limit(limit).read_to_vec()?)
}

pub fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    let text = get_text(url)?;
    serde_json::from_str(&text).with_context(|| format!("unexpected answer from {}", host(url)))
}

/// Fire and forget (Radio Browser's click counter).
pub fn ping(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        let _ = agent().get(&url).call();
    });
}

/// Save `url` to `dest` (through a temp file), reporting (done, total) bytes.
pub fn download(url: &str, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let mut res = agent()
        .get(url)
        .config()
        .timeout_recv_body(None)
        .build()
        .call()
        .with_context(|| format!("couldn't reach {}", host(url)))?;
    let total = res.body().content_length().unwrap_or(0);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension("part");
    let result = (|| -> Result<()> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let mut body = res.body_mut().with_config().limit(u64::MAX).reader();
        let mut buf = vec![0u8; 64 * 1024];
        let (mut done, mut last) = (0u64, 0u64);
        loop {
            let n = body.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            done += n as u64;
            if done - last >= 256 * 1024 {
                last = done;
                progress(done, total);
            }
        }
        out.flush()?;
        if done == 0 {
            bail!("the download was empty");
        }
        progress(done, total.max(done));
        Ok(())
    })();
    match result {
        Ok(()) => {
            std::fs::rename(&tmp, dest)?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// "example.com" for error messages.
pub fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Percent-encode a query value.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_finds_hosts() {
        assert_eq!(encode("rock & roll"), "rock%20%26%20roll");
        assert_eq!(encode("Ünï"), "%C3%9Cn%C3%AF");
        assert_eq!(host("https://feeds.example.com/x?y"), "feeds.example.com");
        assert_eq!(host("example.com"), "example.com");
    }
}
