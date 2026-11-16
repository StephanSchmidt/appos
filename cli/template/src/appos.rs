//! The app's side of AppOS: bindings generated from `wit/` (the shell's
//! contract) plus helpers: routes and modules, links, the request's time,
//! dates, kv with JSON, form errors. `appos new` writes this file and
//! `appos sync` brings it up to date with the `appos` you have, so keep your
//! own code elsewhere. It uses serde and serde_json: keep them in Cargo.toml.
//!
//! An app has two functions. `init` runs once and registers its modules'
//! routes and menu entries (`register(MODULES)`). `start` runs per request
//! (`dispatch(MODULES, ...)`): it sets named HTML with `page::set_html`
//! ("page" for the whole page, any other name for an htmx update of the
//! element with that id) and calls `page::done()`.
//!
//! Templates are Askama, in `templates/`. A `{% block name %}` doubles as the
//! fragment `name`: list it in `blocks = [...]` and render it with `as_name()`.

#![allow(dead_code)]

wit_bindgen::generate!({
    world: "app",
    path: "wit",
    default_bindings_module: "crate::appos",
});

pub use self::appos::framework::types::Request;
pub use self::appos::framework::{log, page, registry};
pub use self::exports::appos::framework::handler::Guest as App;

/// A route: method, path and the function that handles it. `path` is below
/// the app's base; `:name` segments become `req.param("name")`.
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub handler: fn(&Request),
}

impl Route {
    pub const fn get(path: &'static str, handler: fn(&Request)) -> Route {
        Route {
            method: "GET",
            path,
            handler,
        }
    }

    pub const fn post(path: &'static str, handler: fn(&Request)) -> Route {
        Route {
            method: "POST",
            path,
            handler,
        }
    }

    /// The name the shell hands to `start`, e.g. "POST /greet": unique as
    /// long as method and path are.
    fn name(&self) -> String {
        format!("{} {}", self.method, self.path)
    }

    fn matches(&self, handler: &str) -> bool {
        handler
            .strip_prefix(self.method)
            .and_then(|rest| rest.strip_prefix(' '))
            == Some(self.path)
    }
}

/// A part of the app: its routes and its menu entries (label, path). The
/// app lists its modules once, `const MODULES: &[Module] = &[hello::MODULE];`,
/// for both `register` and `dispatch`.
pub struct Module {
    pub routes: &'static [Route],
    pub menu: &'static [(&'static str, &'static str)],
}

/// Registers every module's routes and menu entries, during `init`.
pub fn register(modules: &[Module]) {
    for module in modules {
        for route in module.routes {
            registry::add_route(route.method, route.path, &route.name());
        }
        for (label, path) in module.menu {
            registry::add_menu(label, path);
        }
    }
}

/// Runs the route the shell picked for this request, from all modules'
/// routes; a 404 page if there is none. A request whose time isn't RFC
/// 3339 gets a 500 page here, so `Request::now` can't fail in a handler.
pub fn dispatch(modules: &[Module], handler: &str, req: &Request) {
    if let Err(why) = checked_time(req) {
        return failed(&why);
    }
    match find_route(modules, handler) {
        Some(route) => (route.handler)(req),
        None => not_found(),
    }
}

/// The request's time; the shell sends RFC 3339.
fn checked_time(req: &Request) -> Result<time::Time, String> {
    time::Time::parse(&req.now)
        .ok_or_else(|| format!("the request's time {:?} is not RFC 3339", req.now))
}

fn find_route<'a>(modules: &'a [Module], handler: &str) -> Option<&'a Route> {
    modules
        .iter()
        .flat_map(|module| module.routes)
        .find(|r| r.matches(handler))
}

impl Request {
    /// When the request arrived, on the platform's clock. `dispatch`
    /// answers a request whose time isn't one before any handler runs.
    pub fn now(&self) -> time::Time {
        checked_time(self).expect("dispatch checked the request's time")
    }

    /// A route parameter, e.g. `:event`.
    pub fn param(&self, name: &str) -> Option<&str> {
        find(&self.params, name)
    }

    /// A query parameter.
    pub fn query(&self, name: &str) -> Option<&str> {
        find(&self.query, name)
    }

    /// A form field of a POST.
    pub fn form(&self, name: &str) -> Option<&str> {
        find(&self.form, name)
    }

    /// A link to a path inside this app: `req.url("/desk")` -> "/events/desk",
    /// `req.url("/")` -> "/events". In templates, pass `base` and write
    /// `{{ base }}/desk`.
    pub fn url(&self, path: &str) -> String {
        self.url_parts(path).concat()
    }

    /// A link with a query string, values encoded:
    /// `req.url_with("/", &[("q", "a b")])` -> "/events?q=a%20b".
    pub fn url_with(&self, path: &str, query: &[(&str, &str)]) -> String {
        let mut url = self.url(path);
        let query_len = query
            .iter()
            .map(|(key, value)| encoded_len(key) + encoded_len(value) + 2)
            .sum::<usize>();
        url.reserve(query_len.max(1));
        url.push('?');
        for (i, (key, value)) in query.iter().enumerate() {
            if i > 0 {
                url.push('&');
            }
            encode_into(key, &mut url);
            url.push('=');
            encode_into(value, &mut url);
        }
        url
    }

    /// The same link with the site's origin, e.g. for QR codes and emails.
    pub fn absolute_url(&self, path: &str) -> String {
        let [base, separator, path] = self.url_parts(path);
        [self.origin.as_str(), base, separator, path].concat()
    }

    fn url_parts<'a>(&'a self, path: &'a str) -> [&'a str; 3] {
        let path = path.trim_prefix('/');
        let separator = if path.is_empty() || path.starts_with('?') {
            ""
        } else {
            "/"
        };
        [&self.base, separator, path]
    }
}

/// Percent-encodes a query value.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(encoded_len(value));
    encode_into(value, &mut out);
    out
}

/// Appends a percent-encoded query value to an existing buffer.
pub fn encode_into(value: &str, out: &mut String) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for b in value.bytes() {
        if unreserved(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 15) as usize] as char);
        }
    }
}

fn encoded_len(value: &str) -> usize {
    value.len() + 2 * value.bytes().filter(|b| !unreserved(*b)).count()
}

fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

fn find<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Renders an Askama template or one of its blocks. A template error is
/// logged and shown as a short message instead of breaking the response.
pub fn html(template: &impl askama::Template) -> String {
    template.render().unwrap_or_else(|e| {
        log::warn(&format!("template error: {e}"));
        "<p class=\"error\">This part of the page could not be shown.</p>".into()
    })
}

/// Adds CSS to the page's <head>, e.g. the app's design:
/// `appos::style(include_str!("../../templates/style.css"))`.
pub fn style(css: &str) {
    page::add_header(&format!("<style>{css}</style>"));
}

/// Sets the page's <title>.
pub fn title(title: &str) {
    page::add_header(&format!("<title>{}</title>", escape(title)));
}

/// HTML-escapes text for add_header.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A 404 page.
pub fn not_found() {
    page::set_status(404);
    title("Not found");
    page::set_html(
        "page",
        "<h1 style=\"font-family: system-ui; text-align: center; margin: 4rem 1rem\">Not found</h1>",
    );
}

/// Stored data the app can't read (`kv::Unreadable`): a 500 page, and the
/// key and why in the log. See ERRORS in `appos guide`.
pub fn unreadable(e: &kv::Unreadable) {
    failed(&e.to_string());
}

/// Status 500 and a page saying so; `why` goes to the log, not the page.
fn failed(why: &str) {
    log::warn(why);
    page::set_status(500);
    title("Can't be shown");
    page::set_html(
        "page",
        "<h1 style=\"font-family: system-ui; text-align: center; margin: 4rem 1rem\">This page can't be shown right now</h1>",
    );
}

/// Key-value storage, private to the app (see STORAGE in `appos guide`):
/// get, set, delete and scan from the shell, plus JSON helpers that store
/// one struct per key.
pub mod kv {
    // An app that stores nothing (`grant = []`) never uses kv.
    #[allow(unused_imports)]
    pub use super::appos::framework::kv::*;

    use serde::Serialize;
    use serde::de::DeserializeOwned;

    /// Keys `scan_all_json` asks for per scan. A scan stops the request if
    /// it returns more than 4 MB of keys and values; pages of 100 keep that
    /// far away (40 KB per value) and cost 1 I/O unit each.
    pub const PAGE: u32 = 100;

    /// Text under a key that isn't the JSON the app expects. Stored data
    /// can be anything: another version of the app, a person or a bug may
    /// have written it. The usecase returns this and the handler answers
    /// with `appos::unreadable` (see ERRORS in `appos guide`).
    #[derive(Clone, Debug, PartialEq, thiserror::Error)]
    #[error("{key} is unreadable: {reason}")]
    pub struct Unreadable {
        pub key: String,
        pub reason: String,
    }

    /// The value under `key`, read from JSON: None if there is none,
    /// Unreadable if it isn't the JSON of a `T`.
    pub fn get_json<T: DeserializeOwned>(key: &str) -> Result<Option<T>, Unreadable> {
        get(key).map(|json| parse(key, &json)).transpose()
    }

    /// Stores `value` under `key` as JSON.
    pub fn set_json(key: &str, value: &impl Serialize) {
        set(key, &to_json(value));
    }

    /// One `scan`, read from JSON: at most `limit` entries (up to 1000),
    /// in key order, from the last key with `reverse`. An entry that can't
    /// be read is logged and left out, so one bad entry doesn't take a list
    /// down.
    pub fn scan_json<T: DeserializeOwned>(
        prefix: &str,
        after: Option<&str>,
        limit: u32,
        reverse: bool,
    ) -> Vec<(String, T)> {
        read(scan(prefix, after, limit, reverse))
    }

    /// Every entry under `prefix`, read from JSON, in key order (from the
    /// last key with `reverse`): `PAGE` keys per scan. Unreadable entries
    /// are logged and left out.
    pub fn scan_all_json<T: DeserializeOwned>(prefix: &str, reverse: bool) -> Vec<(String, T)> {
        pages(|after| scan(prefix, after, PAGE, reverse))
    }

    /// A value as JSON text.
    pub fn to_json(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap_or_else(|e| panic!("could not write JSON: {e}"))
    }

    /// Scans page after page, each going on after the last key of the one
    /// before, until a page is not full.
    fn pages<T: DeserializeOwned>(
        mut scan: impl FnMut(Option<&str>) -> Vec<(String, String)>,
    ) -> Vec<(String, T)> {
        let mut all = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page = scan(after.as_deref());
            let full = page.len() == PAGE as usize;
            after = page.last().map(|(key, _)| key.clone());
            all.extend(read(page));
            if !full {
                return all;
            }
        }
    }

    fn read<T: DeserializeOwned>(page: Vec<(String, String)>) -> Vec<(String, T)> {
        page.into_iter()
            .filter_map(|(key, json)| match parse(&key, &json) {
                Ok(value) => Some((key, value)),
                Err(e) => {
                    super::log::warn(&format!("{e}, skipped"));
                    None
                }
            })
            .collect()
    }

    fn parse<T: DeserializeOwned>(key: &str, json: &str) -> Result<T, Unreadable> {
        serde_json::from_str(json).map_err(|e| Unreadable {
            key: key.to_string(),
            reason: e.to_string(),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn scanning_all_goes_on_after_the_last_key_until_a_page_is_not_full() {
            let entries: Vec<(String, String)> = (0..250)
                .map(|n| (format!("k/{n:04}"), n.to_string()))
                .collect();
            let mut scans = 0;
            let all: Vec<(String, u32)> = pages(|after| {
                scans += 1;
                entries
                    .iter()
                    .filter(|(key, _)| after.is_none_or(|after| key.as_str() > after))
                    .take(PAGE as usize)
                    .cloned()
                    .collect()
            });
            assert_eq!(scans, 3);
            assert_eq!(all.len(), 250);
            assert_eq!(all[249], ("k/0249".to_string(), 249));
        }

        #[test]
        fn text_that_isnt_the_json_is_unreadable_with_its_key() {
            assert_eq!(parse::<u32>("k/1", "7"), Ok(7));
            let e = parse::<u32>("k/1", "seven").unwrap_err();
            assert_eq!(e.key, "k/1");
            assert!(e.to_string().starts_with("k/1 is unreadable: "), "{e}");
        }
    }
}

/// Dates and times without a crate: the request's time (`req.now()`), RFC
/// 3339 text, calendar dates and their arithmetic. Time zones and their
/// summer time are the app's: a `Time` knows only the offset from UTC its
/// wall clock shows.
pub mod time {
    use std::fmt;

    pub const MINUTE: i64 = 60;
    pub const HOUR: i64 = 60 * MINUTE;
    pub const DAY: i64 = 24 * HOUR;

    /// A moment, and the wall clock it is read on: seconds since 1970-01-01
    /// 00:00 UTC, and the offset from UTC in seconds (3600 for "+01:00").
    /// Stored (serde) and printed as RFC 3339: "2026-11-11T14:05:09+01:00".
    /// Two times are equal when they are the same moment on the same clock.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Time {
        unix: i64,
        offset: i64,
    }

    impl Time {
        /// The moment `unix` seconds after 1970-01-01 00:00 UTC, in UTC.
        pub const fn utc(unix: i64) -> Time {
            Time { unix, offset: 0 }
        }

        /// Parses RFC 3339 as the shell sends it: "2026-11-11T14:05:09+01:00",
        /// "2026-11-11T13:05:09.25Z" (fractions of a second are dropped).
        /// None if it isn't a real time.
        pub fn parse(text: &str) -> Option<Time> {
            let date = Date::parse(text.get(..10)?)?;
            if !matches!(text.get(10..11)?, "T" | "t" | " ")
                || text.get(13..14)? != ":"
                || text.get(16..17)? != ":"
            {
                return None;
            }
            let hour = digits(text.get(11..13)?, 23)?;
            let minute = digits(text.get(14..16)?, 59)?;
            let second = digits(text.get(17..19)?, 60)?;
            let mut zone = text.get(19..)?;
            if let Some(fraction) = zone.strip_prefix('.') {
                zone = fraction.trim_start_matches(|c: char| c.is_ascii_digit());
                if zone.len() == fraction.len() {
                    return None;
                }
            }
            let offset = match zone {
                "Z" | "z" => 0,
                _ => {
                    let sign = match zone.get(..1)? {
                        "+" => 1,
                        "-" => -1,
                        _ => return None,
                    };
                    if zone.len() != 6 || zone.get(3..4)? != ":" {
                        return None;
                    }
                    let hours = digits(zone.get(1..3)?, 23)?;
                    let minutes = digits(zone.get(4..6)?, 59)?;
                    sign * (hours * HOUR + minutes * MINUTE)
                }
            };
            let wall = date.days() * DAY + hour * HOUR + minute * MINUTE + second;
            Some(Time {
                unix: wall - offset,
                offset,
            })
        }

        /// Seconds since 1970-01-01 00:00 UTC: times subtract and compare
        /// as numbers.
        pub const fn unix(self) -> i64 {
            self.unix
        }

        /// The wall clock's offset from UTC, in seconds.
        pub const fn offset(self) -> i64 {
            self.offset
        }

        /// The same moment on another wall clock, `offset` seconds from UTC
        /// (0 for UTC).
        pub const fn at_offset(self, offset: i64) -> Time {
            Time {
                unix: self.unix,
                offset,
            }
        }

        /// `secs` seconds later (earlier if negative), on the same clock.
        pub const fn plus(self, secs: i64) -> Time {
            Time {
                unix: self.unix + secs,
                offset: self.offset,
            }
        }

        /// The date its wall clock shows.
        pub fn date(self) -> Date {
            Date::from_days(self.wall().div_euclid(DAY))
        }

        /// The hour, minute and second its wall clock shows.
        pub fn clock(self) -> (u32, u32, u32) {
            let s = self.wall().rem_euclid(DAY);
            (
                (s / HOUR) as u32,
                (s % HOUR / MINUTE) as u32,
                (s % MINUTE) as u32,
            )
        }

        fn wall(self) -> i64 {
            self.unix + self.offset
        }
    }

    /// RFC 3339 on its wall clock, to the second: "2026-11-11T14:05:09+01:00",
    /// "…Z" in UTC.
    impl fmt::Display for Time {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            let (hour, minute, second) = self.clock();
            write!(f, "{}T{hour:02}:{minute:02}:{second:02}", self.date())?;
            match self.offset {
                0 => f.write_str("Z"),
                o => {
                    let sign = if o < 0 { '-' } else { '+' };
                    let o = o.abs();
                    write!(f, "{sign}{:02}:{:02}", o / HOUR, o % HOUR / MINUTE)
                }
            }
        }
    }

    impl serde::Serialize for Time {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_str(self)
        }
    }

    impl<'de> serde::Deserialize<'de> for Time {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Time, D::Error> {
            let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
            Time::parse(&text)
                .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not RFC 3339")))
        }
    }

    /// A calendar date, like 2026-12-04. Printed as that.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Date {
        year: i64,
        month: u32,
        day: u32,
    }

    impl Date {
        /// The date, if it is one: `Date::new(2028, 2, 29)`.
        pub fn new(year: i64, month: u32, day: u32) -> Option<Date> {
            let real = (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day);
            real.then_some(Date { year, month, day })
        }

        /// Parses "2026-12-04", as a date field sends it. None if it isn't a
        /// real date.
        pub fn parse(text: &str) -> Option<Date> {
            let text = text.trim();
            if text.len() != 10 || text.get(4..5)? != "-" || text.get(7..8)? != "-" {
                return None;
            }
            let month = digits(text.get(5..7)?, 12)?;
            let day = digits(text.get(8..10)?, 31)?;
            Date::new(digits(text.get(..4)?, 9999)?, month as u32, day as u32)
        }

        /// The date `days` days after 1970-01-01 (before it if negative).
        pub fn from_days(days: i64) -> Date {
            // Howard Hinnant's civil_from_days.
            let z = days + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
            let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
            let year = yoe + era * 400 + i64::from(month <= 2);
            Date { year, month, day }
        }

        /// Days since 1970-01-01: dates subtract and compare as numbers.
        pub fn days(self) -> i64 {
            // Howard Hinnant's days_from_civil.
            let year = if self.month <= 2 {
                self.year - 1
            } else {
                self.year
            };
            let era = year.div_euclid(400);
            let yoe = year - era * 400;
            let mp = (i64::from(self.month) + 9) % 12;
            let doy = (153 * mp + 2) / 5 + i64::from(self.day) - 1;
            let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
            era * 146_097 + doe - 719_468
        }

        pub fn year(self) -> i64 {
            self.year
        }

        /// 1 for January to 12 for December.
        pub fn month(self) -> u32 {
            self.month
        }

        pub fn day(self) -> u32 {
            self.day
        }

        /// 0 for Monday to 6 for Sunday.
        pub fn weekday(self) -> u32 {
            (self.days() + 3).rem_euclid(7) as u32
        }

        /// `n` days later (earlier if negative).
        pub fn plus_days(self, n: i64) -> Date {
            Date::from_days(self.days() + n)
        }

        /// The moment a wall clock `offset` seconds from UTC (0 for UTC)
        /// shows hour:minute:second on this date.
        pub fn at(self, hour: u32, minute: u32, second: u32, offset: i64) -> Time {
            let wall = self.days() * DAY
                + i64::from(hour) * HOUR
                + i64::from(minute) * MINUTE
                + i64::from(second);
            Time {
                unix: wall - offset,
                offset,
            }
        }
    }

    impl fmt::Display for Date {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
        }
    }

    /// Days in a month (1 to 12) of a year.
    pub fn days_in_month(year: i64, month: u32) -> u32 {
        match month {
            2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// Parses a time of day as a time field sends it: "16:00", also with
    /// seconds and their fractions ("16:00:05.250", the fraction dropped).
    /// (hour, minute, second); None if it isn't one.
    pub fn parse_time(text: &str) -> Option<(u32, u32, u32)> {
        let mut parts = text.trim().split(':');
        let hour = two_digits(parts.next()?, 23)?;
        let minute = two_digits(parts.next()?, 59)?;
        let second = match parts.next() {
            None => 0,
            Some(seconds) => {
                let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, "0"));
                if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                two_digits(whole, 59)?
            }
        };
        if parts.next().is_some() {
            return None;
        }
        Some((hour, minute, second))
    }

    fn two_digits(text: &str, max: i64) -> Option<u32> {
        if text.len() != 2 {
            return None;
        }
        digits(text, max).map(|n| n as u32)
    }

    /// ASCII digits only (no sign, no spaces), at most `max`.
    fn digits(text: &str, max: i64) -> Option<i64> {
        if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        text.parse().ok().filter(|n| *n <= max)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn at(text: &str) -> Time {
            Time::parse(text).unwrap()
        }

        #[test]
        fn parses_and_prints_what_the_shell_sends() {
            let t = at("2026-11-11T14:05:09+01:00");
            assert_eq!(t.offset(), HOUR);
            assert_eq!(t.to_string(), "2026-11-11T14:05:09+01:00");
            assert_eq!(at("2026-11-11T13:05:09Z").unix(), t.unix());
            assert_eq!(at("2026-11-11T13:05:09.123456Z").unix(), t.unix());
            assert_eq!(at("2026-11-11T08:05:09-05:00").unix(), t.unix());
            assert_eq!(at("1970-01-01T00:00:00Z").unix(), 0);
            assert_eq!(Time::utc(0).to_string(), "1970-01-01T00:00:00Z");
            assert_eq!(
                t.at_offset(-5 * HOUR).to_string(),
                "2026-11-11T08:05:09-05:00"
            );
        }

        #[test]
        fn refuses_what_isnt_a_time() {
            for bad in [
                "",
                "yesterday",
                "2026-11-11",
                "2026-13-01T00:00:00Z",
                "2026-02-30T00:00:00Z",
                "2026-11-11T24:00:00Z",
                "2026-11-11T14:60:00Z",
                "2026-11-11T14:05:09",
                "2026-11-11T14:05:09+0100",
                "2026-11-11T14:05:09.Z",
                "2026-11-11T14:05:09.+01:00",
                "2026-11-11T14:05:09+24:00",
                "2026-11-11T14:05:09-01:60",
                "2026-11-11T14:05:09+01:00 ",
            ] {
                assert_eq!(Time::parse(bad), None, "{bad}");
            }
        }

        #[test]
        fn moves_across_months_and_years_on_the_same_clock() {
            let t = at("2026-12-15T10:00:00+01:00");
            assert_eq!(t.plus(30 * DAY).to_string(), "2027-01-14T10:00:00+01:00");
            let t = at("2026-03-01T00:30:00-05:00");
            assert_eq!(t.plus(-HOUR).to_string(), "2026-02-28T23:30:00-05:00");
            assert_eq!(t.plus(-HOUR).date(), Date::new(2026, 2, 28).unwrap());
            assert_eq!(t.clock(), (0, 30, 0));
        }

        #[test]
        fn stores_as_rfc_3339_text() {
            let t = at("2026-11-11T14:05:09+01:00");
            let json = serde_json::to_string(&t).unwrap();
            assert_eq!(json, r#""2026-11-11T14:05:09+01:00""#);
            assert_eq!(serde_json::from_str::<Time>(&json).unwrap(), t);
            assert!(serde_json::from_str::<Time>(r#""soon""#).is_err());
        }

        #[test]
        fn dates_are_checked_against_the_calendar() {
            assert!(Date::parse("2028-02-29").is_some());
            assert!(Date::parse("2027-02-29").is_none());
            assert!(Date::parse("2026-13-01").is_none());
            assert!(Date::parse("15.12.2026").is_none());
            assert!(Date::parse("+026-12-15").is_none());
            assert_eq!(
                Date::parse(" 2026-12-04 ").unwrap().to_string(),
                "2026-12-04"
            );
            assert_eq!(Date::new(2026, 4, 31), None);
        }

        #[test]
        fn dates_count_days_and_weekdays() {
            for days in [-1, 0, 59, 10_957, 20_405, 20_770, 47_482] {
                assert_eq!(Date::from_days(days).days(), days);
            }
            let friday = Date::new(2026, 12, 4).unwrap();
            assert_eq!(friday.weekday(), 4);
            assert_eq!(Date::from_days(0).weekday(), 3, "1970-01-01 was a Thursday");
            assert_eq!(friday.plus_days(28).to_string(), "2027-01-01");
            let four_pm = friday.at(16, 0, 0, HOUR);
            assert_eq!(four_pm.to_string(), "2026-12-04T16:00:00+01:00");
            assert_eq!(four_pm.unix(), at("2026-12-04T15:00:00Z").unix());
        }

        #[test]
        fn times_of_day_as_fields_send_them() {
            assert_eq!(parse_time("16:00"), Some((16, 0, 0)));
            assert_eq!(parse_time("16:00:05"), Some((16, 0, 5)));
            assert_eq!(parse_time("16:00:05.125"), Some((16, 0, 5)));
            for bad in [
                "24:00",
                "4pm",
                "16:60",
                "16:00:99",
                "16:00:00.",
                "16:00:00.x",
                "6:00",
                "16:00:00:00",
            ] {
                assert_eq!(parse_time(bad), None, "{bad}");
            }
        }
    }
}

/// What didn't parse in a form: each field's error, by the field's name,
/// in the form's order. Parse every field, so the page shows all errors at
/// once (see FORMS in `appos guide`):
///
/// ```ignore
/// let mut errors = FormErrors::default();
/// let name = errors.keep("name", Name::parse(req.form("name").unwrap_or("")));
/// let email = errors.keep("email", Email::parse(req.form("email").unwrap_or("")));
/// let (Some(name), Some(email)) = (name, email) else {
///     return invalid(req, errors); // 422, each message next to its field
/// };
/// ```
///
/// In a template, `errors.message("name")` is the text next to the field.
#[derive(Debug)]
pub struct FormErrors<E>(Vec<(&'static str, E)>);

impl<E> Default for FormErrors<E> {
    fn default() -> Self {
        FormErrors(Vec::new())
    }
}

impl<E> FormErrors<E> {
    /// Notes an error for the field, e.g. one a usecase found after every
    /// field parsed. A field shows its first error.
    pub fn add(&mut self, field: &'static str, error: E) {
        self.0.push((field, error));
    }

    /// The value if it parsed; otherwise None, and the error is kept for
    /// the field.
    pub fn keep<T>(&mut self, field: &'static str, parsed: Result<T, E>) -> Option<T> {
        parsed.map_err(|e| self.add(field, e)).ok()
    }

    /// The same, with `T`'s default for a field that didn't parse: build a
    /// value field by field, then check `is_empty()`.
    pub fn take<T: Default>(&mut self, field: &'static str, parsed: Result<T, E>) -> T {
        self.keep(field, parsed).unwrap_or_default()
    }

    /// The field's (first) error, if it didn't parse.
    pub fn get(&self, field: &str) -> Option<&E> {
        self.0.iter().find(|(f, _)| *f == field).map(|(_, e)| e)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The fields that didn't parse and why, in the form's order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &E)> {
        self.0.iter().map(|(f, e)| (*f, e))
    }
}

impl<E: std::fmt::Display> FormErrors<E> {
    /// The field's message for the page; "" when it parsed.
    pub fn message(&self, field: &str) -> String {
        self.get(field).map(ToString::to_string).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            base: "/events".into(),
            origin: "https://example.com".into(),
            path: "/".into(),
            method: "GET".into(),
            params: vec![],
            query: vec![],
            form: vec![],
            htmx: false,
            now: "2026-11-11T14:05:09+01:00".into(),
        }
    }

    #[test]
    fn the_request_time_is_the_shells_clock() {
        let now = request().now();
        assert_eq!(now.to_string(), "2026-11-11T14:05:09+01:00");
        assert_eq!(now.clock(), (14, 5, 9));
    }

    #[test]
    fn dispatch_answers_a_request_time_that_isnt_one() {
        let mut req = request();
        assert!(checked_time(&req).is_ok());
        req.now = "yesterday".into();
        assert_eq!(
            checked_time(&req).unwrap_err(),
            "the request's time \"yesterday\" is not RFC 3339"
        );
    }

    #[test]
    fn modules_dispatch_by_method_and_path() {
        const A: &[Route] = &[Route::get("/", |_| {}), Route::post("/save", |_| {})];
        const B: &[Route] = &[Route::get("/b", |_| {})];
        let modules = [
            Module {
                routes: A,
                menu: &[("A", "/")],
            },
            Module {
                routes: B,
                menu: &[],
            },
        ];
        assert_eq!(find_route(&modules, "POST /save").unwrap().path, "/save");
        assert_eq!(find_route(&modules, "GET /b").unwrap().path, "/b");
        assert!(find_route(&modules, "GET /save").is_none());
    }

    #[test]
    fn form_errors_keep_every_field_in_order() {
        let mut errors = FormErrors::default();
        assert_eq!(errors.keep("a", Ok::<u32, &str>(1)), Some(1));
        assert_eq!(errors.keep("b", Err::<u32, _>("b is missing")), None);
        assert_eq!(errors.take("c", Err::<String, _>("c is too long")), "");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors.message("b"), "b is missing");
        assert_eq!(errors.message("a"), "");
        let fields: Vec<_> = errors.iter().map(|(field, _)| field).collect();
        assert_eq!(fields, ["b", "c"]);
        errors.add("b", "b is taken");
        assert_eq!(errors.message("b"), "b is missing", "the first error shows");
    }

    #[test]
    fn route_handlers_match_the_whole_method_and_path() {
        let route = Route::post("/items/:id", |_| {});
        assert!(route.matches(&route.name()));
        for handler in [
            "GET /items/:id",
            "POST/items/:id",
            "POST  /items/:id",
            "POST /items/:id/more",
            "POST /items",
            "",
        ] {
            assert!(!route.matches(handler), "{handler:?}");
        }
        // Matching still agrees with registration for manually built routes.
        let custom = Route {
            method: "CUSTOM METHOD",
            path: "/with space",
            handler: |_| {},
        };
        assert!(custom.matches(&custom.name()));
    }

    #[test]
    fn urls_remove_only_one_leading_slash_and_preserve_query_only_paths() {
        let req = request();
        for (path, expected) in [
            ("", "/events"),
            ("/", "/events"),
            ("desk", "/events/desk"),
            ("/desk", "/events/desk"),
            ("//desk", "/events//desk"),
            ("?q=a", "/events?q=a"),
            ("/?q=a", "/events?q=a"),
            ("/café", "/events/café"),
        ] {
            assert_eq!(req.url(path), expected);
            assert_eq!(
                req.absolute_url(path),
                format!("https://example.com{expected}")
            );
        }
    }

    #[test]
    fn queries_preserve_order_duplicates_and_empty_values_and_encode_delimiters() {
        let req = request();
        assert_eq!(
            req.url_with(
                "/",
                &[("q", "a b&c=d"), ("q", "café"), ("+", ""), ("", "#%")]
            ),
            "/events?q=a%20b%26c%3Dd&q=caf%C3%A9&%2B=&=%23%25"
        );
        assert_eq!(req.url_with("/", &[]), "/events?");
    }

    #[test]
    fn percent_encoding_handles_utf8_controls_and_unreserved_characters() {
        assert_eq!(encode("AZaz09-_.~"), "AZaz09-_.~");
        assert_eq!(
            encode("\0\n\r\t /+&=?#%"),
            "%00%0A%0D%09%20%2F%2B%26%3D%3F%23%25"
        );
        assert_eq!(encode("café 🦀"), "caf%C3%A9%20%F0%9F%A6%80");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn html_escaping_preserves_unicode_and_escapes_existing_entities_once() {
        assert_eq!(
            escape("<&>\"' café 🦀 &amp;"),
            "&lt;&amp;&gt;&quot;&#39; café 🦀 &amp;amp;"
        );
        assert_eq!(escape("plain"), "plain");
        assert_eq!(escape(""), "");
    }
}
