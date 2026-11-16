//! Numbers and times as the admin screen writes them.

use crate::platform::now_ms;

pub(super) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub(super) fn plural(n: usize, what: &str) -> String {
    format!("{} {what}{}", thousands(n as u64), if n == 1 { "" } else { "s" })
}

pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// Like "a", "a and b", "a, b and c".
pub(super) fn and<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_string(),
        [rest @ .., last] => format!("{} and {}", rest.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", "), last.as_ref()),
    }
}

/// An event's message inside a sentence: "Turned off" → "turned off".
pub(super) fn uncapitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

/// Milliseconds with one decimal, like "2,000.0".
/// Milliseconds for a column: a tenth below 10 ms, whole above.
pub(super) fn ms(us: u64) -> String {
    if us < 9_950 {
        let tenths = (us + 50) / 100;
        format!("{}.{}", tenths / 10, tenths % 10)
    } else {
        thousands((us + 500) / 1000)
    }
}

/// Like "4.2 ms", "412 ms" or "2 s".
pub(super) fn duration(us: u64) -> String {
    match us {
        ..10_000 => format!("{:.1} ms", us as f64 / 1000.0),
        10_000..1_000_000 => format!("{} ms", us / 1000),
        _ if us.is_multiple_of(1_000_000) => format!("{} s", us / 1_000_000),
        _ => format!("{:.1} s", us as f64 / 1e6),
    }
}

/// A stretch of time like "0.4 s", "41 s", "2 min 5 s" or "1 h 4 min".
pub(super) fn span(ms: i64) -> String {
    let ms = ms.max(0);
    let (s, min) = ((ms + 500) / 1000, (ms + 30_000) / 60_000);
    match ms {
        0..9_950 => format!("{:.1} s", ms as f64 / 1000.0),
        9_950..59_500 => format!("{s} s"),
        59_500..3_570_000 if s % 60 == 0 => format!("{} min", s / 60),
        59_500..3_570_000 => format!("{} min {} s", s / 60, s % 60),
        _ if min % 60 == 0 => format!("{} h", min / 60),
        _ => format!("{} h {} min", min / 60, min % 60),
    }
}

pub(super) fn fuel(f: u64) -> String {
    match f {
        1_000_000_000.. => format!("{} B", f / 1_000_000_000),
        1_000_000.. => format!("{} M", f / 1_000_000),
        1_000.. => format!("{} k", f / 1_000),
        _ => f.to_string(),
    }
}

pub(super) fn local(at: i64, fmt: &str) -> String {
    chrono::DateTime::from_timestamp_millis(at).map(|t| t.with_timezone(&chrono::Local).format(fmt).to_string()).unwrap_or_default()
}

pub(super) const FULL: &str = "%Y-%m-%d %H:%M:%S";

pub(super) fn hm(at: i64) -> String {
    local(at, "%H:%M")
}

/// The time, and the day too when it isn't today: "07:14", "10 Nov 11:05".
/// One format on every page.
pub(super) fn clock(at: i64) -> String {
    let today = local(at, "%F") == local(now_ms(), "%F");
    local(at, if today { "%H:%M" } else { "%-d %b %H:%M" })
}

pub(super) fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::{and, span};

    #[test]
    fn lists_read_as_a_sentence() {
        assert_eq!(and::<&str>(&[]), "");
        assert_eq!(and(&["a"]), "a");
        assert_eq!(and(&["a", "b"]), "a and b");
        assert_eq!(and(&["a", "b", "c"]), "a, b and c");
    }

    #[test]
    fn spans_round_to_what_a_person_says() {
        let said = [-5, 420, 9_949, 9_950, 41_200, 59_499, 59_500, 125_000, 3_569_999, 3_570_000, 3_840_000, 7_200_000].map(span);
        assert_eq!(said, ["0.0 s", "0.4 s", "9.9 s", "10 s", "41 s", "59 s", "1 min", "2 min 5 s", "59 min 30 s", "1 h", "1 h 4 min", "2 h"]);
    }
}
