//! What the log is narrowed to, as a query string. The admin's log and
//! `/_platform/log` (so `appos log`) take the same: `q` (text in
//! the message, reason, version or app id), `app` (`_platform` for the
//! platform's own events), `kind` (`failures`, `changes` or a kind),
//! `severity` (this bad or worse), `by` (`shell`, `cli`, `admin`), `from`
//! and `to` (days on the shell's clock, both included), and `before` (an
//! event id) for the next page.

use chrono::NaiveDate;

use crate::platform::{Actor, Kind, LogQuery, Severity};

/// The app filter's value for the platform's own events. App ids have no
/// underscore, so no app can be called that.
pub const PLATFORM: &str = "_platform";
/// The failures among the kinds: what the "status" of an event is.
pub const FAILURES: [Kind; 3] = [Kind::Refused, Kind::AutoRolledBack, Kind::LimitHit];
/// The changes to what runs: what the admin's charts mark. Refused and
/// held deploys never served traffic.
pub const CHANGES: [Kind; 5] = [Kind::Deployed, Kind::AutoRolledBack, Kind::RolledBack, Kind::TurnedOff, Kind::TurnedOn];

/// Which kinds the Event filter picks: the failures, the changes the
/// charts mark, or one kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pick {
    Failures,
    Changes,
    One(Kind),
}

impl Pick {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "failures" => Some(Pick::Failures),
            "changes" => Some(Pick::Changes),
            k => Kind::parse(k).map(Pick::One),
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Pick::Failures => "failures",
            Pick::Changes => "changes",
            Pick::One(k) => k.as_str(),
        }
    }

    pub fn kinds(self) -> Vec<Kind> {
        match self {
            Pick::Failures => FAILURES.to_vec(),
            Pick::Changes => CHANGES.to_vec(),
            Pick::One(k) => vec![k],
        }
    }
}

/// What the log is narrowed to, as the form sends it; `None` is "any".
#[derive(Debug, Default, PartialEq)]
pub struct Filters {
    /// In the message, the reason, the version or the app's id.
    pub text: Option<String>,
    /// "" is the platform itself.
    pub app: Option<String>,
    pub pick: Option<Pick>,
    /// This bad or worse.
    pub severity: Option<Severity>,
    pub by: Option<Actor>,
    /// Days on the shell's clock, both included.
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

impl Filters {
    /// The filters and the page from the query. The form sends every field,
    /// empty for "any"; a value it can't send is an error.
    pub fn parse(query: &[(String, String)]) -> Result<(Self, Option<i64>), String> {
        let mut f = Filters::default();
        let mut before = None;
        let date = |v: &str| NaiveDate::parse_from_str(v, "%Y-%m-%d").map_err(|_| format!("{v:?} is not a date like 2026-11-13"));
        for (key, value) in query {
            let v = value.trim();
            if v.is_empty() {
                continue;
            }
            match key.as_str() {
                "q" => f.text = Some(v.to_string()),
                "app" => f.app = Some(if v == PLATFORM { String::new() } else { v.to_string() }),
                "kind" => f.pick = Some(Pick::parse(v).ok_or_else(|| format!("kind {v:?} is not failures, changes or an event kind"))?),
                "severity" => f.severity = Some(Severity::parse(v).ok_or_else(|| format!("severity {v:?} is not info, low, medium, high or critical"))?),
                "by" => f.by = Some(Actor::parse(v).ok_or_else(|| format!("by {v:?} is not shell, cli or admin"))?),
                "from" => f.from = Some(date(v)?),
                "to" => f.to = Some(date(v)?),
                "before" => before = Some(v.parse().map_err(|_| format!("before {v:?} is not an event id"))?),
                _ => {}
            }
        }
        Ok((f, before))
    }

    pub fn query(&self, before: Option<i64>) -> LogQuery {
        LogQuery {
            app_id: self.app.clone(),
            before,
            from: self.from.and_then(midnight),
            until: self.to.and_then(|d| d.succ_opt()).and_then(midnight),
            kinds: self.pick.map(Pick::kinds).unwrap_or_default(),
            min_severity: self.severity,
            actor: self.by,
            text: self.text.clone(),
        }
    }

    /// `path` with these filters as its query, at the page before `before`.
    pub fn at(&self, path: &str, before: Option<i64>) -> String {
        let mut q = form_urlencoded::Serializer::new(String::new());
        let pairs = [
            ("q", self.text.clone()),
            ("app", self.app.as_ref().map(|a| if a.is_empty() { PLATFORM.to_string() } else { a.clone() })),
            ("kind", self.pick.map(|p| p.value().to_string())),
            ("severity", self.severity.map(|s| s.as_str().to_string())),
            ("by", self.by.map(|a| a.as_str().to_string())),
            ("from", self.from.map(|d| d.to_string())),
            ("to", self.to.map(|d| d.to_string())),
            ("before", before.map(|b| b.to_string())),
        ];
        for (key, value) in pairs {
            if let Some(v) = value {
                q.append_pair(key, &v);
            }
        }
        match q.finish() {
            q if q.is_empty() => path.to_string(),
            q => format!("{path}?{q}"),
        }
    }
}

/// The day's start on the shell's clock, in unix ms.
fn midnight(day: NaiveDate) -> Option<i64> {
    day.and_hms_opt(0, 0, 0)?.and_local_timezone(chrono::Local).earliest().map(|t| t.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(q: &str) -> Result<(Filters, Option<i64>), String> {
        Filters::parse(&crate::web::pairs(Some(q)))
    }

    /// What the form sends comes back as the same query, without the empty
    /// fields; a value the form can't send is refused.
    #[test]
    fn filters_make_the_query_they_came_from() {
        let (f, before) = parse("q=+consent+&app=_platform&kind=failures&severity=medium&by=admin&from=2026-11-01&to=2026-11-13&before=42&limit=5").unwrap();
        assert_eq!(f.text.as_deref(), Some("consent"));
        assert_eq!(f.app.as_deref(), Some(""));
        assert_eq!(before, Some(42));
        assert_eq!(f.at("/x", before), "/x?q=consent&app=_platform&kind=failures&severity=medium&by=admin&from=2026-11-01&to=2026-11-13&before=42");
        assert_eq!(parse("q=&app=&kind=&severity=&by=&from=&to=").unwrap(), (Filters::default(), None));
        assert_eq!(Filters::default().at("/x", None), "/x");
        for bad in ["kind=deploy", "severity=bad", "by=me", "from=13.11.2026", "before=x"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    /// "To" takes in the whole day; "failures" and "changes" are kinds.
    #[test]
    fn filters_narrow_the_query() {
        let (f, _) = parse("kind=failures&from=2026-11-13&to=2026-11-13&app=leads").unwrap();
        let q = f.query(None);
        assert_eq!(q.until.unwrap() - q.from.unwrap(), 24 * 3_600_000);
        assert_eq!(q.kinds, FAILURES);
        assert_eq!(q.app_id.as_deref(), Some("leads"));
        assert_eq!(Pick::parse("changes").unwrap().kinds(), CHANGES);
        assert_eq!(Pick::parse("held"), Some(Pick::One(Kind::Held)));
    }
}
