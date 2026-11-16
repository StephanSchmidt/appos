//! The numbers beside the apps, the governance set: apps live, deploys,
//! what the platform stopped, and how fast it recovered. Each tile has the
//! last 30 days as bars, a bar per 24 hours; the newest bar, on the right,
//! is the big number. Requests and failures are the charts' job.

use std::cmp::Ordering;

use crate::platform::{Event, HOUR_MS, Hour, Kind};
use crate::shell::Shell;
use crate::web::escape;

use super::format::{local, plural, span, thousands};
use super::log::of_kinds;
use super::model::App;
use crate::log_filter::Pick;

/// Bars per tile, each 24 hours.
const BARS: usize = 30;
const DAY_MS: i64 = 24 * HOUR_MS;
/// The bars' height in px.
const PLOT: f64 = 28.0;
/// The events the tiles count.
const KINDS: [Kind; 3] = [Kind::Deployed, Kind::Refused, Kind::AutoRolledBack];

/// The last 30 days, read once per page.
pub(super) struct Month {
    /// The current hour, unix ms / HOUR_MS; the newest bar ends with it.
    now_hour: i64,
    hours: Vec<Hour>,
    events: Vec<Event>,
    first_deploys: Vec<(String, i64)>,
}

type Bars = [u64; BARS];

impl Month {
    pub(super) fn load(shell: &Shell, now: i64) -> Self {
        let now_hour = now.div_euclid(HOUR_MS);
        let since = (now_hour + 1) * HOUR_MS - BARS as i64 * DAY_MS;
        Month {
            now_hour,
            hours: shell.platform.hours(None, since).unwrap_or_default(),
            events: shell.platform.events_since(None, since, &KINDS).unwrap_or_default(),
            first_deploys: shell.platform.first_deploys().unwrap_or_default(),
        }
    }

    /// The bar an hour falls in, the oldest 0.
    fn bar(&self, hour: i64) -> Option<usize> {
        let back = (self.now_hour - hour).div_euclid(24);
        (0..BARS as i64).contains(&back).then(|| BARS - 1 - back as usize)
    }

    /// When a bar ends, unix ms; the newest ends with the current hour.
    fn end(&self, bar: usize) -> i64 {
        (self.now_hour + 1) * HOUR_MS - (BARS - 1 - bar) as i64 * DAY_MS
    }

    fn events_of(&self, kind: Kind) -> impl Iterator<Item = (usize, &Event)> {
        self.events.iter().filter(move |e| e.kind == kind).filter_map(|e| Some((self.bar(e.at.div_euclid(HOUR_MS))?, e)))
    }

    fn count(&self, kind: Kind) -> Bars {
        let mut out = [0; BARS];
        for (i, _) in self.events_of(kind) {
            out[i] += 1;
        }
        out
    }

    /// Requests stopped at a limit.
    fn stops(&self) -> Bars {
        let mut out = [0; BARS];
        for h in &self.hours {
            if let Some(i) = self.bar(h.hour) {
                out[i] += h.stops;
            }
        }
        out
    }

    /// Apps live at the end of each bar: first deployed by then; the newest
    /// bar is what runs now.
    fn live(&self, running: usize) -> Bars {
        let mut out = [0; BARS];
        for (i, n) in out.iter_mut().enumerate() {
            *n = self.first_deploys.iter().filter(|(_, at)| *at < self.end(i)).count() as u64;
        }
        out[BARS - 1] = running as u64;
        out
    }
}

/// A tile: its big number, one line of context, and its bars with what
/// each says.
struct Tile {
    label: &'static str,
    value: String,
    context: String,
    /// The newest bar against the one a week before.
    delta: Option<i64>,
    bars: Bars,
    tips: Vec<String>,
    /// The bars for screen readers.
    aria: String,
    /// The events behind it, in the log.
    log: Pick,
}

fn week_delta(bars: &Bars) -> Option<i64> {
    Some(bars[BARS - 1] as i64 - bars[BARS - 8] as i64)
}

fn apps_live(m: &Month, apps: &[App]) -> Tile {
    // Turned off counts as not live, though it is loaded.
    let running = apps.iter().filter(|a| a.running.is_some() && a.off.is_none()).count();
    let off = apps.iter().filter(|a| a.off.is_some()).count();
    let bars = m.live(running);
    let context = match (off, apps.len().saturating_sub(running + off)) {
        (0, 0) => "running now".to_string(),
        (0, n) => format!("running now, {n} more not running"),
        (o, 0) => format!("running now, {o} turned off"),
        (o, n) => format!("running now, {o} turned off, {n} more not running"),
    };
    Tile {
        label: "Apps live",
        log: Pick::Changes,
        value: thousands(running as u64),
        context,
        delta: week_delta(&bars),
        tips: bars.iter().map(|n| format!("{} live", plural(*n as usize, "app"))).collect(),
        aria: format!("{} live 30 days ago, {running} now", bars[0]),
        bars,
    }
}

fn deploys(m: &Month) -> Tile {
    let bars = m.count(Kind::Deployed);
    let total: u64 = bars.iter().sum();
    Tile {
        label: "Deploys",
        log: Pick::One(Kind::Deployed),
        value: thousands(bars[BARS - 1]),
        context: "in the last 24 hours".to_string(),
        delta: week_delta(&bars),
        tips: bars.iter().map(|n| plural(*n as usize, "deploy")).collect(),
        aria: format!("{} in the last 30 days", plural(total as usize, "deploy")),
        bars,
    }
}

/// "2 refused deploys, 14 limit stops", or None when nothing was stopped.
fn stopped_text(refused: u64, stops: u64, rollbacks: u64) -> Option<String> {
    let parts: Vec<String> = [(refused, "refused deploy"), (stops, "limit stop"), (rollbacks, "automatic rollback")]
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, what)| plural(*n as usize, what))
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn stopped(m: &Month) -> Tile {
    let (refused, stops, rollbacks) = (m.count(Kind::Refused), m.stops(), m.count(Kind::AutoRolledBack));
    let mut bars = [0; BARS];
    let mut tips = vec![];
    for i in 0..BARS {
        bars[i] = refused[i] + stops[i] + rollbacks[i];
        tips.push(match stopped_text(refused[i], stops[i], rollbacks[i]) {
            Some(what) => format!("{}: {what}", thousands(bars[i])),
            None => "nothing stopped".to_string(),
        });
    }
    let last = BARS - 1;
    let context = match stopped_text(refused[last], stops[last], rollbacks[last]) {
        Some(what) => format!("in the last 24 hours: {what}"),
        None => "nothing in the last 24 hours".to_string(),
    };
    let sum = |b: &Bars| b.iter().sum::<u64>() as usize;
    let aria = stopped_text(sum(&refused) as u64, sum(&stops) as u64, sum(&rollbacks) as u64)
        .map_or("Nothing stopped in the last 30 days".to_string(), |what| format!("In the last 30 days: {what}"));
    Tile { label: "Stopped by the platform", log: Pick::Failures, value: thousands(bars[last]), context, delta: week_delta(&bars), bars, tips, aria }
}

/// The middle value; between the two middle ones for an even count.
fn median(mut v: Vec<i64>) -> Option<i64> {
    v.sort_unstable();
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2]),
        _ => Some((v[n / 2 - 1] + v[n / 2]) / 2),
    }
}

fn recovery(m: &Month) -> Tile {
    let (mut bars, mut counts) = ([0; BARS], [0; BARS]);
    let mut all = vec![];
    for (i, e) in m.events_of(Kind::AutoRolledBack) {
        if let Some(ms) = e.recovery_ms {
            bars[i] = bars[i].max(ms.max(0) as u64);
            counts[i] += 1;
            all.push(ms.max(0));
        }
    }
    let tips = bars
        .iter()
        .zip(counts)
        .map(|(ms, n)| match n {
            0 => "no automatic rollback".to_string(),
            1 => format!("back in {}", span(*ms as i64)),
            _ => format!("{n} automatic rollbacks, the longest back in {}", span(*ms as i64)),
        })
        .collect();
    let n = all.len();
    let longest = all.iter().max().copied();
    let (value, context) = match median(all) {
        Some(ms) => (span(ms), format!("from the first failure to rolled back, the median of {} in 30 days", plural(n, "automatic rollback"))),
        None => ("—".to_string(), "no automatic rollbacks in 30 days".to_string()),
    };
    Tile {
        label: "Time to recovery",
        log: Pick::One(Kind::AutoRolledBack),
        value,
        context,
        delta: None,
        tips,
        aria: longest.map_or("No automatic rollbacks in the last 30 days".to_string(), |ms| format!("The longest in the last 30 days: {}", span(ms))),
        bars,
    }
}

fn delta_html(d: i64) -> String {
    let (sign, word) = match d.cmp(&0) {
        Ordering::Greater => ("▲", "up"),
        Ordering::Less => ("▼", "down"),
        Ordering::Equal => return r#"<span class="delta">same as a week ago</span>"#.to_string(),
    };
    format!(
        r#"<span class="delta"><span aria-hidden="true">{sign}</span><span class="sr-only">{word}</span> {} vs a week ago</span>"#,
        thousands(d.unsigned_abs())
    )
}

fn tile_html(m: &Month, t: &Tile) -> String {
    let max = t.bars.iter().max().copied().unwrap_or(0).max(1);
    let bars: String = t
        .bars
        .iter()
        .zip(&t.tips)
        .enumerate()
        .map(|(i, (n, tip))| {
            let newest = i == BARS - 1;
            let when = match newest {
                true => "Last 24 hours".to_string(),
                false => format!("{} to {}", local(m.end(i) - DAY_MS, "%b %-d %H:%M"), local(m.end(i), "%b %-d %H:%M")),
            };
            let px = if *n == 0 { 0.0 } else { (PLOT * *n as f64 / max as f64).round().max(2.0) };
            format!(
                r#"<span{} title="{}"><i style="height:{px}px"></i></span>"#,
                if newest { r#" class="now""# } else { "" },
                escape(&format!("{when} · {tip}"))
            )
        })
        .collect();
    format!(
        r#"<a class="kpi" href="{}"><span class="label">{}</span><span class="figure"><b>{}</b>{}</span><span class="context">{}</span><div class="spark" role="img" aria-label="{}">{bars}</div></a>"#,
        escape(&of_kinds(t.log)),
        t.label,
        escape(&t.value),
        t.delta.map(delta_html).unwrap_or_default(),
        escape(&t.context),
        escape(&t.aria),
    )
}

pub(super) fn kpis(apps: &[App], m: &Month) -> String {
    let html: String = [apps_live(m, apps), deploys(m), stopped(m), recovery(m)].iter().map(|t| tile_html(m, t)).collect();
    format!(r#"<section class="card kpis" aria-label="The platform, last 30 days">{html}</section>"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Actor, Severity};

    fn month(now_hour: i64) -> Month {
        Month { now_hour, hours: vec![], events: vec![], first_deploys: vec![] }
    }

    fn event(kind: Kind, at: i64, recovery_ms: Option<i64>) -> Event {
        Event {
            id: 0,
            at,
            app_id: "a".into(),
            label: "l".into(),
            kind,
            severity: Severity::Info,
            actor: Actor::Cli,
            message: String::new(),
            reason: None,
            risk: None,
            recovery_ms,
            outcome: None,
            before: None,
            person: None,
        }
    }

    /// The newest bar is the current hour and the 23 before it.
    #[test]
    fn bars_are_24_hours_ending_with_the_current_hour() {
        let m = month(1000);
        assert_eq!(m.bar(1000), Some(BARS - 1));
        assert_eq!(m.bar(977), Some(BARS - 1));
        assert_eq!(m.bar(976), Some(BARS - 2));
        assert_eq!(m.bar(1000 - 24 * 30 + 1), Some(0));
        assert_eq!(m.bar(1000 - 24 * 30), None);
        assert_eq!(m.bar(1001), None);
        assert_eq!(m.end(BARS - 1), 1001 * HOUR_MS);
        assert_eq!(m.end(BARS - 2), 977 * HOUR_MS);
    }

    /// Refused deploys, limit stops and automatic rollbacks add up.
    #[test]
    fn stopped_adds_up_and_says_what() {
        let mut m = month(1000);
        let at = 1000 * HOUR_MS;
        m.events = vec![event(Kind::Refused, at, None), event(Kind::AutoRolledBack, at - DAY_MS, Some(40_000)), event(Kind::Deployed, at, None)];
        m.hours = vec![Hour { hour: 990, stops: 7 }];
        let t = stopped(&m);
        assert_eq!((t.value.as_str(), t.bars[BARS - 1], t.bars[BARS - 2]), ("8", 8, 1));
        assert_eq!(t.context, "in the last 24 hours: 1 refused deploy, 7 limit stops");
        assert_eq!(t.tips[BARS - 2], "1: 1 automatic rollback");
        assert_eq!(t.delta, Some(8));
        assert_eq!(deploys(&m).value, "1");
    }

    #[test]
    fn recovery_is_the_median() {
        let mut m = month(1000);
        assert_eq!(recovery(&m).value, "—");
        let at = 1000 * HOUR_MS;
        m.events = [41_000, 9_000, 125_000].map(|ms| event(Kind::AutoRolledBack, at, Some(ms))).to_vec();
        let t = recovery(&m);
        assert_eq!(t.value, "41 s");
        assert_eq!(t.tips[BARS - 1], "3 automatic rollbacks, the longest back in 2 min 5 s");
        assert_eq!(t.tips[BARS - 2], "no automatic rollback");
        assert_eq!(median(vec![1, 2, 3, 10]), Some(2));
    }
}
