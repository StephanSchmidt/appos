//! The charts: requests and failures per minute with a line where a
//! version went live or the app was turned off, and every app's last hour
//! as a heat strip. Plain HTML and CSS, no scripts.

use std::collections::BTreeMap;

use crate::platform::{Event, Kind, Request};
use crate::web::escape;

use super::format::{duration, hm, plural};
use super::log::from_event;
use super::model::{App, HOUR, MINUTE, by_kind, kinds_text, name_of, sentence};
use super::style::{CAUSES, FAILED, HEAT, HEAT_FAILED, KINDS, LIVE_PATH, POWER_PATH, ROLLBACK_PATH, SERVED, icon_html, path_icon};

/// The charts: the last 30 minutes, a column a minute, this many px high.
pub(super) const MINUTES: i64 = 30;
pub(super) const PLOT: f64 = 128.0;
/// The heat strips: the last hour in cells of 2½ minutes.
pub(super) const CELLS: i64 = 24;
pub(super) const CELL: i64 = HOUR / CELLS;

/// A column of a chart: its segments (count, color), top first, and its tooltip.
pub(super) type Column = (Vec<(u32, &'static str)>, String);

/// A chart's series: name and colour, bottom of a stack first.
type Series = [(&'static str, &'static str)];

/// The column of a time: minutes of the clock, the newest (this one, so
/// far) last; none when it is older than the chart.
pub(super) fn column(at: i64, now: i64) -> Option<usize> {
    let back = now.div_euclid(MINUTE) - at.div_euclid(MINUTE);
    (0..MINUTES).contains(&back).then(|| (MINUTES - 1 - back) as usize)
}

/// The requests of the last 30 minutes, a column a minute, oldest first.
pub(super) fn minutes(apps: &[App], now: i64) -> Vec<Vec<&Request>> {
    let mut cols = vec![vec![]; MINUTES as usize];
    for r in apps.iter().flat_map(|a| &a.reqs) {
        if let Some(i) = column(r.at, now) {
            cols[i].push(r);
        }
    }
    cols
}

pub(super) fn minute_label(now: i64, col: i64) -> String {
    hm(now - (MINUTES - 1 - col) * MINUTE)
}

/// The changes that get a line on the charts. Refused and held deploys
/// never served traffic, so they get none (nor do they on Cloudflare's).
pub(super) const MARKED: [Kind; 5] = crate::log_filter::CHANGES;

/// How a change looks on the charts, the strongest first: changes in the
/// same minute share a line, which looks like the strongest of them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Look {
    AutoRollback,
    Switch,
    Rollback,
    Deploy,
}

impl Look {
    /// In the legend, in the order things happen.
    const LEGEND: [Look; 4] = [Look::Deploy, Look::AutoRollback, Look::Rollback, Look::Switch];

    fn of(kind: Kind) -> Look {
        match kind {
            Kind::AutoRolledBack => Look::AutoRollback,
            Kind::TurnedOff | Kind::TurnedOn => Look::Switch,
            Kind::RolledBack => Look::Rollback,
            _ => Look::Deploy,
        }
    }

    /// Its classes, its icon and its words in the legend. The switch's
    /// line is dashed: after "off", nothing ran.
    fn parts(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Look::AutoRollback => ("tone-critical", ROLLBACK_PATH, "Rolled back automatically"),
            Look::Switch => ("tone-neutral dashed", POWER_PATH, "Turned off or on"),
            Look::Rollback => ("tone-neutral", ROLLBACK_PATH, "Rolled back by hand"),
            Look::Deploy => ("tone-info", LIVE_PATH, "Deployed"),
        }
    }

    fn flag(self) -> String {
        format!(r#"<span class="flag">{}</span>"#, path_icon(self.parts().1, 12))
    }
}

/// A line on the charts: the column of its minute, its look, and what
/// happened, a line of text per change.
pub(super) struct Mark {
    col: usize,
    look: Look,
    tip: String,
    /// The log from the newest of its changes down.
    log: String,
}

/// The changes of the last 30 minutes, one mark per minute; `app` when
/// they are one app's, so its flags open that app's log.
pub(super) fn marks<'a>(events: impl IntoIterator<Item = &'a Event>, apps: &[App], app: Option<&str>, now: i64) -> Vec<Mark> {
    let mut cols: BTreeMap<usize, Vec<&Event>> = BTreeMap::new();
    for e in events.into_iter().filter(|e| MARKED.contains(&e.kind)) {
        if let Some(i) = column(e.at, now) {
            cols.entry(i).or_default().push(e);
        }
    }
    cols.into_iter()
        .map(|(col, mut changes)| {
            changes.sort_by_key(|e| e.at);
            let look = changes.iter().map(|e| Look::of(e.kind)).min().unwrap_or(Look::Deploy);
            let tip: Vec<String> = changes.iter().map(|e| format!("{} · {}", hm(e.at), change(e, apps))).collect();
            let newest = changes.iter().map(|e| e.id).max().unwrap_or_default();
            Mark { col, look, tip: tip.join("\n"), log: from_event(newest, app) }
        })
        .collect()
}

/// A change in words, like "Calculator 0.2.0 deployed, live in 12 ms".
fn change(e: &Event, apps: &[App]) -> String {
    sentence(e, Some(name_of(apps, &e.app_id)))
}

pub(super) fn requests_chart(apps: &[App], marks: &[Mark], now: i64) -> String {
    card("req", REQUESTS, &requests_plot(apps, marks, now))
}

/// One app's requests with its own changes, in its detail: a bad deploy,
/// its failures, the rollback and the recovery read as one picture.
pub(super) fn app_chart(app: &App, marks: &[Mark], now: i64) -> String {
    format!(
        r#"<section class="part graph" aria-labelledby="ar-h"><h4 id="ar-h">{REQUESTS} <small>· last 30 minutes</small></h4>{}</section>"#,
        requests_plot(std::slice::from_ref(app), marks, now)
    )
}

const REQUESTS: &str = "Requests per minute";

fn requests_plot(apps: &[App], marks: &[Mark], now: i64) -> String {
    let cols: Vec<Column> = minutes(apps, now)
        .iter()
        .enumerate()
        .map(|(i, reqs)| {
            let failures = reqs.iter().filter(|r| r.outcome.failed()).count();
            let mut tip = format!("{} · {}", minute_label(now, i as i64), plural(reqs.len(), "request"));
            if failures > 0 {
                tip += &format!(", {failures} failed");
            }
            (vec![(failures as u32, FAILED), ((reqs.len() - failures) as u32, SERVED)], tip)
        })
        .collect();
    plot(REQUESTS, &[("Served", SERVED), ("Failed", FAILED)], &cols, marks, "No requests in the last 30 minutes.", now)
}

pub(super) fn failures_chart(apps: &[App], marks: &[Mark], now: i64) -> String {
    let cols: Vec<Column> = minutes(apps, now)
        .iter()
        .enumerate()
        .map(|(i, reqs)| {
            let counts = by_kind(reqs);
            let what = if counts.iter().all(|n| *n == 0) { "no failures".to_string() } else { kinds_text(&counts) };
            let mut per_cause = [0; CAUSES.len()];
            for ((_, _, cause), n) in KINDS.iter().zip(&counts) {
                per_cause[*cause] += n;
            }
            let segments = CAUSES.iter().zip(per_cause).rev().map(|((_, color), n)| (n, *color)).collect();
            (segments, format!("{} · {what}", minute_label(now, i as i64)))
        })
        .collect();
    let title = "Failures per minute, by cause";
    card("fail", title, &plot(title, &CAUSES, &cols, marks, "No failures in the last 30 minutes.", now))
}

/// A chart as a card of its own, on the overview.
fn card(id: &str, title: &str, plot: &str) -> String {
    format!(r#"<section class="card chart" aria-labelledby="{id}-h"><h2 id="{id}-h">{title} <small>· last 30 minutes</small></h2>{plot}</section>"#)
}

/// The axis top: the next of 2, 4, 6, 8 or 10 times a power of ten.
pub(super) fn nice(max: u32) -> u32 {
    let mut p = 1;
    while p * 10 < max {
        p *= 10;
    }
    [2, 4, 6, 8, 10].iter().map(|f| f * p).find(|v| *v >= max).unwrap_or(10 * p)
}

/// A stacked column chart with the changes as lines, its numbers for screen
/// readers and its legend; `empty` is said over it when every column is 0.
fn plot(title: &str, series: &Series, cols: &[Column], marks: &[Mark], empty: &str, now: i64) -> String {
    let max = cols.iter().map(|(segs, _)| segs.iter().map(|s| s.0).sum::<u32>()).max().unwrap_or(0);
    let top = nice(max);
    let px = |n: u32| (PLOT * n as f64 / top as f64).round().max(3.0);
    let bars: String = cols
        .iter()
        .map(|(segs, tip)| {
            let segs: String =
                segs.iter().filter(|(n, _)| *n > 0).map(|(n, color)| format!(r#"<i style="height:{}px;background:{color}"></i>"#, px(*n))).collect();
            format!(r#"<div title="{}">{segs}</div>"#, escape(tip))
        })
        .collect();
    let ticks: String = (0..MINUTES).step_by(5).chain([MINUTES - 1]).map(|i| format!("<span>{}</span>", minute_label(now, i))).collect();
    // A change goes in the gap before the column of its minute: column i
    // starts at i × (width + gap) / 30.
    let lines: String = marks
        .iter()
        .map(|m| {
            format!(
                r#"<span class="mk {}" style="left:calc((100% + 4px) * {} / {MINUTES} - 2px)" title="{}"><a class="flag" href="{}" aria-label="{}">{}</a></span>"#,
                m.look.parts().0,
                m.col,
                escape(&m.tip),
                escape(&m.log),
                escape(&m.tip.replace('\n', "; ")),
                path_icon(m.look.parts().1, 12),
            )
        })
        .collect();
    let looks = Look::LEGEND.into_iter().filter(|l| marks.iter().any(|m| m.look == *l));
    let legend: String = series
        .iter()
        .map(|(name, color)| format!(r#"<span><i class="sw" style="background:{color}"></i>{name}</span>"#))
        .chain(looks.map(|l| format!(r#"<span class="{}">{}{}</span>"#, l.parts().0, l.flag(), l.parts().2)))
        .collect();
    // The numbers for screen readers; segments are top first, series bottom first.
    let changes = if marks.is_empty() { "" } else { r#"<th scope="col">Changes</th>"# };
    let head: String = series.iter().map(|(name, _)| format!(r#"<th scope="col">{name}</th>"#)).collect::<String>() + changes;
    let rows: String = cols
        .iter()
        .enumerate()
        .map(|(i, (segs, _))| {
            let mut cells: String = segs.iter().rev().map(|(n, _)| format!("<td>{n}</td>")).collect();
            if !marks.is_empty() {
                let tip = marks.iter().find(|m| m.col == i).map_or("", |m| m.tip.as_str());
                cells += &format!("<td>{}</td>", escape(tip));
            }
            format!(r#"<tr><th scope="row">{}</th>{cells}</tr>"#, minute_label(now, i as i64))
        })
        .collect();
    let empty = if max == 0 { format!(r#"<p class="nodata"><span>{empty}</span></p>"#) } else { String::new() };
    format!(
        r#"<div class="stack">{empty}<div class="plot" aria-hidden="true"><div class="yaxis"><span>{top}</span><span>{half}</span><span>0</span></div>
<div class="bars"><div class="cols">{bars}</div><div class="xaxis">{ticks}</div>{lines}</div></div></div>
<div class="sr-only"><table><caption>{title}, last 30 minutes</caption><thead><tr><th scope="col">Minute</th>{head}</tr></thead><tbody>{rows}</tbody></table></div>
<div class="legend" aria-hidden="true">{legend}</div>"#,
        half = top / 2,
    )
}

/// Which share of the time limit the slowest request took, as a HEAT step.
pub(super) fn level(slowest: u64, limit: u64) -> usize {
    let share = slowest as f64 / limit.max(1) as f64;
    [0.1, 0.25, 0.45, 0.65, 0.85].iter().position(|t| share < *t).unwrap_or(HEAT.len() - 1)
}

/// Every app's last hour as a strip, next to the app in detail.
pub(super) fn heat(apps: &[App], chosen: &str, now: i64) -> String {
    let rows: String = apps
        .iter()
        .map(|a| {
            let limit = a.limit_us();
            let mut cells: Vec<Vec<&Request>> = vec![vec![]; CELLS as usize];
            for r in &a.reqs {
                let back = (now - r.at) / CELL;
                if (0..CELLS).contains(&back) {
                    cells[(CELLS - 1 - back) as usize].push(r);
                }
            }
            let strip: String = cells
                .iter()
                .enumerate()
                .map(|(i, reqs)| {
                    let at = hm(now - (CELLS - 1 - i as i64) * CELL);
                    let counts = by_kind(reqs);
                    let failures: u32 = counts.iter().sum();
                    let (bg, tip) = if failures > 0 {
                        (HEAT_FAILED, format!("{at} · {failures} of {} failed: {}", plural(reqs.len(), "request"), kinds_text(&counts)))
                    } else if reqs.is_empty() {
                        ("var(--gray-3)", format!("{at} · no requests"))
                    } else {
                        let slowest = reqs.iter().map(|r| r.wall_us).max().unwrap_or(0);
                        let tip = format!("{at} · {}, the slowest took {} of {}", plural(reqs.len(), "request"), duration(slowest), duration(limit));
                        (HEAT[level(slowest, limit)], tip)
                    };
                    format!(r#"<i style="background:{bg}" title="{}"></i>"#, escape(&tip))
                })
                .collect();
            let on = a.id == chosen;
            format!(
                r#"<a class="hrow{}" href="/admin/apps/{id}#detail"{}><span class="hname{dim}">{icon}<span>{name}</span></span><span class="strip">{strip}</span></a>"#,
                if on { " on" } else { "" },
                if on { r#" aria-current="true""# } else { "" },
                dim = if a.problems.is_empty() && a.running.is_none() { " dim" } else { "" },
                id = escape(&a.id),
                icon = icon_html(a, 24),
                name = escape(&a.name),
            )
        })
        .collect();
    let scale: String = HEAT.iter().map(|c| format!(r#"<i class="sw" style="background:{c}"></i>"#)).collect();
    format!(
        r#"<section class="card heat" aria-labelledby="heat-h"><h2 id="heat-h">Apps <small>· slowest request, last hour</small></h2>
<div class="hrow head" aria-hidden="true"><span></span><span class="ends"><span>1 h ago</span><span>now</span></span></div>
{rows}
<div class="legend"><span>Per 2½ min: fast {scale} at the limit</span><span><i class="sw" style="background:{HEAT_FAILED}"></i>Failed</span></div></section>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Actor, Severity};

    /// 10:05:30, in milliseconds since the epoch.
    const NOW: i64 = (10 * 60 + 5) * MINUTE + 30_000;

    fn event(kind: Kind, at: i64, actor: Actor, message: &str, reason: Option<&str>) -> Event {
        Event {
            id: 0,
            at,
            app_id: "calc".into(),
            label: "0.2.0".into(),
            kind,
            severity: Severity::Info,
            actor,
            message: message.into(),
            reason: reason.map(Into::into),
            risk: None,
            recovery_ms: None,
            outcome: None,
            before: None,
            person: None,
        }
    }

    /// Columns are minutes of the clock, so a change's time and its
    /// column's label agree; the newest is the minute so far.
    #[test]
    fn a_column_is_a_minute_of_the_clock() {
        assert_eq!(column(NOW, NOW), Some(29));
        assert_eq!(column(NOW - 30_000, NOW), Some(29));
        assert_eq!(column(NOW - 30_001, NOW), Some(28));
        assert_eq!(column(NOW - 29 * MINUTE - 30_000, NOW), Some(0));
        assert_eq!(column(NOW - 29 * MINUTE - 30_001, NOW), None);
        assert_eq!(column(NOW + MINUTE, NOW), None);
    }

    /// Changes in one minute share a line that looks like the strongest;
    /// refused and held deploys never served traffic and get none.
    #[test]
    fn changes_in_a_minute_share_a_line() {
        let minute = NOW - 30_000 - 3 * MINUTE;
        let events = [
            event(Kind::Deployed, minute + 5_000, Actor::Cli, "Deployed, live in 12 ms", None),
            event(Kind::AutoRolledBack, minute + 50_000, Actor::Shell, "Rolled back automatically from 0.2.0 to 0.1.0", Some("5 failed requests in 9 s")),
            event(Kind::Refused, minute - MINUTE, Actor::Cli, "Refused", Some("uses kv::set")),
            event(Kind::Held, minute - 2 * MINUTE, Actor::Cli, "Held", None),
            event(Kind::TurnedOff, NOW - 1_000, Actor::Admin, "Turned off", Some("leaks\n emails")),
        ];
        let marks = marks(&events, &[], None, NOW);
        assert_eq!(marks.len(), 2);
        assert_eq!((marks[0].col, marks[0].look), (26, Look::AutoRollback));
        let ten = |m: i64| hm((10 * 60 + m) * MINUTE);
        assert_eq!(
            marks[0].tip,
            format!("{0} · calc 0.2.0 deployed, live in 12 ms\n{0} · calc rolled back automatically from 0.2.0 to 0.1.0: 5 failed requests in 9 s", ten(2))
        );
        assert_eq!((marks[1].col, marks[1].look), (29, Look::Switch));
        assert_eq!(marks[1].tip, format!("{} · calc turned off by an admin: leaks", ten(5)));
    }

    #[test]
    fn a_rollback_by_hand_says_who() {
        let e = event(Kind::RolledBack, NOW, Actor::Cli, "0.1.0 made live again, replacing 0.2.0", None);
        assert_eq!(change(&e, &[]), "calc 0.1.0 made live again, replacing 0.2.0, with the appos CLI");
        let e = event(Kind::Deployed, NOW, Actor::Admin, "0.3.0 made live, replacing 0.2.0", Some("fixed"));
        assert_eq!(change(&e, &[]), "calc 0.3.0 made live, replacing 0.2.0, by an admin: fixed");
    }
}
