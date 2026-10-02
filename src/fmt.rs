//! Number formatting for readouts. Everything here is shown in the mono font.

/// A track time: 0:07, 3:42, 1:02:09.
pub fn time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

/// A total length: 42 min, 3 h 12 min.
pub fn total(secs: f64) -> String {
    let m = (secs.max(0.0) / 60.0).round() as u64;
    if m >= 60 { format!("{} h {} min", m / 60, m % 60) } else { format!("{m} min") }
}

/// "1 song", "12 songs".
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

/// 1,204
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Decibels for fader readouts: 0, +3.5, -6.0.
pub fn db(v: f64) -> String {
    if v.abs() < 0.05 { "0".into() } else { format!("{v:+.1}") }
}

/// Hz labels: 31, 250, 1k, 16k.
pub fn hz(f: f64) -> String {
    if f >= 1000.0 {
        let k = f / 1000.0;
        if (k - k.round()).abs() < 0.05 { format!("{}k", k.round() as u64) } else { format!("{k:.1}k") }
    } else {
        format!("{}", f.round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(time(7.4), "0:07");
        assert_eq!(time(222.0), "3:42");
        assert_eq!(time(3729.0), "1:02:09");
        assert_eq!(total(2520.0), "42 min");
        assert_eq!(total(11520.0), "3 h 12 min");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(count(1, "song", "songs"), "1 song");
        assert_eq!(count(1204, "song", "songs"), "1,204 songs");
        assert_eq!(thousands(123), "123");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn formats_units() {
        assert_eq!(db(0.01), "0");
        assert_eq!(db(3.5), "+3.5");
        assert_eq!(hz(31.0), "31");
        assert_eq!(hz(1000.0), "1k");
        assert_eq!(hz(16000.0), "16k");
    }
}
