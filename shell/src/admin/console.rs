//! The console at the top of the overview: a device with a screen in the
//! middle, for the person who runs the platform that marketing, sales and
//! support ship to.
//!
//! What needs you runs through the ticker like an inbox: ◀ ▶ step through
//! it, Open tells the app's story, Seen clears it until something new
//! happens. The screen lists every app with its last 16 minutes (up to 16
//! days), needs-you first; a map shows them by group (app.toml's `group`).
//!
//! Left, browse: the App knob picks the app, Group narrows to one group,
//! Range sets how far back the rows go; List and Map are the views, the
//! list with Find in its first line.
//! Right, one column: what to do with the selected app (its story, its
//! requests, its versions), its on/off slide and its limits on three faders.
//! Bottom, the step keys pick the time the Requests page shows; its LED
//! glows orange.
//! Bottom right, the platform: the power button (Stop all, Start all) and
//! the Freeze switch. Like on a device, a control does what it shows at
//! once; each change is logged as admin and can be undone from its toast
//! for ten minutes.
//!
//! The console's state is the page's query ([`View`]), so every knob and
//! key is a link and every change a form: without scripts it is a page like
//! the others. admin.js adds wheel, drag and keyboard, the hint line, the
//! cursor, pins, the held order and swaps pages in without a reload.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use crate::manifest::{MAX_RISK, Risk};
use crate::platform::{Cell, Event, Kind, Outcome, Request, Version, VersionStatus};
use crate::runtime::{AdminLimits, Budget};
use crate::shell::Shell;
use crate::web::escape;

use super::format::{capitalize, clock, duration, first_line, hm, local, plural, span, thousands, uncapitalize};
use super::model::{App, CLOSE, DAY, HOUR, MINUTE, Problem, ProblemKind, Tone, sentence, stats, ver};
use super::style::mail_of;
use super::undo::{UNDO_MS, reverse};

/// Cells in a row: the screen's 16 columns and the 16 step keys.
pub(super) const CELLS: usize = 16;
/// The longest text the view keeps: longer than any app id, group or
/// version label, so a link to one always works.
const TEXT: usize = 256;
/// Requests the requests page lists, the newest first.
const LISTED: u32 = 200;
/// The screen's rows at once.
const ROWS: usize = 12;
/// The most lights round the App knob; past that, a light per stretch.
const LIGHTS: usize = 36;

// ---- the view: what the console shows, as the page's query -----------------

/// The screen's pages.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum Page {
    #[default]
    List,
    Story,
    Log,
    Versions,
    Map,
}

impl Page {
    const ALL: [Page; 5] = [Page::List, Page::Story, Page::Log, Page::Versions, Page::Map];

    fn name(self) -> &'static str {
        match self {
            Page::List => "list",
            Page::Story => "app",
            Page::Log => "log",
            Page::Versions => "versions",
            Page::Map => "map",
        }
    }
}

/// The list's order: a click on a column's head.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum Sort {
    #[default]
    Needs,
    Group,
    Busy,
    Failures,
    Risk,
    Newest,
    Az,
}

impl Sort {
    const ALL: [Sort; 7] = [Sort::Needs, Sort::Group, Sort::Busy, Sort::Failures, Sort::Risk, Sort::Newest, Sort::Az];

    fn name(self) -> &'static str {
        match self {
            Sort::Needs => "needs",
            Sort::Group => "group",
            Sort::Busy => "busy",
            Sort::Failures => "failures",
            Sort::Risk => "risk",
            Sort::Newest => "newest",
            Sort::Az => "az",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Sort::Needs => "Needs you",
            Sort::Group => "Group",
            Sort::Busy => "Busiest",
            Sort::Failures => "Failures",
            Sort::Risk => "Risk",
            Sort::Newest => "Newest deploy",
            Sort::Az => "A–Z",
        }
    }
}

/// RANGE's four detents: how long a cell is, so how far back a row goes.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) enum Zoom {
    #[default]
    Min,
    Five,
    Hour,
    Day,
}

impl Zoom {
    const ALL: [Zoom; 4] = [Zoom::Min, Zoom::Five, Zoom::Hour, Zoom::Day];

    fn name(self) -> &'static str {
        match self {
            Zoom::Min => "1m",
            Zoom::Five => "5m",
            Zoom::Hour => "1h",
            Zoom::Day => "1d",
        }
    }

    fn ms(self) -> i64 {
        match self {
            Zoom::Min => MINUTE,
            Zoom::Five => 5 * MINUTE,
            Zoom::Hour => HOUR,
            Zoom::Day => DAY,
        }
    }

    /// How far back the cells go: "16 min", "80 min", "16 h", "16 days".
    fn range(self) -> String {
        match CELLS as i64 * self.ms() {
            t if t < 2 * HOUR => format!("{} min", t / MINUTE),
            t if t < 2 * DAY => format!("{} h", t / HOUR),
            t => format!("{} days", t / DAY),
        }
    }

    /// A rate's unit: per minute, or per hour and day when cells are that long.
    fn per(self) -> (&'static str, i64) {
        match self {
            Zoom::Min | Zoom::Five => ("/min", MINUTE),
            Zoom::Hour => ("/h", HOUR),
            Zoom::Day => ("/d", DAY),
        }
    }

    /// A step key `back` cells before now: "Now", then bare numbers in the
    /// row's unit ("3" minutes, "15" minutes at 5m, "2" days).
    fn step(self, back: i64) -> String {
        match (back, self) {
            (0, _) => "Now".to_string(),
            (b, Zoom::Five) => (b * 5).to_string(),
            (b, _) => b.to_string(),
        }
    }
}

/// What the console shows: its page, how the list is sorted and narrowed,
/// how far back, and which version the screen shows. The selected app is
/// the page's path (`/admin/apps/{id}`).
#[derive(Clone, Default, Debug, PartialEq)]
pub(super) struct View {
    pub(super) page: Page,
    pub(super) sort: Sort,
    pub(super) zoom: Zoom,
    /// GROUP: only this group's apps.
    pub(super) group: Option<String>,
    /// FIND: apps whose name, id, owner or group has this.
    pub(super) find: Option<String>,
    /// The version the versions page shows.
    pub(super) version: Option<String>,
    /// The start of the cell whose requests the requests page shows (unix ms).
    pub(super) at: Option<i64>,
    /// The toast after a change from the console.
    pub(super) done: Option<Done>,
}

/// A change made from the console: what happened, when, and the event an
/// Undo reverses.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Done {
    pub(super) msg: String,
    pub(super) at: i64,
    pub(super) undo: Option<i64>,
    /// It didn't happen: the message says why.
    pub(super) failed: bool,
}

impl View {
    pub(super) fn parse(pairs: &[(String, String)]) -> View {
        let get = |key: &str| pairs.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.trim()).filter(|v| !v.is_empty());
        let text = |key: &str| get(key).map(|v| v.chars().take(TEXT).collect::<String>());
        let timestamp = |key: &str| get(key).and_then(|v| v.parse().ok()).filter(|at| chrono::DateTime::from_timestamp_millis(*at).is_some());
        View {
            page: get("page").and_then(|p| Page::ALL.into_iter().find(|x| x.name() == p)).unwrap_or_default(),
            sort: get("sort").and_then(|p| Sort::ALL.into_iter().find(|x| x.name() == p)).unwrap_or_default(),
            zoom: get("zoom").and_then(|p| Zoom::ALL.into_iter().find(|x| x.name() == p)).unwrap_or_default(),
            group: text("group"),
            find: text("q"),
            version: text("v"),
            at: timestamp("at"),
            done: match (timestamp("done"), get("msg")) {
                (Some(at), Some(msg)) => {
                    Some(Done { msg: msg.chars().take(300).collect(), at, undo: get("undo").and_then(|u| u.parse().ok()), failed: get("err") == Some("1") })
                }
                _ => None,
            },
        }
    }

    /// Its query, without the toast; empty for the default view.
    pub(super) fn query(&self) -> String {
        let mut q = form_urlencoded::Serializer::new(String::new());
        if self.page != Page::List {
            q.append_pair("page", self.page.name());
        }
        if self.sort != Sort::Needs {
            q.append_pair("sort", self.sort.name());
        }
        if self.zoom != Zoom::Min {
            q.append_pair("zoom", self.zoom.name());
        }
        for (key, value) in [("group", &self.group), ("q", &self.find), ("v", &self.version)] {
            if let Some(v) = value {
                q.append_pair(key, v);
            }
        }
        if let Some(at) = self.at {
            q.append_pair("at", &at.to_string());
        }
        q.finish()
    }

    /// This view of an app, or of the overview: a path and query.
    pub(super) fn path(&self, app: Option<&str>) -> String {
        let path = match app {
            Some(id) => format!("/admin/apps/{}", form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()),
            None => "/admin".to_string(),
        };
        let q = self.query();
        if q.is_empty() { path } else { format!("{path}?{q}") }
    }

    /// A link to it, at the console; escaped where it is written out.
    fn link(&self, app: Option<&str>) -> String {
        format!("{}#console", self.path(app))
    }

    /// The same view, changed.
    fn with(&self, f: impl FnOnce(&mut View)) -> View {
        let mut v = self.clone();
        v.done = None;
        f(&mut v);
        v
    }

    /// Another page; the version and minute it showed stay behind.
    fn to(&self, page: Page) -> View {
        self.with(|v| {
            v.page = page;
            v.version = None;
            v.at = None;
        })
    }
}

// ---- the board: the apps as the console sees them --------------------------

/// What the console counts per app over its 16 cells.
#[derive(Default, Clone, Copy)]
struct Sums {
    requests: u64,
    failures: u64,
    stops: u64,
}

/// The apps on the console: everything it shows, read once in [`Board::new`];
/// drawing it reads nothing more.
pub(super) struct Board<'a> {
    shell: &'a Shell,
    apps: &'a [App],
    view: &'a View,
    now: i64,
    /// The selected app: the page's, else the list's first.
    chosen: Option<&'a App>,
    /// The row's first cell starts here; each is `cell` ms.
    from: i64,
    cell: i64,
    cells: BTreeMap<String, Vec<Cell>>,
    /// The apps on the list, in its order.
    order: Vec<&'a App>,
    /// What needs you, most pressing first.
    inbox: Vec<&'a App>,
    frozen: Option<Event>,
    /// The last change to policy.csv not taken, when and why.
    policy: Option<(i64, String)>,
    /// The apps Stop all turned off.
    stopped: Vec<String>,
    /// The selected app's newest events, oldest first: its story.
    events: Vec<Event>,
    /// The requests page's requests, newest first, and how many there are.
    requests: (Vec<Request>, u32),
    /// The toast's change, while Undo can still reverse it.
    undo: Option<i64>,
}

/// The screen's inks: a word's or a light's colour, never colour alone.
/// Ordered worst first, so the worst of some apps is their least.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Ink {
    Coral,
    Amber,
    Cyan,
    Dim,
    White,
    Blue,
}

impl Ink {
    fn class(self) -> &'static str {
        match self {
            Ink::Coral => "r",
            Ink::Amber => "a",
            Ink::Cyan => "c",
            Ink::Dim => "d",
            Ink::White => "w",
            Ink::Blue => "b",
        }
    }

    /// A tone as the screen colours it.
    fn of(tone: Tone) -> Ink {
        match tone {
            Tone::Critical => Ink::Coral,
            Tone::Warning | Tone::Caution => Ink::Amber,
            Tone::Info | Tone::Success => Ink::Cyan,
            Tone::Neutral => Ink::Dim,
        }
    }

    /// The LED that glows in it: red, amber, else blue.
    fn led(self) -> &'static str {
        match self {
            Ink::Coral => "red",
            Ink::Amber => "amber",
            _ => "blue",
        }
    }
}

/// A cell's or a map square's look: brighter is busier; amber at a limit
/// or close to it, coral when requests failed, hollow when not running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Heat {
    Idle,
    Low,
    Mid,
    High,
    Near,
    Failed,
    Off,
}

impl Heat {
    fn class(self) -> &'static str {
        match self {
            Heat::Idle => "",
            Heat::Low => "k1",
            Heat::Mid => "k2",
            Heat::High => "k3",
            Heat::Near => "ka",
            Heat::Failed => "kr",
            Heat::Off => "off",
        }
    }

    /// `n` against the busiest: thirds.
    fn busy(n: u64, busiest: u64) -> Heat {
        match (n, n * 3 / busiest.max(1)) {
            (0, _) => Heat::Idle,
            (_, 0) => Heat::Low,
            (_, 1) => Heat::Mid,
            _ => Heat::High,
        }
    }
}

/// An owner as the console shows it: the name without the address.
pub(super) fn who(owner: &str) -> String {
    let (name, mail) = mail_of(owner);
    if name.is_empty() { mail.unwrap_or("").to_string() } else { name.to_string() }
}

fn owner_of(a: &App) -> String {
    a.owner().map(who).unwrap_or_default()
}

/// An app's group as app.toml says it, trimmed; empty without one.
fn group_of(a: &App) -> String {
    a.group().map(|g| g.trim().to_string()).unwrap_or_default()
}

/// Whether two groups are the same: letter case and spaces around don't count.
fn same_group(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The start of the cell that holds `now`, cells starting on local time.
fn cell_start(now: i64, cell: i64) -> i64 {
    let offset = chrono::Local::now().offset().local_minus_utc() as i64 * 1000;
    now - (now + offset).rem_euclid(cell)
}

impl<'a> Board<'a> {
    /// Reads what the console shows of `apps` for `view`, with `selected`
    /// (the page's app) or else the list's first.
    pub(super) fn new(shell: &'a Shell, apps: &'a [App], view: &'a View, now: i64, selected: Option<&str>) -> Board<'a> {
        let db = &shell.platform;
        let cell = view.zoom.ms();
        let from = cell_start(now, cell) - (CELLS as i64 - 1) * cell;
        let mut inbox: Vec<&App> = apps.iter().filter(|a| a.needs_you()).collect();
        inbox.sort_by_cached_key(|a| (a.first_tone(), a.name.to_lowercase()));
        let undo = view.done.as_ref().filter(|d| !d.failed).and_then(|d| d.undo);
        let undo = undo.filter(|id| db.event_by_id(*id).ok().flatten().is_some_and(|e| reverse(db, &e, now).is_some()));
        let mut board = Board {
            shell,
            apps,
            view,
            now,
            chosen: None,
            from,
            cell,
            cells: db.cells(from, cell, CELLS).unwrap_or_default(),
            order: vec![],
            inbox,
            frozen: db.frozen().ok().flatten(),
            policy: shell.policy.as_ref().and_then(|p| p.refused()),
            stopped: db.stopped_all().unwrap_or_default(),
            events: vec![],
            requests: (vec![], 0),
            undo,
        };
        board.order = board.sorted();
        board.chosen = selected.and_then(|id| apps.iter().find(|a| a.id == id)).or_else(|| board.order.first().copied()).or(apps.first());
        if let Some(a) = board.chosen {
            match view.page {
                Page::Story => {
                    board.events = db.events(Some(&a.id), 8).unwrap_or_default();
                    board.events.reverse();
                }
                Page::Log => {
                    let at = board.log_cell(a);
                    let (reqs, n) = db.requests_like(&a.id, &Outcome::ALL, at, at + cell, LISTED, true).unwrap_or_default();
                    // The cell's count when the page shows one of the row's cells.
                    let counted = board.cells.get(&a.id).and_then(|c| c.get(usize::try_from((at - from) / cell).ok()?)).filter(|_| (at - from) % cell == 0);
                    board.requests = (reqs, counted.map_or(n, |c| u32::try_from(c.requests).unwrap_or(u32::MAX).max(n)));
                }
                _ => {}
            }
        }
        board
    }

    /// The selected app; none while there are no apps.
    pub(super) fn chosen(&self) -> Option<&'a App> {
        self.chosen
    }

    fn sums(&self, a: &App) -> Sums {
        self.cells.get(&a.id).map_or_else(Sums::default, |cells| {
            cells.iter().fold(Sums::default(), |s, c| Sums { requests: s.requests + c.requests, failures: s.failures + c.failures, stops: s.stops + c.stops })
        })
    }

    fn risk(&self, a: &App) -> i32 {
        match &a.running {
            Some(r) => self.shell.risk.of_app(&r.manifest).map_or(MAX_RISK as i32 + 1, i32::from),
            None => -1,
        }
    }

    /// The apps the list shows, narrowed by group and find, in its order.
    fn sorted(&self) -> Vec<&'a App> {
        let find = self.view.find.as_deref().map(str::to_lowercase);
        let mut out: Vec<&App> = self
            .apps
            .iter()
            .filter(|a| self.view.group.as_deref().is_none_or(|g| same_group(&group_of(a), g)))
            .filter(|a| {
                find.as_ref()
                    .is_none_or(|f| [&a.name, &a.id, a.owner().unwrap_or(""), a.group().unwrap_or("")].iter().any(|t| t.to_lowercase().contains(f.as_str())))
            })
            .collect();
        let name = |a: &App| a.name.to_lowercase();
        let newest = |a: &App| a.versions.iter().find(|v| v.status != VersionStatus::Refused).map_or(0, |v| v.deployed_at);
        match self.view.sort {
            Sort::Needs => out.sort_by_cached_key(|a| (a.urgency(), Reverse(self.sums(a).requests), name(a))),
            Sort::Group => {
                out.sort_by_cached_key(|a| (group_of(a).is_empty(), group_of(a).to_lowercase(), a.urgency(), Reverse(self.sums(a).requests), name(a)))
            }
            Sort::Busy => out.sort_by_cached_key(|a| (Reverse(self.sums(a).requests), name(a))),
            Sort::Failures => out.sort_by_cached_key(|a| (Reverse(self.sums(a).failures), Reverse(self.sums(a).requests), name(a))),
            Sort::Risk => out.sort_by_cached_key(|a| (Reverse(self.risk(a)), name(a))),
            Sort::Newest => out.sort_by_cached_key(|a| (Reverse(newest(a)), name(a))),
            Sort::Az => out.sort_by_cached_key(|a| name(a)),
        }
        out
    }

    /// The cell `i`'s start.
    fn start(&self, i: usize) -> i64 {
        self.from + i as i64 * self.cell
    }

    /// The groups, A–Z, each with how many apps it has.
    fn groups(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = vec![];
        for g in self.apps.iter().map(group_of).filter(|g| !g.is_empty()) {
            match out.iter_mut().find(|(x, _)| same_group(x, &g)) {
                Some((_, n)) => *n += 1,
                None => out.push((g, 1)),
            }
        }
        out.sort_by_key(|(g, _)| g.to_lowercase());
        out
    }

    /// A rate over the row, like "11" (per minute) or "0.4".
    fn rate(&self, requests: u64) -> String {
        let (_, per) = self.view.zoom.per();
        let span = (self.now - self.from).max(1);
        let r = requests as f64 * per as f64 / span as f64;
        match r {
            0.0 => "0".to_string(),
            r if r < 10.0 => format!("{r:.1}"),
            r => thousands(r.round() as u64),
        }
    }

    /// A cell's look against the app's busiest cell.
    fn heat(a: &App, c: &Cell, busiest: u64) -> Heat {
        if c.failures > c.stops {
            Heat::Failed
        } else if c.stops > 0 || (c.slowest_us as f64) >= a.limit_us() as f64 * CLOSE {
            Heat::Near
        } else {
            Heat::busy(c.requests, busiest)
        }
    }

    /// The app's cells, each with its start and look.
    fn row_cells(&self, a: &App) -> Vec<(i64, Heat, Cell)> {
        let none = vec![Cell::default(); CELLS];
        let cells = self.cells.get(&a.id).unwrap_or(&none);
        let busiest = cells.iter().map(|c| c.requests).max().unwrap_or(0);
        cells.iter().enumerate().map(|(i, c)| (self.start(i), Board::heat(a, c, busiest), *c)).collect()
    }

    /// What a cell says when the pointer is on it.
    fn cell_hint(&self, a: &App, at: i64, c: &Cell) -> String {
        let mut out = format!("{} {} · {}", a.name, cell_label(at, self.cell), plural(c.requests as usize, "request"));
        if c.failures > 0 {
            out += &format!(" · {} failed", c.failures);
        }
        if c.stops > 0 {
            out += &format!(" · {} at the {} s limit", c.stops, secs(a.limit_us()));
        }
        if c.slowest_us > 0 {
            out += &format!(" · slowest {} s", secs(c.slowest_us));
        }
        out
    }
}

/// An app on the App knob's lights and the minimap: coral (broken or
/// turned off), amber, not running (dim) or fine (cyan).
fn light(a: &App) -> Ink {
    if a.off.is_some() {
        return Ink::Coral;
    }
    match (Ink::of(a.first_tone()), a.running.is_none()) {
        (Ink::Coral | Ink::Amber, _) => Ink::of(a.first_tone()),
        (_, true) => Ink::Dim,
        _ => Ink::Cyan,
    }
}

/// An app's status on the screen: a word and its colour, never colour
/// alone. Turned off is coral, seen or not: visitors get the 503 page.
fn status(a: &App, sums: Sums, now: i64) -> (&'static str, Ink) {
    if a.off.is_some() {
        return ("Off", Ink::Coral);
    }
    if let Some(p) = a.unseen().next() {
        return (p.kind.word(), Ink::of(p.tone));
    }
    let new = a.changed.as_ref().is_some_and(|e| e.kind == Kind::Deployed && e.at >= now - HOUR);
    match () {
        _ if !a.problems.is_empty() => ("Seen", Ink::Dim),
        _ if a.running.is_none() => ("Not live", Ink::Dim),
        _ if new => ("New", Ink::Cyan),
        _ if sums.requests == 0 => ("Idle", Ink::Dim),
        _ => ("", Ink::Dim),
    }
}

// ---- small parts ---------------------------------------------------------------

/// Seconds like "0.4" or "2.0".
fn secs(us: u64) -> String {
    format!("{:.1}", us as f64 / 1e6)
}

/// The screen's four readouts: label, value, below, and its colour.
fn readouts(items: &[(String, String, String, Ink)]) -> String {
    let items: String = items
        .iter()
        .map(|(label, value, sub, ink)| {
            format!(
                r#"<div class="c-readout {}"><span class="c-rl">{}</span><b>{}</b><span class="c-rl">{}</span></div>"#,
                ink.class(),
                escape(label),
                escape(value),
                escape(sub)
            )
        })
        .collect();
    format!(r#"<div class="c-readouts">{items}</div>"#)
}

/// The screen's last line: what the pointer is on, or what keys do now.
fn hint(text: &str) -> String {
    format!(r#"<div class="c-hint" data-default="{0}">{0}</div>"#, escape(text))
}

/// The power symbol on the platform's and an app's power button.
const POWER_ICON: &str = r#"<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.5v8"></path><path d="M6.6 7a7.5 7.5 0 1 0 10.8 0"></path></svg>"#;

/// A slide switch with an LED at each end: a button that flips it.
struct Slide<'s> {
    /// On: the nub at the first end (left, or top when upright).
    on: bool,
    upright: bool,
    /// Nothing to switch: greyed, and it does nothing.
    dead: bool,
    /// The LED colours at the first and second end; "" is dark.
    leds: (&'s str, &'s str),
    label: &'s str,
    letter: &'s str,
    caption: &'s str,
    hint: &'s str,
}

impl Slide<'_> {
    fn html(&self) -> String {
        let class = format!("{}{}{}", if self.on { "" } else { " off" }, if self.upright { " v" } else { "" }, if self.dead { " dead" } else { "" });
        let dead = if self.dead { r#" type="button" aria-disabled="true""# } else { "" };
        let (corner, shortcut) = corner(self.letter);
        let caption = if self.caption.is_empty() { String::new() } else { format!(r#"<span class="c-silk dim">{}</span>"#, escape(self.caption)) };
        let led = |c: &str| if c.is_empty() { String::new() } else { format!(" {c}") };
        format!(
            r#"<button class="c-slide{class}" role="switch" aria-checked="{on}"{dead} aria-label="{label}"{shortcut} data-hint="{hint}">{corner}<span class="c-ends"><span class="c-led{first}"></span><span class="c-led{second}"></span></span><span class="c-track"><span class="c-nub"></span></span>{caption}</button>"#,
            on = self.on && !self.dead,
            label = escape(self.label),
            hint = escape(self.hint),
            first = led(self.leds.0),
            second = led(self.leds.1),
        )
    }
}

/// A key's letter in its corner, and the attributes that bind it; both
/// empty without one.
fn corner(letter: &str) -> (String, String) {
    if letter.is_empty() {
        return (String::new(), String::new());
    }
    (
        format!(r#"<span class="c-kh" aria-hidden="true">{}</span>"#, escape(&key_name(letter))),
        format!(r#" data-key="{}" aria-keyshortcuts="{}""#, escape(letter), escape(&aria_key(letter))),
    )
}

/// A key: a link, its keyboard letter in the corner, its hint. `flat` keys
/// sit in a strip; a page's key (`here` is some) is lit while its page shows.
fn key(label: &str, href: &str, letter: &str, hint: &str, flat: bool, here: Option<bool>) -> String {
    let (corner, shortcut) = corner(letter);
    let class = format!("{}{}{}", if flat { " flat" } else { "" }, if here.is_some() { " pg" } else { "" }, if here == Some(true) { " here" } else { "" });
    let current = if here == Some(true) { r#" aria-current="page""# } else { "" };
    format!(r#"<a class="c-key{class}" href="{}"{shortcut}{current} data-hint="{}">{corner}{label}</a>"#, escape(href), escape(hint))
}

/// A flat key that posts a form.
fn post_key(label: &str, action: &str, fields: &[(&str, &str)], letter: &str, hint: &str) -> String {
    let fields: String = fields.iter().map(|(k, v)| format!(r#"<input type="hidden" name="{k}" value="{}">"#, escape(v))).collect();
    let (corner, shortcut) = corner(letter);
    format!(
        r#"<form class="c-keyform" method="post" action="{}">{fields}<button class="c-key flat"{shortcut} data-hint="{}">{corner}{label}</button></form>"#,
        escape(action),
        escape(hint)
    )
}

/// A key that does nothing now.
fn dead_key(label: &str) -> String {
    format!(r#"<span class="c-key flat off">{label}</span>"#)
}

/// A key as its corner says it.
fn key_name(letter: &str) -> String {
    match letter {
        "Enter" => "↵".to_string(),
        "Escape" => "Esc".to_string(),
        "Shift" => "⇧".to_string(),
        l => l.to_uppercase(),
    }
}

fn aria_key(letter: &str) -> String {
    match letter {
        "/" => "Slash".to_string(),
        l => l.to_string(),
    }
}

/// An encoder: an LED ring for where the value is, round a knob whose
/// pointer turns with it. Clicking it follows `href`; admin.js turns it
/// with the wheel, a drag or the arrow keys through its `stops`.
struct Knob<'k> {
    label: &'k str,
    value: String,
    sub: &'k str,
    /// Where in its range, 0 to 1; none for a knob with nothing to do.
    at: Option<f64>,
    size: &'k str,
    href: String,
    /// The key that pushes it; "" for none.
    letter: &'k str,
    /// What it turns, for admin.js; "" for a knob that only links.
    param: &'k str,
    stops: Vec<Stop>,
    current: usize,
    /// Steps it snaps to, with a dot each; none for an endless one.
    detents: bool,
    hint: String,
    lit: bool,
    /// The App knob: a light per app (or stretch of apps) in `stops`'
    /// order, by tone; `current` is lit.
    lights: Vec<Light>,
}

/// One of a knob's values: what it is, how it reads under the knob, the
/// page it goes to and what the hint line says while it is turned there.
struct Stop {
    value: String,
    label: String,
    href: String,
    hint: String,
}

/// One of the App knob's lights: its colour, the first app it stands for,
/// the app a click on it picks (indexes into `stops`) and what hovering it
/// says.
struct Light {
    ink: Ink,
    from: usize,
    at: usize,
    hint: String,
}

impl Knob<'_> {
    fn html(&self) -> String {
        // The pointer: on the lit light or dot, else at `at`. Light k stands
        // for the values from its `from` on, so the lit one is the last
        // starting at or before `current`.
        let lights = self.lights.len();
        let lit_light = self.lights.iter().rposition(|l| l.from <= self.current).unwrap_or(0);
        let frac = match (lights, self.detents, self.stops.len()) {
            (l, _, _) if l > 1 => lit_light as f64 / (l - 1) as f64,
            (1, _, _) => 0.0,
            (_, true, n) if n > 1 => self.current as f64 / (n - 1) as f64,
            _ => self.at.unwrap_or(0.0),
        };
        let a = -135.0 + frac * 270.0;
        // A slider for assistive tech and admin.js: where it is, and for
        // each value its label, page and hint.
        let data = if self.param.is_empty() {
            String::new()
        } else {
            let json = |f: fn(&Stop) -> &str| escape(&serde_json::to_string(&self.stops.iter().map(f).collect::<Vec<_>>()).unwrap_or_default());
            format!(
                r#" role="slider" aria-valuemin="0" aria-valuemax="{}" aria-valuenow="{}" aria-valuetext="{}" data-knob="{}" data-at="{}" data-values="{}" data-labels="{}" data-hrefs="{}" data-hints="{}""#,
                self.stops.len().saturating_sub(1),
                self.current,
                escape(&self.value),
                self.param,
                self.current,
                json(|s| &s.value),
                json(|s| &s.label),
                json(|s| &s.href),
                json(|s| &s.hint),
            )
        };
        let ring = match (self.at, self.detents) {
            (Some(at), false) => format!(r#" style="--v: {:.0}deg""#, at * 270.0),
            _ => r#" style="--v: 0deg""#.to_string(),
        };
        let dots = if lights > 0 {
            let r = if lights <= 16 { 3.0 } else { 2.2 };
            let dots: String = self
                .lights
                .iter()
                .enumerate()
                .map(|(k, l)| {
                    let ang = (-135.0 + if lights > 1 { k as f64 * 270.0 / (lights - 1) as f64 } else { 0.0 }).to_radians();
                    let (x, y) = (64.0 + 60.0 * ang.sin(), 64.0 - 60.0 * ang.cos());
                    format!(
                        r#"<circle class="{}{}" cx="{x:.1}" cy="{y:.1}" r="{r}" data-from="{}" data-i="{}" data-hint="{}"></circle>"#,
                        l.ink.class(),
                        if k == lit_light { " on" } else { "" },
                        l.from,
                        l.at,
                        escape(&l.hint)
                    )
                })
                .collect();
            format!(r#"<svg class="c-dots" viewBox="0 0 128 128" aria-hidden="true">{dots}</svg>"#)
        } else if self.detents && self.stops.len() > 1 {
            let n = self.stops.len();
            let dots: String = (0..n)
                .map(|k| {
                    let ang = (-135.0 + k as f64 * 270.0 / (n - 1) as f64).to_radians();
                    let (x, y) = (37.0 + 32.0 * ang.sin(), 37.0 - 32.0 * ang.cos());
                    format!(r#"<circle cx="{x:.1}" cy="{y:.1}" r="2.6"{}></circle>"#, if k == self.current { r#" class="on""# } else { "" })
                })
                .collect();
            format!(r#"<svg class="c-dots" viewBox="0 0 74 74" aria-hidden="true">{dots}</svg>"#)
        } else {
            String::new()
        };
        let class = match self.detents {
            _ if lights > 0 => "c-det l",
            true => "c-det",
            false => "c-ring",
        };
        let lit = if self.lit { " lit" } else { "" };
        let (_, shortcut) = corner(self.letter);
        format!(
            r#"<div class="c-enc"><span class="c-silk">{label}</span><a class="{class}{lit}" href="{href}" draggable="false"{shortcut}{ring}{data} data-hint="{hint}" aria-label="{label}">{dots}<span class="c-knob {size}" style="--a: {a:.0}deg"></span></a><span class="c-val">{value}</span><span class="c-silk dim">{sub}</span></div>"#,
            label = self.label,
            href = escape(&self.href),
            hint = escape(&self.hint),
            value = escape(&self.value),
            size = self.size,
            sub = self.sub,
        )
    }
}

// ---- the console ---------------------------------------------------------------

impl Board<'_> {
    /// The console, for the selected app; empty without apps.
    pub(super) fn html(&self) -> String {
        let Some(chosen) = self.chosen else { return String::new() };
        let lcd = match self.view.page {
            Page::List => self.list(chosen),
            Page::Story => self.story(chosen),
            Page::Log => self.log(chosen),
            Page::Versions => self.versions(chosen),
            Page::Map => self.map(chosen),
        };
        let foot = match self.view.page {
            Page::List => match self.view.sort {
                Sort::Needs => ("Needs you first, then busiest".to_string(), format!("strip = all {}", self.order.len())),
                Sort::Group => ("By group, needs you first in each".to_string(), format!("strip = all {}", self.order.len())),
                s => (format!("Sorted: {}", s.label()), format!("strip = all {}", self.order.len())),
            },
            Page::Story => ("The app's story, from the event log".to_string(), "Esc = the list".to_string()),
            Page::Log => ("A step key below opens its minute here".to_string(), "Esc = the list".to_string()),
            Page::Map => ("One row per group, from app.toml".to_string(), "click a group = only its apps".to_string()),
            Page::Versions => ("Click a version to see what changes".to_string(), "nothing changes before you press the button".to_string()),
        };
        let shell = self.shell;
        format!(
            r#"<section class="console" id="console" aria-label="Platform console" data-app="{id}" data-name="{name}" data-page="{page}" data-failed="✗ No answer from the shell: the screen shows what it had · try again">
{ticker}
<div class="c-device">
<span class="c-screw tl"></span><span class="c-screw tr"></span><span class="c-screw bl"></span><span class="c-screw br"></span>
<div class="c-top"><span class="c-model">APPOS-1</span><span class="c-silk dim">Platform console · {platform}</span><span class="c-grow"></span><span class="c-silk dim c-wide">Hover anything: the screen's last line explains it</span></div>
<div class="c-body">
{left}
<div class="c-mid"><div class="c-bezel"><div class="c-lcd">{lcd}</div></div><div class="c-foot"><span class="c-silk dim">{f0}</span><span class="c-silk dim">{f1}</span></div></div>
{right}
</div>
{bottom}
</div>
</section>"#,
            id = escape(&chosen.id),
            name = escape(&chosen.name),
            page = self.view.page.name(),
            ticker = self.ticker(chosen),
            platform = escape(&shell.name),
            left = self.left(chosen),
            right = self.right(chosen),
            bottom = self.bottom(chosen),
            f0 = escape(&foot.0),
            f1 = escape(&foot.1),
        )
    }

    // ---- the ticker: the inbox ------------------------------------------------

    fn ticker(&self, chosen: &App) -> String {
        let n = self.inbox.len();
        // The selected app's item, else the first.
        let at = self.inbox.iter().position(|a| a.id == chosen.id).unwrap_or(0);
        let list = self.view.to(Page::List);
        let (text, led, open, seen, prev, next) = match self.inbox.get(at) {
            Some(a) => {
                let unseen: Vec<&Problem> = a.unseen().collect();
                let p = unseen[0];
                let more = if unseen.len() > 1 { format!(" · {} more", unseen.len() - 1) } else { String::new() };
                let group = group_of(a);
                let group = if group.is_empty() { String::new() } else { format!(r#"<span class="c-idx">{}</span>"#, escape(&group)) };
                let text = format!(
                    r#"<span class="c-idx">{}/{n}</span><b>{}</b>{group}<span class="c-said">{}{more}</span>"#,
                    at + 1,
                    escape(&a.name),
                    escape(&p.text)
                );
                let kinds: Vec<&str> = unseen.iter().map(|p| p.kind.as_str()).collect();
                let then = list.path(Some(&a.id));
                let open = key("Open", &self.view.to(Page::Story).link(Some(&a.id)), "", &format!("Open: {}'s story, from the event log", a.name), true, None);
                let seen = post_key(
                    "Seen",
                    &format!("/admin/apps/{}/seen", form_urlencoded::byte_serialize(a.id.as_bytes()).collect::<String>()),
                    &[("kinds", &kinds.join(",")), ("then", &then)],
                    "s",
                    &format!("Seen: {} leaves the inbox until something new happens; logged as admin", a.name),
                );
                let prev = self.inbox[(at + n - 1) % n];
                let next = self.inbox[(at + 1) % n];
                let led = if a.off.is_some() { Ink::Coral } else { Ink::of(p.tone) };
                (text, led, open, seen, list.link(Some(&prev.id)), list.link(Some(&next.id)))
            }
            None => {
                let running = self.apps.iter().filter(|a| a.running.is_some() && a.off.is_none()).count();
                let text = format!(r#"<span class="c-said">Nothing needs you · {} · {running} running</span>"#, plural(self.apps.len(), "app"));
                (text, Ink::Cyan, dead_key("Open"), dead_key("Seen"), list.link(Some(&chosen.id)), list.link(Some(&chosen.id)))
            }
        };
        let led = led.led();
        let (prev, next) = (escape(&prev), escape(&next));
        let frozen = match &self.frozen {
            Some(e) => format!(
                r#"<span class="c-frozen" data-hint="Deploys frozen since {} {}: new deploys wait for an admin; the Freeze switch, bottom right, lets them go live">Frozen</span>"#,
                clock(e.at),
                escape(&super::model::by(e)),
            ),
            None => String::new(),
        };
        let policy = match &self.policy {
            Some((at, _)) => format!(
                r#"<a class="c-frozen" href="/admin/policy" data-hint="policy.csv changed at {} but has a mistake; the policy stays as it was">Policy refused</a>"#,
                clock(*at)
            ),
            None => String::new(),
        };
        let (lit, cur, max) = self.load();
        let leds: String = (0..8).map(|i| format!(r#"<i class="c-led sq{}"></i>"#, if i < lit { " blue" } else { "" })).collect();
        format!(
            r#"<div class="c-ticker">
<a class="c-key flat c-arrow" href="{prev}" data-key="p" data-hint="Previous that needs you (P)" aria-label="Previous that needs you">◀</a><a class="c-key flat c-arrow" href="{next}" data-key="n" data-hint="Next that needs you (N)" aria-label="Next that needs you">▶</a>
<div class="c-tick" role="status"><span class="c-led {led}"></span>{text}</div>
<span class="c-tacts">{open}{seen}</span>
{frozen}{policy}<span class="c-load" data-hint="Load: {cur} requests in the last minute; the busiest minute this hour had {max}"><span class="c-silk dim">Load</span><span class="c-leds">{leds}</span></span>
</div>"#
        )
    }

    /// The platform's load: the last minute's requests against the busiest
    /// minute this hour, as 0 to 8 LEDs.
    fn load(&self) -> (usize, usize, usize) {
        let mut minutes = [0usize; 60];
        for r in self.apps.iter().flat_map(|a| &a.reqs) {
            let back = ((self.now - r.at) / MINUTE) as usize;
            if back < 60 {
                minutes[back] += 1;
            }
        }
        let (cur, max) = (minutes[0], minutes.iter().copied().max().unwrap_or(0));
        let lit = if cur == 0 { 0 } else { (cur * 8).div_ceil(max.max(1)).clamp(1, 8) };
        (lit, cur, max)
    }

    // ---- the left column: browse -----------------------------------------------

    fn left(&self, chosen: &App) -> String {
        let v = self.view;
        let dial = self.dial();
        let pos = dial.iter().position(|a| a.id == chosen.id);
        let n = dial.len();
        // Pushed: the next app on the dial (after the last, the first), on the same page.
        let next = pos.map_or(dial.first(), |p| dial.get((p + 1) % n.max(1))).copied().unwrap_or(chosen);
        let next_view = match v.page {
            Page::Log => v.with(|_| {}),
            page => v.to(page),
        };
        let app_knob = Knob {
            label: "App",
            value: chosen.name.clone(),
            sub: "push: next app",
            at: Some(pos.map_or(0.0, |p| if n > 1 { p as f64 / (n - 1) as f64 } else { 0.0 })),
            size: "l",
            href: next_view.link(Some(&next.id)),
            letter: "",
            param: "app",
            stops: dial
                .iter()
                .map(|a| Stop { value: a.id.clone(), label: a.name.clone(), href: next_view.path(Some(&a.id)), hint: format!("App: {}", a.name) })
                .collect(),
            current: pos.unwrap_or(0),
            detents: false,
            hint: format!("App: a light per app, amber and coral need you; click a light for its app, turn to step, push for the next ({}) · ↑↓", next.name),
            lit: false,
            lights: self.lights(&dial),
        };
        // GROUP: All, then the groups A–Z, each its list from its first app;
        // pushed, the next one.
        let groups = self.groups();
        let only = |g: &str| {
            v.to(Page::List).with(|x| {
                x.group = Some(g.to_string()).filter(|g| !g.is_empty());
                x.find = None;
            })
        };
        let mut stops = vec![Stop {
            value: String::new(),
            label: "All".to_string(),
            href: only("").path(None),
            hint: format!("Group: All · {}", plural(self.apps.len(), "app")),
        }];
        stops.extend(groups.iter().map(|(g, n)| Stop {
            value: g.clone(),
            label: g.clone(),
            href: only(g).path(None),
            hint: format!("Group: {g} · {}", plural(*n, "app")),
        }));
        let current = v.group.as_deref().and_then(|g| stops.iter().position(|s| same_group(&s.value, g))).unwrap_or(0);
        let next_group = stops[(current + 1) % stops.len()].value.clone();
        let count = match current {
            0 => plural(self.apps.len(), "app"),
            i => plural(groups[i - 1].1, "app"),
        };
        let detents = stops.len() <= 12;
        let group_knob = Knob {
            label: "Group",
            value: stops[current].label.clone(),
            sub: "push: next",
            at: Some(if stops.len() > 1 { current as f64 / (stops.len() - 1) as f64 } else { 0.0 }),
            size: "s",
            href: only(&next_group).link(None),
            letter: "g",
            param: "group",
            stops,
            current,
            detents,
            hint: format!("Group: only one group's apps ({count}), from app.toml's group; push for the next (G)"),
            lit: v.group.is_some(),
            lights: vec![],
        };
        let zoom_at = Zoom::ALL.iter().position(|z| *z == v.zoom).unwrap_or(0);
        let ranges: Vec<String> = Zoom::ALL.iter().map(|z| z.range()).collect();
        // Another range has other cells: the requests page's cell goes.
        let zoomed = |z: Zoom| {
            v.with(|x| {
                x.zoom = z;
                x.at = None;
            })
        };
        let range_knob = Knob {
            label: "Range",
            value: v.zoom.range(),
            sub: "− +",
            at: None,
            size: "s",
            href: zoomed(Zoom::ALL[(zoom_at + 1) % Zoom::ALL.len()]).link(Some(&chosen.id)),
            letter: "",
            param: "zoom",
            stops: Zoom::ALL
                .iter()
                .zip(&ranges)
                .map(|(z, r)| Stop { value: z.name().to_string(), label: r.clone(), href: zoomed(*z).path(Some(&chosen.id)), hint: format!("Range: {r}") })
                .collect(),
            current: zoom_at,
            detents: true,
            hint: format!("Range: how far back the rows and step keys go: {} (− +)", super::format::and(&ranges).replacen(" and ", " or ", 1)),
            lit: false,
            lights: vec![],
        };
        // List: from anywhere to the list; on the list, clears find, then the group.
        let list = match v.page {
            Page::List if v.find.is_some() => v.with(|x| x.find = None),
            Page::List if v.group.is_some() => v.with(|x| x.group = None),
            _ => v.to(Page::List),
        };
        format!(
            r#"<div class="c-left"><div class="c-grp"><span class="c-silk">Browse</span><span class="c-silk dim">which apps</span></div>
<div class="c-big">{app}</div>
<div class="c-knobs">{group}{range}</div>
<div class="c-keys2">{list}{map}</div></div>"#,
            app = app_knob.html(),
            group = group_knob.html(),
            range = range_knob.html(),
            list = key(
                "List",
                &list.link(Some(&chosen.id)),
                "Escape",
                "List: every app with its last minutes, needs you first; / finds in it; on the list, Esc clears find, then the group (Esc)",
                true,
                Some(v.page == Page::List),
            ),
            map =
                key("Map", &v.to(Page::Map).link(Some(&chosen.id)), "m", "Map: every app on one screen, a row per group (M)", true, Some(v.page == Page::Map)),
        )
    }

    // ---- the right column: what to do ----------------------------------------------

    fn right(&self, chosen: &App) -> String {
        let v = self.view;
        let id = chosen.id.as_str();
        let then = escape(&v.path(Some(id)));
        let action = |word: &str| escape(&format!("/admin/apps/{}/{word}", form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()));
        let name = chosen.name.as_str();
        // The app's on/off: green on the left while visitors get it, red on the right once off.
        let power = if chosen.running.is_none() {
            let hint = format!("On/off: {name} isn't running, so there is nothing to turn off");
            let label = format!("{name} isn't running");
            let button = Slide { on: false, upright: false, dead: true, leds: ("", ""), label: &label, letter: "", caption: "not running", hint: &hint };
            format!(r#"<div class="c-apwr">{}</div>"#, button.html())
        } else {
            let off = chosen.off.is_some();
            let hint = if off {
                format!("On/off: {name} is off, visitors get a “turned off” page; slide it on again, at once (X)")
            } else {
                format!("On/off: {name} is on; slide it off and visitors get a “turned off” page at once; undo from the toast (X)")
            };
            let leds = if off { ("", "red") } else { ("green", "") };
            let button = Slide { on: !off, upright: false, dead: false, leds, label: name, letter: "x", caption: if off { "off" } else { "on" }, hint: &hint };
            format!(
                r#"<form class="c-apwr" method="post" action="{}"><input type="hidden" name="then" value="{then}">{}</form>"#,
                action(if off { "on" } else { "off" }),
                button.html()
            )
        };
        let page_key = |label: &str, page: Page, letter: &str, hint: &str| key(label, &v.to(page).link(Some(id)), letter, hint, false, Some(v.page == page));
        format!(
            r#"<div class="c-right"><div class="c-grp"><span class="c-silk">This app</span></div><span class="c-val c-this">{name}</span>
<div class="c-col">{story}{requests}{versions}</div>
{power}
<div class="c-grp c-group"><span class="c-silk">Limits</span><span class="c-silk dim">per request</span></div>
{faders}</div>"#,
            name = escape(name),
            faders = self.faders(chosen),
            story = page_key("Story", Page::Story, "Enter", "Story: what happened to it, what is wrong now, what is left for you (↵)"),
            requests = page_key("Requests", Page::Log, "r", "Requests: one minute's requests; the step keys below pick the minute (R)"),
            versions = page_key("Versions", Page::Versions, "v", "Versions: every kept version; pick one to roll back to or make live (V)"),
        )
    }

    /// The app's limits per request as three faders: memory, time and fuel,
    /// each in steps with its default marked. A step posts its value at
    /// once; admin.js drags and turns them.
    fn faders(&self, chosen: &App) -> String {
        let now = chosen.budget();
        let default = Budget::with(chosen.toml_time(), AdminLimits::default());
        let set = chosen.limits;
        let st = stats(&chosen.live());
        let ran = st.requests > st.failures;
        let (mem_now, sec_now, fuel_now) = ((now.memory >> 20) as u32, now.time.as_secs() as u32, (now.fuel / 1_000_000) as u32);
        let mem = Fader {
            label: "Mem",
            what: "Memory",
            name: "memory",
            steps: steps(&[16, 32, 64, 128, 256, 512], &AdminLimits::MEMORY_MB, mem_now, (default.memory >> 20) as u32),
            now: mem_now,
            default: (default.memory >> 20) as u32,
            set: set.memory_mb.is_some(),
            text: |m| format!("{m} MB"),
            note: "a request asking for more stops".to_string(),
        };
        let time = Fader {
            label: "Time",
            what: "Time",
            name: "seconds",
            steps: steps(&[1, 2, 5, 10, 20, 30], &AdminLimits::SECONDS, sec_now, default.time.as_secs() as u32),
            now: sec_now,
            default: default.time.as_secs() as u32,
            set: set.seconds.is_some(),
            text: |s| format!("{s} s"),
            note: if ran {
                format!("a slower request stops; 95 in 100 of the last hour took {}", duration(st.wall_us.1))
            } else {
                "a slower request stops".to_string()
            },
        };
        let fuel = Fader {
            label: "Fuel",
            what: "Fuel",
            name: "fuel",
            steps: steps(&[250, 500, 1000, 2000, 4000, 8000], &AdminLimits::FUEL_M, fuel_now, (default.fuel / 1_000_000) as u32),
            now: fuel_now,
            default: (default.fuel / 1_000_000) as u32,
            set: set.fuel_m.is_some(),
            text: |m| super::format::fuel(u64::from(m) * 1_000_000),
            note: if ran {
                format!("instructions; a request running more stops; 95 in 100 of the last hour ran {}", super::format::fuel(st.fuel.1))
            } else {
                "instructions; a request running more stops".to_string()
            },
        };
        // Each fader posts only its own value; the shell keeps the others.
        let action = format!("/admin/apps/{}/limits", form_urlencoded::byte_serialize(chosen.id.as_bytes()).collect::<String>());
        let then = escape(&self.view.path(Some(&chosen.id)));
        let form = |f: &Fader| {
            format!(r#"<form class="c-fdrf" method="post" action="{}"><input type="hidden" name="then" value="{then}">{}</form>"#, escape(&action), f.html())
        };
        format!(r#"<div class="c-faders">{}{}{}</div>"#, form(&mem), form(&time), form(&fuel))
    }

    /// The platform's controls, beside the pins and step keys: the power
    /// button (Stop all, Start all) and the Freeze switch. Both act at once.
    fn platform(&self, chosen: &App) -> String {
        let v = self.view;
        let id = chosen.id.as_str();
        let stopped = &self.stopped;
        let running = self.apps.iter().filter(|a| a.running.is_some() && a.off.is_none()).count();
        let (power, power_action, power_hint) = if stopped.is_empty() {
            (
                "on",
                "/admin/stop-all",
                format!("Power: {} on; press to turn every app off at once, visitors get a 503; undo from the toast", plural(running, "app")),
            )
        } else {
            ("off", "/admin/start-all", format!("Power: Stop all turned {} off; press to turn them back on", plural(stopped.len(), "app")))
        };
        let frozen = self.frozen.is_some();
        let held = self.apps.iter().flat_map(|a| &a.versions).filter(|x| x.status == VersionStatus::Waiting).count();
        let freeze_hint = if frozen {
            format!(
                "Freeze is on (amber): new deploys wait for an admin ({} now, make each live on its Versions page); slide it down to let new ones go live (F)",
                plural(held, "held version")
            )
        } else {
            "Freeze is off (green): new deploys go live; slide it up to hold them for an admin, the CLI can't roll back then (F)".to_string()
        };
        // Up and amber: deploys held. Down and green: deploys go live.
        let leds = if frozen { ("amber", "") } else { ("", "green") };
        let freeze = Slide { on: frozen, upright: true, dead: false, leds, label: "Freeze deploys", letter: "f", caption: "", hint: &freeze_hint }.html();
        format!(
            r#"<div class="c-plat"><div class="c-grp"><span class="c-silk">Platform</span></div><div class="c-plat-row">
<form class="c-ctl" method="post" action="{power_action}"><input type="hidden" name="then" value="{then}"><button class="c-power {power}" data-hint="{power_hint}" aria-label="{power_label}">{POWER_ICON}</button><span class="c-silk">Power</span></form>
<form class="c-ctl" method="post" action="/admin/freeze"><input type="hidden" name="frozen" value="{flip}"><input type="hidden" name="then" value="{then}">{freeze}<span class="c-silk">Freeze</span></form>
</div></div>"#,
            power_hint = escape(&power_hint),
            power_label = if stopped.is_empty() { "Stop all apps" } else { "Start the stopped apps" },
            flip = if frozen { "0" } else { "1" },
            then = escape(&v.path(Some(id))),
        )
    }

    // ---- the bottom: pins and step keys -----------------------------------------

    fn bottom(&self, chosen: &App) -> String {
        let v = self.view;
        let pins: String = (1..=8)
            .map(|i| {
                format!(
                    r#"<button type="button" class="c-key c-pin" data-pin="{i}" aria-keyshortcuts="{i}" data-hint="Pin {i}: empty; hold it to keep the selected app here"><span class="c-pinrow"><span class="c-led"></span><span class="c-pinno">{i}</span></span><span class="c-pinname"></span></button>"#
                )
            })
            .collect();
        // The pins live in the browser; admin.js fills these in: {i} the
        // pin, {name} its app, {id} the app's id in a path.
        let jump = v.with(|x| {
            x.version = None;
            x.at = None;
        });
        let jump = match jump.query() {
            q if q.is_empty() => "/admin/apps/{id}".to_string(),
            q => format!("/admin/apps/{{id}}?{q}"),
        };
        let pin_texts = format!(
            r#" data-jump="{}" data-set="Pin {{i}}: {{name}} · click jumps to it ({{i}}) · hold keeps the selected app here instead" data-pinned="Pinned {{name}} to {{i}}""#,
            escape(&jump)
        );
        let log_at = (v.page == Page::Log).then(|| self.log_cell(chosen));
        let steps: String = self
            .row_cells(chosen)
            .iter()
            .enumerate()
            .map(|(i, (start, _, c))| {
                let back = (CELLS - 1 - i) as i64;
                let (led, down) = if log_at == Some(*start) { (" orange", " down") } else { ("", "") };
                let shown = if down.is_empty() { "" } else { r#" aria-current="true""# };
                format!(
                    r#"<div class="c-step"><span class="c-led{led}"></span><a class="c-key flat{down}" href="{}"{shown} data-hint="{} · click = its requests">{}</a></div>"#,
                    escape(&v.to(Page::Log).with(|x| x.at = Some(*start)).link(Some(&chosen.id))),
                    escape(&self.cell_hint(chosen, *start, c)),
                    escape(&v.zoom.step(back)),
                )
            })
            .collect();
        let platform = self.platform(chosen);
        let ago = match v.zoom {
            Zoom::Min | Zoom::Five => "Minutes ago",
            Zoom::Hour => "Hours ago",
            Zoom::Day => "Days ago",
        };
        format!(
            r#"<div class="c-lower"><div class="c-well"><div class="c-pins"{pin_texts}><div class="c-grp"><span class="c-silk dim">Pins · click = jump · hold = keep the selected app here</span></div><div class="c-pinkeys">{pins}</div></div><div class="c-grp"><span class="c-silk dim">{ago}</span><span class="c-silk dim">click = that {}'s requests</span></div><div class="c-steps">{steps}</div></div>{platform}</div>"#,
            match v.zoom {
                Zoom::Min | Zoom::Five => "minute",
                Zoom::Hour => "hour",
                Zoom::Day => "day",
            }
        )
    }

    // ---- the screen's pages -----------------------------------------------------

    /// The screen's first line: its page, what it shows, and on the right
    /// more; under it the toast after a change.
    fn head(&self, tag: &str, left: &str, right: &str) -> String {
        format!(r#"<div class="c-head"><span class="c-tag">{tag}</span>{left}<span class="c-grow"></span>{right}</div>{}"#, self.toast())
    }

    /// What a change did, for ten minutes, with its Undo while the server
    /// can still reverse it.
    fn toast(&self) -> String {
        let Some(done) = &self.view.done else { return String::new() };
        if self.now - done.at > UNDO_MS || done.at > self.now + MINUTE {
            return String::new();
        }
        if done.failed {
            return format!(r#"<div class="c-toast r" role="alert"><span>✗ {} · {}</span></div>"#, escape(&done.msg), hm(done.at));
        }
        let undo = match (self.undo, self.chosen) {
            (Some(event), Some(a)) => format!(
                r#"<form class="c-undo" method="post" action="/admin/undo"><input type="hidden" name="event" value="{event}"><input type="hidden" name="then" value="{}"><button data-key="u" aria-keyshortcuts="u" data-hint="Undo: reverses it, logged as admin (U)">U Undo</button></form>"#,
                escape(&self.view.path(Some(&a.id)))
            ),
            _ => String::new(),
        };
        format!(r#"<div class="c-toast" role="status"><span>✓ {} · {} · admin</span><span class="c-grow"></span>{undo}</div>"#, escape(&done.msg), hm(done.at))
    }

    fn list(&self, chosen: &App) -> String {
        let v = self.view;
        let total = self.order.len();
        let needs = self.order.iter().filter(|a| a.needs_you()).count();
        let found = if v.find.is_some() { " found" } else { "" };
        let filter = match &v.group {
            Some(g) => format!(r#"<span class="b">{} · {total}{found}</span>"#, escape(g)),
            None if v.find.is_some() => format!(r#"<span class="b">All · {total}{found}</span>"#),
            None => format!("<span>All · {total}</span>"),
        };
        let need = match needs {
            0 => r#"<span class="d">Nothing needs you</span>"#.to_string(),
            1 => r#"<span class="a">1 needs you</span>"#.to_string(),
            n => format!(r#"<span class="a">{n} need you</span>"#),
        };
        let clock = format!(r#"<span class="c-clock">{}</span><span class="c">Live</span>"#, hm(self.now));
        let pos = self.order.iter().position(|a| a.id == chosen.id).unwrap_or(0);
        let first = pos.saturating_sub(ROWS - 1);
        let range = if total == 0 { "0/0".to_string() } else { format!("{}–{}/{total}", first + 1, (first + ROWS).min(total)) };
        let right = format!(
            r#"<span class="a c-held" hidden>Order held</span>{}<span class="d c-range" data-of="{{first}}–{{last}}/{{n}}">{range}</span>"#,
            self.find_form(chosen)
        );
        let sort_link = |sort: Sort, text: &str, class: &str| {
            let mark = if v.sort == sort { " ▾" } else { "" };
            let current = if v.sort == sort { r#" aria-current="true""# } else { "" };
            format!(
                r#"<a class="{class}" href="{}"{current} data-hint="Sort by {}">{text}{mark}</a>"#,
                escape(&v.with(|x| x.sort = sort).link(Some(&chosen.id))),
                sort.label().to_lowercase()
            )
        };
        let (unit, _) = v.zoom.per();
        let cols = format!(
            r#"<div class="c-row c-colhead d"><span class="c-n">#</span>{}{}<span class="c-lim">Mem</span><span class="c-lim">Time</span><span class="c-lim">Fuel</span>{}{}{}</div>"#,
            sort_link(Sort::Az, "App", "c-name"),
            sort_link(Sort::Group, "Group", "c-gcol"),
            sort_link(Sort::Needs, "Status", "c-st"),
            sort_link(Sort::Failures, &format!("{} ······ {}", v.zoom.step(CELLS as i64 - 1), v.zoom.step(0)), "c-cells"),
            sort_link(Sort::Busy, unit, "c-rate"),
        );
        let mut rows = String::new();
        let mut last_group: Option<String> = None;
        for (i, a) in self.order.iter().enumerate() {
            if v.sort == Sort::Needs && i == needs && needs > 0 {
                rows += &format!(r#"<div class="c-sep d"><span>{} more · busiest first</span></div>"#, total - needs);
            }
            if v.sort == Sort::Group {
                let g = group_of(a);
                if last_group.as_deref().is_none_or(|l| !same_group(l, &g)) {
                    let n = self.order.iter().filter(|x| same_group(&group_of(x), &g)).count();
                    let name = if g.is_empty() { "No group".to_string() } else { g.clone() };
                    rows += &format!(r#"<div class="c-sep b"><span>{} · {n}</span></div>"#, escape(&name));
                    last_group = Some(g);
                }
            }
            rows += &self.row(i, a, a.id == chosen.id);
        }
        if self.order.is_empty() {
            rows = format!(
                r#"<p class="c-none">No app matches. <a href="{}">Show all</a></p>"#,
                escape(
                    &v.with(|x| {
                        x.find = None;
                        x.group = None;
                    })
                    .link(Some(&chosen.id))
                )
            );
        }
        let body = format!(r#"{cols}<div class="c-listwrap"><div class="c-list">{rows}</div>{mini}</div>"#, mini = self.minimap(first));
        format!(
            "{}{body}<span class=\"c-grow\"></span>{}{}",
            self.head("List", &format!("{clock}{filter}{need}"), &right),
            self.app_readouts(chosen),
            hint(&format!("↑↓ move · click a row = select it · ↵ its story · / find · N next that needs you · hover a cell = that {}", unit_word(v.zoom))),
        )
    }

    fn row(&self, i: usize, a: &App, selected: bool) -> String {
        let v = self.view;
        let sums = self.sums(a);
        let (word, ink) = status(a, sums, self.now);
        let href = escape(&v.link(Some(&a.id)));
        // For admin.js: the page a click selects it on, the one ↵ opens,
        // and what Find looks in, as the shell does.
        let select = escape(&v.path(Some(&a.id)));
        let open = escape(&v.to(Page::Story).path(Some(&a.id)));
        let find = escape(&[a.name.as_str(), &a.id, a.owner().unwrap_or(""), a.group().unwrap_or("")].map(str::to_lowercase).join("\n"));
        let limits = limit_cells(a);
        // The group in words; the Group knob narrows to it, a click on the row selects.
        let group = group_of(a);
        let group_html =
            if group.is_empty() { r#"<span class="c-gcol d">—</span>"#.to_string() } else { format!(r#"<span class="c-gcol b">{}</span>"#, escape(&group)) };
        let cells: String = self
            .row_cells(a)
            .iter()
            .map(|(at, heat, c)| format!(r#"<i class="{}" data-hint="{}"></i>"#, heat.class(), escape(&self.cell_hint(a, *at, c))))
            .collect();
        let why = match a.unseen().next().or(a.problems.first()) {
            Some(p) => format!("{}: {}", a.name, p.text),
            None => format!("{}: {}", a.name, a.line()),
        };
        format!(
            r#"<div class="c-row{sel}" data-app="{id}" data-href="{select}" data-open="{open}" data-find="{find}" data-status="{word}"><span class="c-n d">{n}</span><a class="c-name {name_ink}" href="{href}"{current} data-hint="{why}">{name}</a>{group_html}{limits}<span class="c-st {tone}">{word}</span><span class="c-cells">{cells}</span><span class="c-rate">{rate}</span></div>"#,
            sel = if selected { " sel" } else { "" },
            current = if selected { r#" aria-current="true""# } else { "" },
            id = escape(&a.id),
            name = escape(&a.name),
            n = i + 1,
            tone = ink.class(),
            name_ink = if ink == Ink::Dim { Ink::White } else { ink }.class(),
            why = escape(&why),
            rate = self.rate(sums.requests),
        )
    }

    /// The strip beside the list: every app on it, problems marked, the
    /// part on screen boxed (admin.js moves the box as the list scrolls).
    /// The App knob's apps: the list's, A to Z. A dial's marks stay where
    /// they are; an app that turns red keeps its place on it.
    fn dial(&self) -> Vec<&App> {
        let mut dial = self.order.clone();
        dial.sort_by_cached_key(|a| (a.name.to_lowercase(), a.id.clone()));
        dial
    }

    /// The App knob's lights: one per app on the dial, at most [`LIGHTS`];
    /// beyond that each stands for a stretch of apps and shows, and picks,
    /// the worst of them.
    fn lights(&self, dial: &[&App]) -> Vec<Light> {
        let n = dial.len();
        let count = n.min(LIGHTS);
        (0..count)
            .map(|k| {
                let (from, to) = (k * n / count, (k + 1) * n / count);
                let at = (from..to).min_by_key(|i| light(dial[*i])).unwrap_or(from);
                let a = dial[at];
                let (word, _) = status(a, self.sums(a), self.now);
                let mut hint = a.name.clone();
                if !word.is_empty() {
                    hint += &format!(" · {word}");
                }
                if to - from > 1 {
                    hint += &format!(" · and {} more here", to - from - 1);
                }
                hint += " · click = pick it";
                Light { ink: light(a), from, at, hint }
            })
            .collect()
    }

    fn minimap(&self, first: usize) -> String {
        let n = self.order.len().max(1);
        let marks: String = self
            .order
            .iter()
            .enumerate()
            .filter_map(|(i, a)| match light(a) {
                Ink::Cyan => None,
                ink => Some(format!(r#"<rect class="{}" x="1" y="{i}" width="10" height="1"></rect>"#, ink.class())),
            })
            .collect();
        let shown = ROWS.min(n);
        format!(
            r#"<svg class="c-mini" viewBox="0 0 12 {n}" preserveAspectRatio="none" aria-label="Where the screen is among all {n} apps"><rect class="c-track" x="3" y="0" width="6" height="{n}"></rect>{marks}<rect class="c-win" x="0.5" y="{first}" width="11" height="{shown}"></rect></svg>"#
        )
    }

    fn find_form(&self, chosen: &App) -> String {
        let v = self.view.with(|x| {
            x.page = Page::List;
            x.find = None;
        });
        let hidden: String = form_urlencoded::parse(v.query().as_bytes())
            .map(|(k, val)| format!(r#"<input type="hidden" name="{}" value="{}">"#, escape(&k), escape(&val)))
            .collect();
        format!(
            r#"<form class="c-find" method="get" action="{}" role="search"><label for="c-q">Find</label><input id="c-q" name="q" value="{}" placeholder="name, owner, group" autocomplete="off" aria-keyshortcuts="Slash" data-hint="Find: narrows the list as you type a name, id, owner or group; Enter keeps it, Esc clears it (/)">{hidden}</form>"#,
            escape(&format!("/admin/apps/{}", form_urlencoded::byte_serialize(chosen.id.as_bytes()).collect::<String>())),
            escape(self.view.find.as_deref().unwrap_or("")),
        )
    }

    /// The selected app's numbers under the list.
    fn app_readouts(&self, a: &App) -> String {
        let sums = self.sums(a);
        let live = a.versions.iter().find(|x| x.status == VersionStatus::Live);
        let folder = a.folder_version();
        let since = match (&a.changed, live, folder) {
            (Some(e), ..) if matches!(e.kind, Kind::RolledBack | Kind::AutoRolledBack) => format!("back since {}", hm(e.at)),
            (Some(e), ..) if e.kind == Kind::TurnedOff => format!("off since {}", hm(e.at)),
            (_, Some(l), _) => format!("since {}", clock(l.deployed_at)),
            (_, None, Some(_)) => "from its folder".to_string(),
            (_, None, None) => "not live".to_string(),
        };
        let version = live.map(|l| l.version.as_str()).or(folder).unwrap_or("—");
        let (unit, _) = self.view.zoom.per();
        let window = self.view.zoom.range();
        let (risk, grant) = self.risk_line(a);
        let failed_sub = if sums.stops > 0 { format!("{} at the limit", sums.stops) } else { format!("in {window}") };
        readouts(&[
            (a.name.clone(), version.to_string(), since, Ink::White),
            (format!("Req{unit}"), self.rate(sums.requests), format!("{} in {window}", thousands(sums.requests)), Ink::Cyan),
            ("Failed".to_string(), thousands(sums.failures), failed_sub, if sums.failures > 0 { Ink::Coral } else { Ink::Dim }),
            ("Risk".to_string(), risk, grant, Ink::Blue),
        ])
    }

    /// An app's risk and its riskiest grant: "2", "kv-write".
    fn risk_line(&self, a: &App) -> (String, String) {
        match &a.running {
            Some(r) => {
                let grants = &r.manifest.capabilities.grant;
                let top = grants.iter().max_by_key(|g| self.shell.risk.of(g).map_or(i32::MAX, i32::from)).cloned().unwrap_or_else(|| "no grants".to_string());
                (self.shell.risk.of_app(&r.manifest).map_or("?".to_string(), |l| l.to_string()), top)
            }
            None => ("—".to_string(), "not running".to_string()),
        }
    }

    /// The app's name, group and owner for a page's first line.
    fn who_line(&self, a: &App) -> String {
        let group = group_of(a);
        let owner = owner_of(a);
        let mut out = format!(r#"<span class="c-clock">{}</span>"#, escape(&a.name));
        if !group.is_empty() {
            out += &format!(r#"<span class="b">{}</span>"#, escape(&group));
        }
        if !owner.is_empty() {
            out += &format!(r#"<span class="d">{}</span>"#, escape(&owner));
        }
        out
    }

    /// What happened to the app, oldest first: its events, with how its
    /// requests went between them when the screen knows.
    fn timeline(&self, a: &App) -> Vec<(String, String, Ink)> {
        let events = &self.events;
        let mut out = vec![];
        for (i, e) in events.iter().enumerate() {
            out.push((clock(e.at), sentence(e, None), Ink::of(Tone::of_event(e.kind))));
            // After a version went live: how it did until the next change.
            if matches!(e.kind, Kind::Deployed | Kind::RolledBack | Kind::AutoRolledBack) {
                let until =
                    events[i + 1..].iter().find(|x| matches!(x.kind, Kind::Deployed | Kind::RolledBack | Kind::AutoRolledBack)).map_or(self.now, |x| x.at);
                let reqs: Vec<&Request> = a.reqs.iter().filter(|r| r.at >= e.at && r.at < until && *r.label == *e.label).collect();
                if !reqs.is_empty() && e.at >= self.now - HOUR {
                    let fails: Vec<&&Request> = reqs.iter().filter(|r| r.outcome.failed()).collect();
                    let stretch = if until == self.now { format!("{}–now", hm(e.at)) } else { format!("{}–{}", hm(e.at), hm(until)) };
                    let text = if fails.is_empty() {
                        format!("{} · {} · 0 failed", ver(&e.label), plural(reqs.len(), "request"))
                    } else {
                        let routes: Vec<&str> = fails.iter().map(|r| &*r.route).collect();
                        let one = routes.iter().all(|r| *r == routes[0]);
                        let on = if one { format!(" · all on {}", routes[0]) } else { String::new() };
                        format!("{} of {} failed{on}", fails.len(), reqs.len())
                    };
                    out.push((stretch, text, if fails.is_empty() { Ink::Cyan } else { Ink::Coral }));
                }
            }
        }
        out
    }

    fn story(&self, a: &App) -> String {
        let owner = owner_of(a);
        let live = a.versions.iter().find(|x| x.status == VersionStatus::Live);
        let state = match (live, a.folder_version()) {
            (Some(l), _) => format!("{} live", escape(&l.version)),
            (None, Some(v)) => format!("{} from its folder", escape(v)),
            (None, None) => "not live".to_string(),
        };
        let left = format!(r#"{}<span class="c">{state}</span>"#, self.who_line(a));
        let lines: String = self
            .timeline(a)
            .iter()
            .map(|(at, text, ink)| {
                format!(r#"<div class="c-line"><span class="d">{}</span><span class="{}">{}</span></div>"#, escape(at), ink.class(), escape(text))
            })
            .collect();
        let lines = if lines.is_empty() { r#"<p class="c-none">Nothing happened to it yet.</p>"#.to_string() } else { lines };
        let (now, left_for_you) = verdict(a, if owner.is_empty() { "the owner" } else { &owner });
        let seen = match &a.seen {
            Some(e) if !a.problems.is_empty() && !a.needs_you() => format!(" Seen at {}.", hm(e.at)),
            _ => String::new(),
        };
        let fails_hour = a.reqs.iter().filter(|r| r.outcome.failed()).count();
        let fails_now = a.reqs.iter().filter(|r| r.outcome.failed() && r.at >= self.now - 15 * MINUTE).count();
        let recovered = a.paused.as_ref().and_then(|e| e.recovery_ms).map(span);
        let limit = a.limit_us();
        let slowest = a.reqs.iter().map(|r| r.wall_us).max().unwrap_or(0);
        let (risk, grant) = self.risk_line(a);
        let keys = if a.needs_you() {
            "S marks it seen · R its requests · V its versions · Esc the list"
        } else {
            "R its requests · V its versions · Esc the list"
        };
        format!(
            r#"{}<div class="c-lines">{lines}</div><div class="c-verdict"><span>{}{}</span><span class="c">{}</span></div><span class="c-grow"></span>{}{}"#,
            self.head("Story", &left, r#"<span class="d">What happened</span>"#),
            escape(&now),
            escape(&seen),
            escape(&left_for_you),
            readouts(&[
                ("Failed".to_string(), thousands(fails_hour as u64), "in the last hour".to_string(), if fails_hour > 0 { Ink::Coral } else { Ink::Dim }),
                match recovered {
                    Some(r) => ("Recovered".to_string(), r, "after the 1st failure".to_string(), Ink::Cyan),
                    None =>
                        ("Now".to_string(), thousands(fails_now as u64), "failed in 15 min".to_string(), if fails_now > 0 { Ink::Coral } else { Ink::Cyan }),
                },
                ("Slowest".to_string(), format!("{} s", secs(slowest)), format!("of {} s, last hour", secs(limit)), near(slowest, limit)),
                ("Risk".to_string(), risk, grant, Ink::Blue),
            ]),
            hint(keys),
        )
    }

    /// The cell the log shows: as asked, else the newest with requests.
    fn log_cell(&self, a: &App) -> i64 {
        if let Some(at) = self.view.at {
            return at;
        }
        let cells = self.row_cells(a);
        cells.iter().rev().find(|(_, _, c)| c.requests > 0).map_or(self.start(CELLS - 1), |(at, _, _)| *at)
    }

    fn log(&self, a: &App) -> String {
        let at = self.log_cell(a);
        let limit = a.limit_us();
        let (reqs, total) = (&self.requests.0, self.requests.1);
        let fails = reqs.iter().filter(|r| r.outcome.failed()).count();
        let stops = reqs.iter().filter(|r| r.outcome.stopped()).count();
        let slowest = reqs.iter().map(|r| r.wall_us).max().unwrap_or(0);
        let left = format!(
            r#"<span class="c-clock">{}</span><span class="b">{}</span><span>{}</span>{}"#,
            escape(&a.name),
            escape(&cell_label(at, self.cell)),
            plural(total as usize, "request"),
            match (fails, stops) {
                (0, _) => String::new(),
                (_, s) if s > 0 => format!(r#"<span class="a">{s} stopped at {} s</span>"#, secs(limit)),
                (f, _) => format!(r#"<span class="r">{f} failed</span>"#),
            }
        );
        let right = format!(r#"<span class="d">newest {} of {}</span>"#, reqs.len(), total);
        let rows: String = reqs
            .iter()
            .map(|r| {
                let share = (r.wall_us as f64 / limit.max(1) as f64 * 100.0).min(100.0);
                let ink = if r.outcome.stopped() {
                    Ink::Amber
                } else if r.outcome.failed() {
                    Ink::Coral
                } else {
                    Ink::Cyan
                };
                let word = match r.outcome {
                    Outcome::Timeout => "stopped",
                    Outcome::OverBudget => "over budget",
                    o => o.as_str(),
                };
                let why = r.error.as_deref().map(|e| format!(" · {}", first_line(e))).unwrap_or_default();
                let tone = ink.class();
                format!(
                    r#"<div class="c-row c-req" data-hint="{} {} · {} · {} s of {} s{}"><span class="d">{}</span><span class="{tone}">{}</span><span class="{tone}">{word}</span><span class="c-rate">{}</span><span class="c-bar1"><i class="{}" style="width: {share:.0}%"></i></span></div>"#,
                    local(r.at, "%H:%M:%S"),
                    escape(&r.route),
                    word,
                    secs(r.wall_us),
                    secs(limit),
                    escape(&why),
                    local(r.at, "%H:%M:%S"),
                    escape(&r.route),
                    secs(r.wall_us),
                    if ink == Ink::Cyan { "" } else { tone },
                )
            })
            .collect();
        let rows = if rows.is_empty() { format!(r#"<p class="c-none">No requests in this {}.</p>"#, unit_word(self.view.zoom)) } else { rows };
        let insight = insight(a, reqs, limit);
        format!(
            r#"{}<div class="c-row c-req c-colhead d"><span>Time</span><span>Request</span><span>Status</span><span class="c-rate">Sec</span><span>of {} s</span></div><div class="c-list c-reqs">{rows}</div>{insight}<span class="c-grow"></span>{}{}"#,
            self.head("Requests", &left, &right),
            secs(limit),
            readouts(&[
                ("Requests".to_string(), thousands(total as u64), cell_label(at, self.cell), Ink::White),
                (
                    "Failed".to_string(),
                    fails.to_string(),
                    if stops > 0 { format!("{stops} at the limit") } else { format!("of the newest {}", reqs.len()) },
                    if fails > 0 { Ink::Coral } else { Ink::Dim }
                ),
                ("Slowest".to_string(), format!("{} s", secs(slowest)), format!("of {} s", secs(limit)), near(slowest, limit)),
                ("Version".to_string(), a.running.as_ref().map_or("—".to_string(), |r| r.manifest.app.version.clone()), "live now".to_string(), Ink::Blue),
            ]),
            hint(&format!("↑↓ requests · ← → the {0} before or after · Esc the list", unit_word(self.view.zoom))),
        )
    }

    /// The version the page shows: as picked, else the one an admin most
    /// likely wants: a waiting one, the one a rollback goes to, the live one.
    fn version_on_screen<'v>(&self, a: &'v App) -> Option<&'v Version> {
        if let Some(l) = &self.view.version {
            return a.versions.iter().find(|x| x.label == *l);
        }
        let live = a.versions.iter().position(|x| x.status == VersionStatus::Live);
        let back = a.versions.iter().skip(live.map_or(0, |i| i + 1)).find(|x| x.status == VersionStatus::Retired);
        a.versions.iter().find(|x| x.status == VersionStatus::Waiting).or(back).or(live.map(|i| &a.versions[i])).or(a.versions.first())
    }

    fn versions(&self, a: &App) -> String {
        let v = self.view;
        let live = a.versions.iter().find(|x| x.status == VersionStatus::Live);
        let picked = self.version_on_screen(a);
        let kept = a.versions.iter().filter(|x| x.status != VersionStatus::Refused).count();
        let left = format!(r#"{}<span>{kept} kept</span>"#, self.who_line(a));
        let right = live.map_or(String::new(), |l| format!(r#"<span class="c">{} live</span>"#, escape(&l.version)));
        let rows: String = a
            .versions
            .iter()
            .take(40)
            .map(|x| {
                let note = match x.status {
                    VersionStatus::Live => {
                        let hits = a.reqs.iter().filter(|r| *r.label == x.label && r.outcome.stopped()).count();
                        if hits > 0 { format!("live · {}", plural(hits, "limit hit")) } else { "live".to_string() }
                    }
                    VersionStatus::Waiting => "waits for an admin".to_string(),
                    VersionStatus::Failed => format!("failed{}", x.reason.as_deref().map(|r| format!(": {}", first_line(r))).unwrap_or_default()),
                    VersionStatus::Refused => "refused, never stored".to_string(),
                    _ => "kept".to_string(),
                };
                let tone = Ink::of(Tone::of_version(x.status)).class();
                let picked = picked.is_some_and(|p| p.id == x.id);
                let (sel, current) = if picked { (" sel", r#" aria-current="true""#) } else { ("", "") };
                let shown = v.with(|y| y.version = Some(x.label.clone()));
                format!(
                    r#"<a class="c-row c-ver{sel}" href="{}" data-open="{}"{current} data-hint="{} · click = what changes if it goes live"><span class="{tone}">{}</span><span class="b">{}</span><span class="{tone}">{}</span></a>"#,
                    escape(&shown.link(Some(&a.id))),
                    escape(&shown.path(Some(&a.id))),
                    escape(&x.version),
                    escape(&x.version),
                    escape(&clock(x.deployed_at)),
                    escape(&note),
                )
            })
            .collect();
        let (compare, form) = match (picked, live) {
            (Some(p), _) if p.status == VersionStatus::Refused => (
                format!(
                    r#"<div class="c-box"><div class="d">{} was refused: it was never stored, so it can't go live.</div><div>{}</div></div>"#,
                    escape(&p.version),
                    escape(first_line(p.reason.as_deref().unwrap_or("")))
                ),
                String::new(),
            ),
            (Some(p), Some(l)) if p.id == l.id => (
                format!(r#"<div class="c-box"><div>{} is live. Click another version to roll back to it or make it live.</div></div>"#, escape(&p.version)),
                String::new(),
            ),
            (Some(p), live) => {
                let button = match live {
                    Some(l) if p.deployed_at < l.deployed_at => format!("Roll back to {}", p.version),
                    _ => format!("Make {} live", p.version),
                };
                let form = self.form(
                    a,
                    &format!("/admin/apps/{}/rollback", form_urlencoded::byte_serialize(a.id.as_bytes()).collect::<String>()),
                    &[("label", &p.label)],
                    &button,
                );
                (compare(&self.shell.risk, p, live, a, self.now), form)
            }
            (None, _) => (r#"<div class="c-box">No versions kept.</div>"#.to_string(), String::new()),
        };
        format!(
            r#"{}<div class="c-list c-vers">{rows}</div>{compare}<span class="c-grow"></span>{form}{}"#,
            self.head("Versions", &left, &right),
            hint("Click a version to see what changes · the button makes it live · Esc the list"),
        )
    }

    /// A change on the screen: a button that says what it does, and Cancel.
    fn form(&self, chosen: &App, action: &str, fields: &[(&str, &str)], button: &str) -> String {
        let then = self.view.to(Page::List).path(Some(&chosen.id));
        let hidden: String =
            fields.iter().chain(&[("then", then.as_str())]).map(|(k, val)| format!(r#"<input type="hidden" name="{k}" value="{}">"#, escape(val))).collect();
        format!(
            r#"<form class="c-act" method="post" action="{action}">{hidden}<div class="c-actrow"><button class="c-btn" data-hint="{button}: happens at once, logged as admin, undo for 10 min">{button}</button><a class="c-cancel" href="{cancel}" data-hint="Cancel: back to the list, nothing changes (Esc)">Cancel</a><span class="c-grow"></span><span class="d">Logged as admin · undo for 10 min</span></div></form>"#,
            action = escape(action),
            button = escape(button),
            cancel = escape(&self.view.to(Page::List).link(Some(&chosen.id))),
        )
    }

    fn map(&self, chosen: &App) -> String {
        let v = self.view;
        // One row per group, those with something that needs you first.
        let mut groups: Vec<(String, Vec<&App>)> = vec![];
        for a in self.apps {
            let g = group_of(a);
            match groups.iter_mut().find(|(x, _)| same_group(x, &g)) {
                Some((_, apps)) => apps.push(a),
                None => groups.push((g, vec![a])),
            }
        }
        for (_, apps) in &mut groups {
            apps.sort_by_cached_key(|a| (a.urgency(), Reverse(self.sums(a).requests), a.name.to_lowercase()));
        }
        groups.sort_by_cached_key(|(g, apps)| (Reverse(apps.iter().filter(|a| a.needs_you()).count()), g.is_empty(), g.to_lowercase()));
        let mine = group_of(chosen);
        let rows: String = groups
            .iter()
            .map(|(g, apps)| {
                let busiest = apps.iter().map(|x| self.sums(x).requests).max().unwrap_or(1).max(1);
                let squares: String = apps
                    .iter()
                    .map(|a| {
                        let sums = self.sums(a);
                        let (word, ink) = status(a, sums, self.now);
                        let heat = match (ink, a.off.is_some() || a.running.is_none()) {
                            (Ink::Coral, _) => Heat::Failed,
                            (Ink::Amber, _) => Heat::Near,
                            (_, true) => Heat::Off,
                            _ => Heat::busy(sums.requests, busiest),
                        };
                        let (cur, current) = if a.id == chosen.id { (" cur", r#" aria-current="true""#) } else { ("", "") };
                        let what = if word.is_empty() { String::new() } else { format!(" · {word}") };
                        let story = v.to(Page::Story);
                        format!(
                            r#"<a class="c-sq {}{cur}" href="{}" data-app="{}" data-open="{}"{current} data-hint="{}{} · {}{} · click = its story" aria-label="{}"></a>"#,
                            heat.class(),
                            escape(&story.link(Some(&a.id))),
                            escape(&a.id),
                            escape(&story.path(Some(&a.id))),
                            escape(&a.name),
                            escape(&what),
                            self.rate(sums.requests),
                            v.zoom.per().0,
                            escape(&a.name),
                        )
                    })
                    .collect();
                let needs: Vec<&&App> = apps.iter().filter(|a| a.needs_you()).collect();
                let note = match needs.first() {
                    Some(a) => {
                        let p = a.unseen().next().map_or("", |p| p.kind.chip());
                        format!(r#"<span class="{}">{} {}</span>"#, Ink::of(a.first_tone()).class(), needs.len(), escape(&p.to_lowercase()))
                    }
                    None => String::new(),
                };
                let sel = if same_group(g, &mine) { " sel" } else { "" };
                let name = if g.is_empty() { "No group".to_string() } else { g.clone() };
                let only = v.to(Page::List).with(|x| {
                    x.group = Some(g.clone()).filter(|g| !g.is_empty());
                    x.find = None;
                });
                format!(
                    r#"<div class="c-mrow{sel}"><a class="b" href="{}" data-hint="{}: click = only its apps, in the list">{}</a><span class="d c-rate">{}</span><span class="c-sqs">{squares}</span>{note}</div>"#,
                    escape(&only.link(apps.first().map(|a| a.id.as_str()))),
                    escape(&name),
                    escape(&name),
                    apps.len(),
                )
            })
            .collect();
        let theirs: Vec<&App> = self.apps.iter().filter(|a| same_group(&group_of(a), &mine)).collect();
        let reqs: u64 = theirs.iter().map(|a| self.sums(a).requests).sum();
        let needing: Vec<&&App> = theirs.iter().filter(|a| a.needs_you()).collect();
        let max_risk = theirs.iter().filter_map(|a| a.running.as_ref().and_then(|r| self.shell.risk.of_app(&r.manifest))).max();
        let (unit, _) = v.zoom.per();
        format!(
            r#"{}<div class="c-legend d"><span>Square = app</span><span>Brighter = busier</span><span class="a">Amber = needs a look</span><span class="r">Coral = broken or off</span><span>Hollow = not running</span></div><div class="c-list c-map">{rows}</div><span class="c-grow"></span>{}{}"#,
            self.head(
                "Map",
                &format!(r#"<span class="c">By group</span><span>{} · {}</span>"#, plural(groups.len(), "group"), plural(self.apps.len(), "app")),
                &format!(r#"<span class="d">Last {}</span>"#, escape(&v.zoom.range()))
            ),
            readouts(&[
                (if mine.is_empty() { "No group".to_string() } else { mine.clone() }, theirs.len().to_string(), "apps".to_string(), Ink::White),
                (format!("Req{unit}"), self.rate(reqs), format!("all {}", theirs.len()), Ink::Cyan),
                (
                    "Needs you".to_string(),
                    needing.len().to_string(),
                    needing.first().map_or("none".to_string(), |a| a.name.clone()),
                    if needing.is_empty() { Ink::Dim } else { Ink::Coral }
                ),
                ("Risk max".to_string(), max_risk.map_or("—".to_string(), |r| r.to_string()), "of its apps".to_string(), Ink::Blue),
            ]),
            hint("↑↓ app · ↵ or click = its story · click a group = only its apps · Esc the list"),
        )
    }
}

/// A limit's steps: the usual ones an admin may set (the ends of `range`
/// among them), the app's default and what it has now.
fn steps(usual: &[u32], range: &std::ops::RangeInclusive<u32>, now: u32, default: u32) -> Vec<u32> {
    let mut out: Vec<u32> = usual.iter().copied().filter(|x| range.contains(x)).chain([*range.start(), *range.end(), now, default]).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// A fader: a limit in steps, with its default marked.
struct Fader<'f> {
    label: &'f str,
    what: &'f str,
    steps: Vec<u32>,
    now: u32,
    default: u32,
    /// The form field it posts.
    name: &'f str,
    /// An admin set it.
    set: bool,
    text: fn(u32) -> String,
    note: String,
}

impl Fader<'_> {
    fn html(&self) -> String {
        let n = self.steps.len();
        let at = self.steps.iter().position(|&x| x == self.now).unwrap_or(0);
        let def = self.steps.iter().position(|&x| x == self.default).unwrap_or(0);
        let pos = |k: usize| (k as f64 + 0.5) * 100.0 / n as f64;
        let zones: String = self
            .steps
            .iter()
            .enumerate()
            .rev()
            .map(|(k, &x)| format!(r#"<button name="{}" value="{x}" tabindex="-1" aria-hidden="true" data-i="{k}"></button>"#, self.name))
            .collect();
        let ticks: String = (0..n).map(|k| format!(r#"<i class="{}" style="--y: {:.1}%"></i>"#, if k == def { "def" } else { "" }, pos(k))).collect();
        let labels: Vec<String> = self.steps.iter().map(|&x| (self.text)(x)).collect();
        let hints: Vec<String> = labels.iter().map(|l| format!("{}: {l} per request · set when it rests", self.what)).collect();
        let value = (self.text)(self.now);
        let whose = if self.set { format!("set by an admin, default {}", (self.text)(self.default)) } else { "the default".to_string() };
        let hint = format!("{}: {value} per request ({whose}); {}; drag or click the scale, it changes at once · ↑↓ when focused", self.what, self.note);
        format!(
            r#"<div class="c-fdr{set}" role="slider" tabindex="0" data-fader="{name}" data-at="{at}" aria-label="{what} per request" aria-valuemin="0" aria-valuemax="{max}" aria-valuenow="{at}" aria-valuetext="{value}" data-labels="{labels}" data-hints="{hints}" data-hint="{hint}"><span class="c-silk">{label}</span><span class="c-slot" style="--p: {p:.1}%">{zones}{ticks}<span class="c-cap"></span></span><span class="c-val">{value}</span></div>"#,
            set = if self.set { " set" } else { "" },
            label = self.label,
            what = self.what,
            max = n - 1,
            value = escape(&value),
            name = self.name,
            labels = escape(&serde_json::to_string(&labels).unwrap_or_default()),
            hints = escape(&serde_json::to_string(&hints).unwrap_or_default()),
            hint = escape(&hint),
            p = pos(at),
        )
    }
}

/// A cell's time: "07:19", "07:15–07:20", "07:00–08:00", "Mon 10 Nov".
/// A day is named by its middle, so a day an hour short or long in a
/// daylight-saving change still reads as itself.
fn cell_label(at: i64, cell: i64) -> String {
    match cell {
        c if c <= MINUTE => hm(at),
        c if c < DAY => format!("{}–{}", hm(at), hm(at + c)),
        c => local(at + c / 2, "%a %-d %b"),
    }
}

/// A readout's colour for the slowest against the limit: amber close to it.
fn near(slowest: u64, limit: u64) -> Ink {
    if slowest as f64 >= limit as f64 * CLOSE { Ink::Amber } else { Ink::Cyan }
}

fn unit_word(zoom: Zoom) -> &'static str {
    match zoom {
        Zoom::Min => "minute",
        Zoom::Five => "5 minutes",
        Zoom::Hour => "hour",
        Zoom::Day => "day",
    }
}

/// An app's limits per request for its row, as its faders read: memory,
/// time and fuel, cyan where an admin set them.
fn limit_cells(a: &App) -> String {
    let (now, set) = (a.budget(), a.limits);
    let default = Budget::with(a.toml_time(), AdminLimits::default());
    let mb = |b: usize| format!("{} MB", b >> 20);
    let s = |d: std::time::Duration| format!("{} s", d.as_secs());
    [
        ("Memory", mb(now.memory), mb(default.memory), set.memory_mb.is_some()),
        ("Time", s(now.time), s(default.time), set.seconds.is_some()),
        ("Fuel", super::format::fuel(now.fuel), super::format::fuel(default.fuel), set.fuel_m.is_some()),
    ]
    .into_iter()
    .map(|(what, value, default, set)| {
        let (ink, whose) = if set { ("c", format!("set by an admin, default {default}")) } else { ("d", "the default".to_string()) };
        format!(r#"<span class="c-lim {ink}" data-hint="{what} per request: {value}, {whose}">{value}</span>"#)
    })
    .collect()
}

/// What is true now, and what is left for you, with the keys that do it.
fn verdict(a: &App, owner: &str) -> (String, String) {
    let live = a.running.as_ref().map_or("—".to_string(), |r| r.manifest.app.version.clone());
    let failing = a.problems.iter().any(|p| p.kind == ProblemKind::Failing);
    let Some(p) = a.problems.first() else {
        return ("Nothing is wrong now.".to_string(), "Nothing left for you.".to_string());
    };
    match p.kind {
        ProblemKind::RolledBack if !failing => (
            format!("The platform rolled back; {live} runs. Nothing is broken now."),
            format!("Left for you: tell {owner}; new deploys wait until you make a version live (V); mark it seen (S)."),
        ),
        ProblemKind::RolledBack => {
            (format!("The platform rolled back, and {live} fails too."), format!("Left for you: look at its requests (R), turn it off (X), tell {owner}."))
        }
        ProblemKind::TurnedOff => {
            (format!("{} is off: visitors get the “turned off” page.", a.name), format!("Left for you: turn it on (X) when it's fixed; tell {owner}."))
        }
        ProblemKind::NotRunning => {
            ("The live version didn't start.".to_string(), "Left for you: see the shell's output, or make another version live (V).".to_string())
        }
        ProblemKind::Failing => (
            format!("Failing now: {}.", uncapitalize(&p.line)),
            format!("Left for you: look at its requests (R); roll back (V) or turn it off (X); tell {owner}."),
        ),
        ProblemKind::Busy => (format!("Turning requests away: {}.", uncapitalize(&p.line)), format!("Left for you: look at its requests (R); tell {owner}.")),
        ProblemKind::NearLimit => {
            (format!("Close to its limits: {}.", uncapitalize(&p.line)), format!("Left for you: tell {owner}: faster code, or more seconds in app.toml."))
        }
        ProblemKind::Refused => {
            ("A deploy was refused; visitors don't notice.".to_string(), format!("Left for you: tell {owner} why: {}.", uncapitalize(&p.line)))
        }
        ProblemKind::Unrated => (format!("Risk unknown: {}.", uncapitalize(&p.line)), "Left for you: rate the grant in the risk file.".to_string()),
    }
}

/// What `new` has that `old` hasn't, and what it loses.
fn diff<'a>(old: &'a [String], new: &'a [String]) -> (Vec<&'a str>, Vec<&'a str>) {
    let gains = new.iter().filter(|x| !old.contains(x)).map(String::as_str).collect();
    let loses = old.iter().filter(|x| !new.contains(x)).map(String::as_str).collect();
    (gains, loses)
}

/// "gains a, b; loses c", or "" when nothing changes.
fn diff_words((gains, loses): &(Vec<&str>, Vec<&str>), gain: &str, lose: &str) -> String {
    let mut parts = vec![];
    if !gains.is_empty() {
        parts.push(format!("{gain} {}", gains.join(", ")));
    }
    if !loses.is_empty() {
        parts.push(format!("{lose} {}", loses.join(", ")));
    }
    parts.join("; ")
}

/// `p` against the live version: what to know first, then grants, risk,
/// routes, owner and group, its requests this hour, and the data.
fn compare(risk: &Risk, p: &Version, live: Option<&Version>, a: &App, now: i64) -> String {
    let level = |x: &Version| risk.of_grants(&x.grants).map_or("?".to_string(), |l| l.to_string());
    let mut lines = vec![];
    // What to know before it goes live.
    if p.status == VersionStatus::Failed {
        lines.push(format!(
            r#"<div class="a">{} was rolled back because it kept failing: make it live again only if what made it fail is fixed elsewhere.</div>"#,
            escape(&p.version)
        ));
    }
    if let Some(e) = &a.paused {
        lines.push(format!(
            r#"<div class="a">This ends the pause: {} rolled itself back at {}, so deploys wait; once a version is live here, they go live again.</div>"#,
            escape(&a.name),
            clock(e.at)
        ));
    }
    match live {
        Some(l) => {
            lines.push(format!(r#"<div class="d">{} compared with live {}</div>"#, escape(&p.version), escape(&l.version)));
            let grants = diff(&l.grants, &p.grants);
            let words = match diff_words(&grants, "gains", "loses") {
                w if w.is_empty() => format!("Same grants: {}", if p.grants.is_empty() { "none".to_string() } else { p.grants.join(", ") }),
                w => capitalize(&w),
            };
            let ink = if grants.0.is_empty() { "" } else { r#" class="a""# };
            lines.push(format!(r#"<div{ink}>{} · risk {} → {}</div>"#, escape(&words), level(l), level(p)));
            let routes = diff(&l.routes, &p.routes);
            if !routes.0.is_empty() || !routes.1.is_empty() {
                let dropped = if routes.1.is_empty() { "" } else { "; dropped routes answer 404" };
                lines.push(format!("<div>Routes: {}{dropped}</div>", escape(&diff_words(&routes, "adds", "drops"))));
            }
            for (what, was, becomes) in [("Owner", &l.owner, &p.owner), ("Group", &l.group, &p.group)] {
                if was != becomes {
                    lines.push(format!(r#"<div class="a">{what}: {} → {}</div>"#, escape(&who(was)), escape(&who(becomes))));
                }
            }
        }
        None => lines.push(format!(r#"<div class="d">{} · nothing is live</div>"#, escape(&p.version))),
    }
    let reqs: Vec<&Request> = a.reqs.iter().filter(|r| *r.label == p.label && r.at >= now - HOUR).collect();
    if !reqs.is_empty() {
        let slowest = reqs.iter().filter(|r| !r.outcome.failed()).map(|r| r.wall_us).max().unwrap_or(0);
        let fails = reqs.iter().filter(|r| r.outcome.failed()).count();
        lines.push(format!("<div>This hour: {} · slowest {} s · {} failed</div>", plural(reqs.len(), "request"), secs(slowest), fails));
    }
    lines.push(r#"<div class="d">The data stays: it belongs to the app, not the version.</div>"#.to_string());
    format!(r#"<div class="c-box">{}</div>"#, lines.concat())
}

/// One line on a stretch of requests: a route that got slow since the live
/// version, what failed, or that all is fine.
fn insight(a: &App, reqs: &[Request], limit: u64) -> String {
    if reqs.is_empty() {
        return String::new();
    }
    let mean = |rs: &[&Request]| if rs.is_empty() { 0 } else { rs.iter().map(|r| r.wall_us).sum::<u64>() / rs.len() as u64 };
    // The slowest route here, on the version the requests ran on, against the
    // same route on the version before it this hour.
    let mut routes: BTreeMap<(&str, &str), Vec<&Request>> = BTreeMap::new();
    for r in reqs.iter().filter(|r| !r.outcome.failed()) {
        routes.entry((&r.route, &r.label)).or_default().push(r);
    }
    if let Some(((route, label), rs)) = routes.iter().max_by_key(|(_, rs)| mean(rs)) {
        let now = mean(rs);
        let before: Vec<&Request> = a.reqs.iter().filter(|r| *r.route == **route && *r.label != **label && !r.outcome.failed()).collect();
        let then = mean(&before);
        let since = a.versions.iter().find(|v| v.label == *label);
        if then > 0 && now >= then * 3 / 2 && now - then >= 100_000 {
            let since = since.map(|v| format!(" since {} ({})", v.version, hm(v.deployed_at))).unwrap_or_default();
            return format!(r#"<div class="c-insight a">{}: {} s on average{since}, {} s before</div>"#, escape(route), secs(now), secs(then));
        }
    }
    let fails: Vec<&Request> = reqs.iter().filter(|r| r.outcome.failed()).collect();
    if let Some(f) = fails.first() {
        let why = f.error.as_deref().map(|e| format!(": {}", first_line(e))).unwrap_or_default();
        return format!(r#"<div class="c-insight r">{} failed; the newest on {}{}</div>"#, fails.len(), escape(&f.route), escape(&why));
    }
    let slowest = reqs.iter().map(|r| r.wall_us).max().unwrap_or(0);
    format!(r#"<div class="c-insight c">All fine · the slowest took {} of the {} limit</div>"#, duration(slowest), duration(limit))
}

/// The console's look: a dark-blue device in the dark scheme, a pale one in
/// the light; the screen, knobs and LEDs are the same in both. Its type is
/// on the admin's scale and its spacing on the 4px grid, like the rest.
pub(super) const CSS: &str = r#"
.console { --c-hi: light-dark(#e9eef3, #26323f); --c-lo: light-dark(#d5dee7, #1b242e); --c-edge: light-dark(#b4c2d0, #313f4f);
  --c-well: light-dark(#c9d4de, #121921); --c-silk: light-dark(#2e4257, #a9bacb); --c-silk-dim: light-dark(#4c6076, #8496aa);
  --c-key-hi: light-dark(#fbfcfd, #3b4959); --c-key-lo: light-dark(#e2e8ee, #2d3947); --c-key-ink: light-dark(#21364b, #cad7e4);
  --c-key-base: light-dark(#9fb0c1, #151c24); --c-val: light-dark(#13283d, #dfeaf5);
  --c-lcd: #050a10; --c-ink: #d6edff; --c-cyan: #74cdff; --c-dim: #6f87a3; --c-rule: #1c2a39; --c-sel: #2a6fd6;
  --c-amber: #ffb547; --c-coral: #ff6f66; --c-blue: #8ab8ff;
  /* Focus: a ring of its own, apart from the cyan of hover. */
  --c-focus: light-dark(#13283d, #fff);
  /* A screen row; the list shows ROWS of them (console.rs). */
  --c-row: 24px; --c-rows: 12;
  /* A slide switch: its track's length and width, its nub's length. */
  --c-track: 72px; --c-track-w: 24px; --c-nub: 36px;
  display: flex; flex-direction: column; gap: 12px; scroll-margin-top: 16px; }
/* What the script hides stays hidden, whatever its display. */
.console [hidden] { display: none !important; }
.console :focus-visible { outline: 2px solid var(--c-focus); outline-offset: 3px; }

/* The ticker: what needs you, one at a time, on a strip of screen. */
.c-ticker { display: flex; align-items: center; gap: 8px; }
.c-tick { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; height: 40px; padding: 0 16px; border-radius: 8px;
  background: #0a0f15; box-shadow: inset 0 1px 3px rgba(0, 0, 0, .9), 0 1px 0 rgba(255, 255, 255, .05); color: #dbe5ee;
  font: 500 13.5px/1 var(--mono); white-space: nowrap; overflow: hidden; }
.c-tick b { font-weight: 600; color: #fff; }
.c-idx { color: #8d9cad; }
.c-said { min-width: 0; overflow: hidden; text-overflow: ellipsis; color: #b9c9d9; }
.c-tacts { display: flex; gap: 8px; }
.c-frozen { display: inline-flex; align-items: center; height: 28px; padding: 0 12px; border-radius: 6px; background: var(--c-amber); color: #3a2400;
  font: 700 12px/1 var(--mono); letter-spacing: .1em; text-transform: uppercase; }
.c-load { display: inline-flex; align-items: center; gap: 8px; margin-left: 4px; }
.c-leds { display: inline-flex; gap: 4px; }

/* The device. */
.c-device { position: relative; padding: 20px 32px 24px; border-radius: 20px; border: 1px solid var(--c-edge); color: var(--c-silk);
  background: linear-gradient(180deg, var(--c-hi), var(--c-lo));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, .08), inset 0 -2px 0 rgba(0, 0, 0, .2), 0 2px 0 light-dark(#a3b2c2, #0b1016), 0 20px 48px light-dark(rgba(40, 60, 90, .22), rgba(0, 0, 0, .5)); }
.c-screw { position: absolute; width: 10px; height: 10px; border-radius: 50%; background: radial-gradient(circle at 35% 30%, #8a9bb0, #2a3440 70%); }
.c-screw::after { content: ""; position: absolute; left: 2px; right: 2px; top: 4px; height: 1.5px; background: #0d1218; transform: rotate(35deg); }
.c-screw.tl { left: 12px; top: 12px; } .c-screw.tr { right: 12px; top: 12px; } .c-screw.bl { left: 12px; bottom: 12px; } .c-screw.br { right: 12px; bottom: 12px; }
.c-top { display: flex; align-items: baseline; gap: 16px; margin: 0 8px 16px; }
.c-model { font: 700 15px/1 var(--mono); letter-spacing: .3em; color: var(--c-val); }
.c-silk { font: 600 12px/1.2 var(--mono); letter-spacing: .12em; text-transform: uppercase; color: var(--c-silk); }
.c-silk.dim { font-weight: 500; letter-spacing: .04em; text-transform: none; color: var(--c-silk-dim); }
.c-grow { flex: 1; }
.c-body { display: grid; grid-template-columns: 196px minmax(0, 1fr) 160px; gap: 24px; align-items: start; }
.c-grp { display: flex; justify-content: space-between; align-items: baseline; gap: 8px; margin-bottom: 12px; }
.c-group { margin: 24px 0 8px; }
.c-val { font: 500 12px/1 var(--mono); color: var(--c-val); white-space: nowrap; max-width: 100%; overflow: hidden; text-overflow: ellipsis; }

/* Encoders: an LED ring (or detent dots) round a knob whose pointer turns with --a. */
.c-enc { display: flex; flex-direction: column; align-items: center; gap: 8px; min-width: 0; max-width: 100%; text-align: center; }
.c-big { display: flex; justify-content: center; }
.c-knobs { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); row-gap: 16px; justify-items: center; }
.c-left .c-knobs { margin-top: 16px; }
.c-ring { --p: clamp(11deg, var(--v), 259deg); display: block; padding: 4px; border-radius: 50%; touch-action: none; cursor: grab;
  background: conic-gradient(from 225deg, #2b3644 0 calc(var(--p) - 11deg), var(--c-cyan) calc(var(--p) - 11deg) calc(var(--p) + 11deg), #2b3644 calc(var(--p) + 11deg) 270deg, transparent 270deg 360deg); }
.c-ring.lit, .c-det.lit { box-shadow: 0 0 16px rgba(116, 205, 255, .35); }
.c-det { position: relative; display: grid; place-items: center; width: 74px; height: 74px; border-radius: 50%; touch-action: none; cursor: grab; }
.c-dots { position: absolute; inset: 0; }
.c-det.l { width: 128px; height: 128px; }
.c-dots circle { fill: light-dark(#9aabbd, #3a4757); }
.c-dots circle.c { fill: light-dark(#a7c0d8, #2f5f8a); }
.c-dots circle.a { fill: var(--c-amber); }
.c-dots circle.r { fill: var(--c-coral); }
.c-dots circle.d { fill: none; stroke: light-dark(#7d8a99, #4d6580); stroke-width: 1px; }
.c-dots circle.on { fill: var(--c-cyan); stroke: none; filter: drop-shadow(0 0 3px var(--c-cyan)); }
.c-det.l .c-dots circle.on { r: 3.6px; fill: light-dark(#0a7cff, #fff); filter: drop-shadow(0 0 3px var(--c-cyan)) drop-shadow(0 0 6px var(--c-cyan)); }
.c-det.l .c-dots circle.hov { r: 4px; stroke: light-dark(#1b232c, #e8f2fb); stroke-width: 1.5px; }
.c-knob { --a: 0deg; --size: 40px; --tip: 4px; --arc: 6px; position: relative; display: block; width: var(--size); height: var(--size); border-radius: 50%;
  background: radial-gradient(circle at 50% 32%, #4f5e70, #2a3440 58%, #1b232c);
  box-shadow: 0 0 0 3px var(--c-lo), 0 6px 10px rgba(0, 0, 0, .5), inset 0 1px 0 rgba(255, 255, 255, .2); }
/* The arc: from the knob's start to its pointer, between the cap and its LEDs. */
.c-knob::before { content: ""; position: absolute; inset: calc(-1 * var(--arc)); border-radius: 50%; pointer-events: none;
  background: conic-gradient(from 225deg, light-dark(#0a7cff, var(--c-cyan)) calc(var(--a) + 135deg), transparent 0);
  mask: radial-gradient(closest-side, transparent calc(100% - 3px), #000 calc(100% - 2.5px)); }
.c-ring .c-knob::before { content: none; }
/* The pointer turns round the cap's centre. */
.c-knob::after { content: ""; position: absolute; left: 50%; top: var(--tip); width: 3px; height: 11px; margin-left: -1.5px; border-radius: 2px; background: #e8f2fb;
  transform-origin: 50% calc(var(--size) / 2 - var(--tip)); transform: rotate(var(--a)); }
.c-knob.l { --size: 92px; --tip: 8px; --arc: 7px; }
.c-knob.l::after { width: 4px; height: 22px; margin-left: -2px; }
.c-ring:focus-visible, .c-det:focus-visible { outline-offset: 4px; }
.c-ring.turning, .c-det.turning { cursor: grabbing; }

/* Keys: a link or a button; the keyboard's letter in the corner. A key
   stands --rest above its base and sinks to 1px when pressed; its drop
   shadow and the hover ring follow, whatever kind of key it is. */
.c-key { --rest: 4px; --lift: var(--rest); --drop: 0 6px 12px light-dark(rgba(40, 60, 90, .2), rgba(0, 0, 0, .45)); --ring: 0 0 transparent;
  position: relative; display: inline-flex; flex-direction: column; align-items: center; justify-content: center; gap: 4px;
  min-width: 0; height: 48px; padding: 0 4px; border: 0; border-radius: 8px; background: linear-gradient(180deg, var(--c-key-hi), var(--c-key-lo));
  box-shadow: 0 var(--lift) 0 var(--c-key-base), var(--ring), var(--drop), inset 0 1px 0 rgba(255, 255, 255, .12);
  color: var(--c-key-ink); font: 600 12px/1 var(--mono); letter-spacing: .08em; text-transform: uppercase; text-align: center; cursor: pointer; user-select: none;
  -webkit-touch-callout: none; }
.c-key.flat { --rest: 3px; --drop: 0 0 transparent; height: 40px; }
/* The ring goes round the key and its base, so its bottom edge shows. */
.c-key:hover, .c-key.pg.here { --ring: 0 var(--lift) 0 2px var(--c-cyan), 0 0 0 2px var(--c-cyan); }
.c-key:active, .c-key.down, .c-key.pg.here { --lift: 1px; --drop: 0 0 transparent; transform: translateY(calc(var(--rest) - 1px)); }
/* A key with nothing to do now: dim, and it doesn't answer the pointer. */
.c-key.off { --lift: var(--rest); --ring: 0 0 transparent; transform: none; opacity: .45; cursor: default; }
.c-kh { position: absolute; top: 4px; right: 8px; font: 500 12px/1 var(--mono); letter-spacing: 0; text-transform: none; opacity: .45; }
.c-arrow { width: 40px; }
.c-tacts .c-key { padding: 0 16px; }
.c-keyform { display: contents; }
/* Browse's two views, side by side; the right column, one key a row. A page
   key's LED is lit on its page. */
.c-keys2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; margin-top: 24px; }
.c-col { display: flex; flex-direction: column; gap: 12px; }
.c-key.pg::before { content: ""; position: absolute; top: 8px; left: 8px; width: 8px; height: 8px; border-radius: 50%;
  background: var(--c-glint), radial-gradient(circle at 50% 60%, light-dark(#5d6b7b, #24303d), light-dark(#3f4b58, #121920)); box-shadow: inset 0 1px 2px rgba(0, 0, 0, .6); }
.c-this { display: block; margin: -4px 0 12px; font-size: 13.5px; }

/* LEDs: a tinted lens with a glint when off, a hot core and a glow when lit. */
.console { --c-glint: radial-gradient(circle at 34% 30%, rgba(255, 255, 255, .55) 0 .8px, rgba(255, 255, 255, 0) 2px); }
.c-led { display: inline-block; flex: none; width: 10px; height: 10px; border-radius: 50%;
  background: var(--c-glint), radial-gradient(circle at 50% 60%, light-dark(#5d6b7b, #24303d), light-dark(#3f4b58, #121920));
  box-shadow: inset 0 1px 2px rgba(0, 0, 0, .6), 0 1px 0 light-dark(rgba(255, 255, 255, .7), rgba(255, 255, 255, .07)); }
.c-led.sq { width: 10px; height: 8px; border-radius: 2px; }
/* A lit LED: its colour's core, body and edge, and its glow; a page key's
   LED is a blue one. */
.c-led:is(.blue, .amber, .red, .orange, .green), .c-key.pg.here::before { --shade: transparent;
  background: var(--c-glint), radial-gradient(circle at 45% 40%, var(--core) 0 1.5px, var(--body) 3px, var(--edge) 6px);
  box-shadow: 0 0 6px var(--glow), 0 0 14px var(--halo), inset 0 -1px 1px var(--shade); }
.c-led.blue, .c-key.pg.here::before { --core: #f2faff; --body: #9ed6ff; --edge: #3f9cf5; --glow: #4aa8ff; --halo: rgba(74, 168, 255, .55); --shade: rgba(0, 40, 90, .35); }
.c-led.amber { --core: #fff8ec; --body: #ffcf80; --edge: #f59a1e; --glow: #ffa52e; --halo: rgba(255, 165, 46, .5); --shade: rgba(90, 40, 0, .35); }
.c-led.red { --core: #fff1f0; --body: #ff9b93; --edge: #ec4a3f; --glow: #ff5a4f; --halo: rgba(255, 90, 79, .55); --shade: rgba(90, 0, 0, .35); }
/* The step whose requests the screen shows, the pin whose app it shows. */
.c-led.orange { --core: #fff3ea; --body: #ffae7a; --edge: #ff6b1f; --glow: #ff7a2e; --halo: rgba(255, 122, 46, .55); --shade: rgba(90, 30, 0, .35); }
.c-led.green { --core: #f0fff5; --body: #8ff0b4; --edge: #22c46a; --glow: #35d67a; --halo: rgba(53, 214, 122, .5); }

/* The screen: a bezel and a dark-blue LCD with a faint line texture; the
   same height on every page, so the device doesn't jump. */
.c-mid { min-width: 0; }
.c-bezel { padding: 8px; border-radius: 10px; background: #0a0f15; box-shadow: inset 0 1px 3px rgba(0, 0, 0, .9), 0 1px 0 rgba(255, 255, 255, .06); }
.c-lcd { display: flex; flex-direction: column; height: 488px; padding: 8px; border-radius: 6px; overflow: hidden; color: var(--c-ink);
  background-color: var(--c-lcd); background-image: repeating-linear-gradient(0deg, rgba(120, 190, 255, .025) 0 1px, transparent 1px 3px);
  box-shadow: inset 0 0 24px rgba(40, 110, 200, .12); font: 500 13.5px/1 var(--mono); text-transform: uppercase; letter-spacing: .02em; }
.c-lcd { --c-focus: #fff; }
.c-lcd ::selection { background: var(--c-sel); color: #fff; }
.c-lcd .c { color: var(--c-cyan); } .c-lcd .w { color: #fff; } .c-lcd .a { color: var(--c-amber); } .c-lcd .r { color: var(--c-coral); }
.c-lcd .d { color: var(--c-dim); } .c-lcd .b { color: var(--c-blue); }
.c-lcd a:hover { text-decoration: underline; text-underline-offset: 3px; }
.c-foot { display: flex; justify-content: space-between; gap: 16px; margin-top: 8px; }
.c-head { display: flex; align-items: baseline; gap: 12px; padding: 0 4px 8px; white-space: nowrap; overflow: hidden; }
.c-tag { padding: 0 4px; background: var(--c-ink); color: var(--c-lcd); }
.c-clock { color: #fff; }
.c-colhead { height: 20px; font-size: 12px; }
.c-colhead a:hover { color: var(--c-ink); }
.c-listwrap { display: flex; gap: 4px; min-height: 0; }
.c-list { flex: 1; min-width: 0; max-height: calc(var(--c-rows) * var(--c-row)); overflow-y: auto; scrollbar-width: none; }
.c-list::-webkit-scrollbar { display: none; }
/* A toast takes two rows' room; the readouts below stay put. */
.c-toast ~ .c-listwrap { --c-rows: 10; }
.c-row { display: grid; grid-template-columns: 28px minmax(0, 1.2fr) minmax(0, 1.2fr) 6ch 4ch 5ch 80px 158px 56px; align-items: center; gap: 8px; height: var(--c-row); padding: 0 4px;
  white-space: nowrap; }
.c-row > * { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
.c-row.sel { background: var(--c-sel); }
.c-row.sel, .c-row.sel > *, .c-row.sel .d { color: #fff; }
.c-row.cursor:not(.sel) { box-shadow: inset 0 0 0 1px var(--c-cyan); }
.c-row.changed { animation: c-changed 1.6s ease-out; }
@keyframes c-changed { from { background: rgba(116, 205, 255, .35); } to { background: transparent; } }
@media (prefers-reduced-motion: reduce) { .c-row.changed { animation: none; } }
.c-rate, .c-lim { text-align: right; }
.c-cells { display: flex; gap: 2px; overflow: visible; }
.c-cells i { flex: none; width: 8px; height: 12px; background: #16222f; }
.c-cells i.k1, .c-sq.k1 { background: #2b5a85; } .c-cells i.k2, .c-sq.k2 { background: #4b94d1; } .c-cells i.k3, .c-sq.k3 { background: #8fd6ff; }
.c-cells i.ka, .c-sq.ka { background: var(--c-amber); } .c-cells i.kr, .c-sq.kr { background: var(--c-coral); }
.c-cells i:hover { outline: 2px solid #fff; outline-offset: 1px; }
.c-sep { display: flex; align-items: center; gap: 12px; height: var(--c-row); padding: 0 4px; font-size: 12px; }
.c-sep::before, .c-sep::after { content: ""; flex: 1; border-top: 1px dashed #2a3a4d; }
.c-mini { flex: none; width: 12px; height: calc(var(--c-rows) * var(--c-row)); }
.c-mini .c-track { fill: #101a25; } .c-mini .r { fill: var(--c-coral); } .c-mini .a { fill: var(--c-amber); } .c-mini .d { fill: #4d6580; }
.c-mini .c-win { fill: none; stroke: var(--c-cyan); stroke-width: 1.5px; vector-effect: non-scaling-stroke; }
.c-none { margin: 24px 4px; color: var(--c-dim); }
.c-none a { color: var(--c-cyan); }
.c-readouts { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; padding: 0 4px; }
.c-readout { display: flex; flex-direction: column; gap: 4px; min-width: 0; padding-top: 4px; border-top: 3px solid var(--c-sel); }
.c-readout > * { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.c-readout b { font: 500 28px/1 var(--mono); }
.c-readout.c { border-color: var(--c-cyan); } .c-readout.r { border-color: var(--c-coral); } .c-readout.a { border-color: var(--c-amber); }
.c-readout.b { border-color: var(--c-blue); } .c-readout.d { border-color: #2a3a4d; } .c-readout.w b { color: #fff; }
.c-rl { font-size: 12px; color: var(--c-dim); }
.c-hint { margin-top: 8px; padding: 8px 4px 0; border-top: 1px solid var(--c-rule); font-size: 12px; color: var(--c-dim); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.c-toast { display: flex; align-items: center; gap: 12px; margin: 0 0 8px; padding: 4px 8px; background: var(--c-cyan); color: var(--c-lcd); }
.c-toast.r { background: var(--c-coral); color: var(--c-lcd); }
/* Undo: a dark key on the bright toast. */
.c-undo button { min-height: 24px; height: 24px; padding: 0 8px; border-radius: 4px; background: var(--c-lcd); color: var(--c-cyan);
  font: 600 12px/1 var(--mono); letter-spacing: .06em; text-transform: uppercase; }
.c-undo button:hover { background: #10304e; }
.c-toast { --c-focus: var(--c-lcd); }
/* Find, in the list's first line. */
.c-find { display: flex; align-items: center; gap: 8px; flex: 0 1 280px; min-width: 0; align-self: center; }
.c-find label { color: var(--c-dim); }
.c-find input { flex: 1; min-width: 0; height: 24px; padding: 0 8px; border: 0; border-radius: 0; background: #0f1a26; color: var(--c-cyan);
  font: 500 13.5px/1 var(--mono); text-transform: none; box-shadow: inset 0 -1px 0 var(--c-cyan); }
/* A change: why, the usual reasons, the button that does it. */
.c-act { display: flex; flex-direction: column; gap: 12px; margin-top: 8px; padding: 12px; border: 1px solid #2a3a4d; text-transform: none; }
.c-actrow { display: flex; align-items: center; gap: 16px; white-space: nowrap; }
.c-actrow > .d { overflow: hidden; text-overflow: ellipsis; }
.c-btn { height: 32px; padding: 0 16px; border: 0; border-radius: 4px; background: var(--c-cyan); color: var(--c-lcd); font: 600 13.5px/1 var(--mono); cursor: pointer; }
.c-btn:hover { background: #fff; }
.c-lcd .c-cancel { color: var(--c-ink); text-decoration: underline; text-underline-offset: 3px; }
.c-box { margin-top: 8px; padding: 8px 12px; border: 1px solid var(--c-rule); line-height: 1.6; text-transform: none; }
.c-box > div { overflow: hidden; text-overflow: ellipsis; }
.c-insight { margin-top: 8px; padding: 4px; border-top: 1px dashed #2a3a4d; text-transform: none; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.c-lines { display: flex; flex-direction: column; overflow: hidden; }
.c-line { display: grid; grid-template-columns: 120px minmax(0, 1fr); gap: 12px; padding: 4px; line-height: 1.3; text-transform: none; }
.c-line > span:first-child { white-space: nowrap; }
.c-verdict { display: flex; flex-direction: column; gap: 4px; margin-top: 8px; padding: 8px 4px 0; border-top: 1px dashed #2a3a4d; line-height: 1.4; text-transform: none; }
.c-req { grid-template-columns: 72px minmax(0, 1fr) 96px 48px 120px; }
.c-reqs { max-height: 216px; }
.c-bar1 { display: block; height: 8px; background: #16222f; }
.c-bar1 i { display: block; height: 100%; background: var(--c-cyan); }
.c-bar1 i.a { background: var(--c-amber); } .c-bar1 i.r { background: var(--c-coral); }
.c-ver { grid-template-columns: minmax(0, 1fr) 132px minmax(0, 2fr); }
.c-vers { flex: 0 1 auto; max-height: 144px; }
.c-legend { display: flex; flex-wrap: wrap; gap: 4px 16px; padding: 0 4px 8px; font-size: 12px; text-transform: none; }
.c-map { max-height: 264px; }
.c-mrow { display: grid; grid-template-columns: 128px 32px minmax(0, 1fr) auto; align-items: center; gap: 8px; min-height: 24px; padding: 4px; }
.c-mrow.sel { box-shadow: inset 0 0 0 1px var(--c-sel); }
.c-mrow > a, .c-mrow > span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.c-sqs { display: flex; flex-wrap: wrap; gap: 4px; }
.c-sq { width: 14px; height: 14px; background: #16222f; }
.c-sq.off { background: transparent; box-shadow: inset 0 0 0 1.5px #4d6580; }
.c-sq.cur { box-shadow: 0 0 0 2px var(--c-lcd), 0 0 0 4px #fff; }
.c-sq.cursor:not(.cur) { box-shadow: 0 0 0 2px var(--c-lcd), 0 0 0 4px var(--c-cyan); }

/* The well: pins (with the script) and the step keys, one per cell. */
.c-well { margin-top: 24px; padding: 12px 20px 16px; border-radius: 14px; background: var(--c-well);
  box-shadow: inset 0 2px 6px light-dark(rgba(40, 60, 90, .25), rgba(0, 0, 0, .6)), 0 1px 0 rgba(255, 255, 255, .05); }
.c-well .c-grp { margin-bottom: 8px; }
/* Pins need the script: it marks the page, so a swapped-in console has them at once. */
.c-pins { display: none; margin-bottom: 12px; }
:root[data-script] .c-pins { display: block; }
.c-pinkeys { display: grid; grid-template-columns: repeat(8, minmax(0, 1fr)); gap: 12px; }
.c-pin { align-items: stretch; justify-content: space-between; height: 56px; padding: 8px 12px; text-transform: none; letter-spacing: .02em; }
.c-pinrow { display: flex; justify-content: space-between; align-items: center; }
.c-pinno { opacity: .55; }
.c-pinname { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; text-align: left; }
.c-pin:not([data-app]) .c-pinname { opacity: .5; font-weight: 500; }
.c-steps { display: grid; grid-template-columns: repeat(16, minmax(0, 1fr)); gap: 8px; }
.c-step { display: flex; flex-direction: column; align-items: center; gap: 8px; min-width: 0; }
.c-step .c-key { width: 100%; height: 32px; padding: 0; }

/* Faders: a slot, a scale with the default longer, a cap at --p from the
   bottom. Each step is a link; admin.js drags the cap. */
.c-faders { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 4px; }
.c-fdr { display: flex; flex-direction: column; align-items: center; gap: 8px; min-width: 0; border-radius: 8px; outline-offset: 4px; }
.c-slot { --p: 50%; position: relative; display: flex; flex-direction: column; width: 44px; height: 136px; touch-action: none; cursor: ns-resize; }
.c-slot::before { content: ""; position: absolute; left: calc(50% - 3px); top: 4px; bottom: 4px; width: 6px; border-radius: 3px;
  background: light-dark(#26313d, #0a0f15); box-shadow: inset 0 1px 2px #000, 0 1px 0 light-dark(rgba(255, 255, 255, .8), rgba(255, 255, 255, .06)); }
.c-fdrf { display: contents; }
/* The admin's buttons are boxes; a fader's steps and the switch are not. */
.c-slot button { position: relative; z-index: 1; flex: 1; min-height: 0; padding: 0; border: 0; border-radius: 0; background: none; box-shadow: none; cursor: pointer; }
/* A tick each side of the slot, the default's longer: mirrored across it. */
.c-slot i { --x: 4px; --w: 8px; position: absolute; left: var(--x); bottom: var(--y); width: var(--w); height: 2px; transform: translateY(50%); border-radius: 1px;
  background: var(--c-silk-dim); opacity: .6; box-shadow: calc(44px - 2 * var(--x) - var(--w)) 0 0 var(--c-silk-dim); }
.c-slot i.def { --x: 0px; --w: 12px; opacity: 1; background: var(--c-silk); box-shadow: calc(44px - 2 * var(--x) - var(--w)) 0 0 var(--c-silk); }
.c-cap { position: absolute; z-index: 2; left: 50%; bottom: var(--p); width: 36px; height: 20px; transform: translate(-50%, 50%); border-radius: 4px; pointer-events: none;
  background: linear-gradient(180deg, #6a7b8f, #3a4858 48%, #2c3846); box-shadow: 0 5px 8px rgba(0, 0, 0, .45), inset 0 1px 0 rgba(255, 255, 255, .3); }
.c-cap::after { content: ""; position: absolute; left: 6px; right: 6px; top: 9px; height: 2px; border-radius: 1px; background: #e8f2fb; }
.c-fdr.set .c-cap::after { background: var(--c-cyan); box-shadow: 0 0 6px var(--c-cyan); }
.c-fdr.set .c-val { color: light-dark(#0a62c4, var(--c-cyan)); }
.c-fdr.turning .c-cap, .c-fdr:hover .c-cap { box-shadow: 0 5px 8px rgba(0, 0, 0, .45), 0 0 0 2px var(--c-cyan), inset 0 1px 0 rgba(255, 255, 255, .3); }

/* Below: the well (pins, step keys) and the platform panel beside it. */
.c-lower { display: grid; grid-template-columns: minmax(0, 1fr) 232px; gap: 24px; margin-top: 24px; }
.c-lower .c-well { margin-top: 0; }
.c-plat { display: flex; flex-direction: column; padding: 12px 20px 16px; border-radius: 14px; background: var(--c-well);
  box-shadow: inset 0 2px 6px light-dark(rgba(40, 60, 90, .25), rgba(0, 0, 0, .6)), 0 1px 0 rgba(255, 255, 255, .05); }
.c-plat .c-grp { margin-bottom: 8px; }
/* Both controls centred in the same band, their labels on one line below. */
.c-plat-row { flex: 1; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); align-content: center; justify-items: center; gap: 16px; }
.c-ctl { display: grid; grid-template-rows: calc(var(--c-track) + 16px) auto; justify-items: center; align-items: center; row-gap: 12px; }
/* The power button: a round key in an LED ring, green while the apps are
   on, red once Stop all turned them off. */
.c-power { --glow: #35d67a; --glow-a: rgba(53, 214, 122, .45); position: relative; display: grid; place-items: center; width: 64px; height: 64px; margin-bottom: 4px;
  padding: 0; border: 0; border-radius: 50%; cursor: pointer; color: var(--glow); background: radial-gradient(circle at 50% 30%, #56677b, #2c3846 60%, #1d2630);
  box-shadow: 0 0 0 4px var(--c-well), 0 0 0 7px var(--glow), 0 0 18px 6px var(--glow-a), 0 6px 0 4px #0d1218, inset 0 1px 0 rgba(255, 255, 255, .25); }
.c-power.off { --glow: #ff5a4f; --glow-a: rgba(255, 90, 79, .45); }
.c-power svg { width: 28px; height: 28px; fill: none; stroke: currentColor; stroke-width: 2.6; stroke-linecap: round; filter: drop-shadow(0 0 4px currentColor); }
.c-power:hover { box-shadow: 0 0 0 4px var(--c-well), 0 0 0 7px var(--glow), 0 0 24px 8px var(--glow-a), 0 6px 0 4px #0d1218, inset 0 1px 0 rgba(255, 255, 255, .3); }
.c-power:active { transform: translateY(4px);
  box-shadow: 0 0 0 4px var(--c-well), 0 0 0 7px var(--glow), 0 0 18px 6px var(--glow-a), 0 2px 0 4px #0d1218, inset 0 1px 0 rgba(255, 255, 255, .15); }
/* The selected app's on/off: a slide switch with an LED at each end, green
   on the left while on, red on the right once off. */
.c-apwr { display: flex; justify-content: center; margin-top: 20px; }
.c-slide { --travel: calc(var(--c-track) - var(--c-nub) - 4px); position: relative; display: flex; flex-direction: column; align-items: center; gap: 12px;
  padding: 0 12px; border: 0; border-radius: 8px; min-height: 0; background: none; box-shadow: none; color: inherit; font: inherit; cursor: pointer; }
.c-slide .c-kh { top: 0; right: -16px; }
.c-ends { display: flex; justify-content: space-between; width: calc(var(--c-track) + 16px); }
.c-slide .c-track { position: relative; width: var(--c-track); height: var(--c-track-w); border-radius: 12px; background: #0e141b;
  box-shadow: inset 0 2px 4px rgba(0, 0, 0, .8), 0 1px 0 rgba(255, 255, 255, .06); }
.c-nub { position: absolute; top: 2px; left: 2px; width: var(--c-nub); height: calc(var(--c-track-w) - 4px); border-radius: 10px;
  background: repeating-linear-gradient(90deg, #b9c3cd 0 2px, #7a8592 2px 4px);
  box-shadow: 0 2px 4px rgba(0, 0, 0, .6), inset 0 1px 0 rgba(255, 255, 255, .5); transition: transform .12s ease-out; }
.c-slide.off .c-nub { transform: translateX(var(--travel)); }
.c-slide:hover .c-track { box-shadow: inset 0 2px 4px rgba(0, 0, 0, .8), 0 0 0 2px var(--c-cyan); }
/* Focus rings the track, the part that moves. */
.c-slide:focus-visible { outline: none; }
.c-slide:focus-visible .c-track { outline: 2px solid var(--c-focus); outline-offset: 3px; }
.c-slide.dead { cursor: default; }
.c-slide.dead:hover .c-track { box-shadow: inset 0 2px 4px rgba(0, 0, 0, .8), 0 1px 0 rgba(255, 255, 255, .06); }
.c-slide.dead .c-nub { opacity: .45; }
@media (prefers-reduced-motion: reduce) { .c-nub { transition: none; } }
/* Freeze: the same slide, upright, LEDs beside its ends: up and amber holds
   deploys, down and green lets them go live. */
.c-slide.v { flex-direction: row; }
/* As wide as the LEDs, on the other side: the track stands over its label. */
.c-slide.v::after { content: ""; width: 10px; }
.c-slide.v .c-kh { top: 0; right: 0; }
.c-slide.v .c-ends { flex-direction: column; width: auto; height: calc(var(--c-track) + 16px); }
.c-slide.v .c-track { width: var(--c-track-w); height: var(--c-track); }
.c-slide.v .c-nub { width: calc(var(--c-track-w) - 4px); height: var(--c-nub); background: repeating-linear-gradient(0deg, #b9c3cd 0 2px, #7a8592 2px 4px); }
.c-slide.v.off .c-nub { transform: translateY(var(--travel)); }

/* Narrower: the screen on top, browse and the keys side by side below. */
@media (max-width: 1100px) {
  .c-body { grid-template-columns: 196px minmax(0, 1fr); grid-template-areas: "mid mid" "left right"; }
  .c-mid { grid-area: mid; } .c-left { grid-area: left; } .c-right { grid-area: right; }
  .c-right .c-col { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); }
  .c-faders { max-width: 320px; }
  .c-lower { grid-template-columns: minmax(0, 1fr); }
  .c-plat-row { grid-template-columns: repeat(2, 120px); justify-content: start; }
}
/* Phones: one column; group, limit and number columns go, each row's cells get a line of their own, two rows of step keys. */
@media (max-width: 700px) {
  .c-device { padding: 16px 12px 16px; border-radius: 16px; }
  .c-screw, .c-wide, .c-load { display: none; }
  .c-body { grid-template-columns: minmax(0, 1fr); grid-template-areas: "mid" "left" "right"; }
  .c-ticker { flex-wrap: wrap; }
  .c-tick { order: -1; flex-basis: 100%; padding: 0 12px; }
  .c-lcd { height: auto; min-height: 488px; }
  .c-head { flex-wrap: wrap; row-gap: 4px; }
  .c-row:not(.c-req):not(.c-ver) { grid-template-columns: minmax(0, 1fr) 72px 40px; grid-template-areas: "name st rate" "cells cells cells";
    height: auto; gap: 4px 8px; padding: 4px; }
  .c-row:not(.c-req):not(.c-ver) > .c-name { grid-area: name; } .c-row:not(.c-req):not(.c-ver) > .c-st { grid-area: st; }
  .c-row:not(.c-req):not(.c-ver) > .c-rate { grid-area: rate; } .c-row:not(.c-req):not(.c-ver) > .c-cells { grid-area: cells; }
  .c-row .c-n, .c-row .c-lim, .c-row .c-gcol { display: none; }
  .c-cells i { flex: 1 1 0; width: auto; }
  .console { --c-rows: 18; } .c-mini { height: auto; }
  .c-req { grid-template-columns: 64px minmax(0, 1fr) 72px 40px; }
  .c-req .c-bar1, .c-req > :last-child { display: none; }
  .c-ver { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
  .c-ver > :last-child { display: none; }
  .c-readouts { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .c-readout b { font-size: 20px; }
  .c-mrow { grid-template-columns: 96px minmax(0, 1fr); }
  .c-mrow > .c-rate, .c-mrow > :last-child:not(.c-sqs) { display: none; }
  .c-line { grid-template-columns: 88px minmax(0, 1fr); }
  .c-well { padding: 12px; }
  .c-pinkeys { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  .c-steps { grid-template-columns: repeat(8, minmax(0, 1fr)); row-gap: 12px; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_is_its_query_and_back() {
        let pairs = |q: &str| form_urlencoded::parse(q.as_bytes()).into_owned().collect::<Vec<_>>();
        assert_eq!(View::parse(&[]), View::default());
        assert_eq!(View::default().query(), "");
        let v = View::parse(&pairs("page=log&sort=group&zoom=5m&group=Customer+service&q=quo&v=0.1.0-ab&at=60000"));
        assert_eq!((v.page, v.sort, v.zoom, v.at), (Page::Log, Sort::Group, Zoom::Five, Some(60_000)));
        assert_eq!(View::parse(&pairs(&v.query())), v);
        // Unknown values fall back; the toast needs both.
        let odd = View::parse(&pairs("page=nope&zoom=2h&done=5"));
        assert_eq!((odd.page, odd.zoom, odd.done), (Page::List, Zoom::Min, None));
        for at in [i64::MIN, i64::MAX] {
            let invalid = View::parse(&pairs(&format!("at={at}&done={at}&msg=invalid")));
            assert_eq!((invalid.at, invalid.done), (None, None));
        }
        // The toast's Undo is an event's id, or nothing.
        let done = |q: &str| View::parse(&pairs(&format!("done=60000&msg=ok&{q}"))).done.unwrap();
        assert_eq!((done("undo=42").undo, done("undo=rm").undo, done("err=1").failed), (Some(42), None, true));
        // A link to the longest version label or app id comes back whole.
        let long = "a".repeat(255);
        assert_eq!(View::parse(&pairs(&format!("v={long}"))).version.as_deref(), Some(long.as_str()));
        // Another page forgets what the screen asked, not the group.
        let to = v.to(Page::Map);
        assert_eq!((to.page, to.version, to.at), (Page::Map, None, None));
        assert_eq!(to.group.as_deref(), Some("Customer service"));
        assert_eq!(View::default().path(Some("a b")), "/admin/apps/a+b");
    }

    #[test]
    fn step_keys_count_back_from_now() {
        assert_eq!(Zoom::Min.step(0), "Now");
        // The row's label says "minutes ago", "days ago": the keys are bare numbers.
        assert_eq!(Zoom::Min.step(3), "3");
        assert_eq!(Zoom::Five.step(3), "15");
        assert_eq!(Zoom::Day.step(2), "2");
    }

    #[test]
    fn the_screen_says_versions() {
        assert_eq!(ver("0.3.1-ab12cd"), "0.3.1");
        assert_eq!(ver("1.0.0-beta-ab12cd"), "1.0.0-beta");
    }

    /// A row shows the app's limits per request, cyan where an admin set one.
    #[tokio::test]
    async fn a_row_shows_the_apps_limits() {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        crate::deploy::deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let now = crate::platform::now_ms();
        let (mut apps, _) = super::super::model::load(&shell, now);
        let list = Board::new(&shell, &apps, &View::default(), now, None).html();
        assert!(list.contains(r#"<span class="c-lim">Mem</span><span class="c-lim">Time</span><span class="c-lim">Fuel</span>"#), "{list}");
        assert!(list.contains(r#"<span class="c-lim d" data-hint="Memory per request: 64 MB, the default">64 MB</span>"#), "{list}");
        assert!(list.contains(r#"<span class="c-lim d" data-hint="Fuel per request: 1 B, the default">1 B</span>"#), "{list}");
        apps[0].limits.seconds = Some(10);
        let list = Board::new(&shell, &apps, &View::default(), now, None).html();
        assert!(list.contains(r#"<span class="c-lim c" data-hint="Time per request: 10 s, set by an admin, default 2 s">10 s</span>"#), "{list}");
        assert!(!list.contains("c-owner"), "{list}");
    }

    #[test]
    fn a_knob_with_steps_points_at_its_lit_dot() {
        let knob = |current| {
            Knob {
                label: "Zoom",
                value: String::new(),
                sub: "",
                at: None,
                size: "s",
                href: String::new(),
                letter: "",
                param: "zoom",
                stops: Zoom::ALL
                    .iter()
                    .map(|z| Stop { value: z.name().to_string(), label: z.range(), href: format!("/{}", z.name()), hint: String::new() })
                    .collect(),
                current,
                detents: true,
                hint: String::new(),
                lit: false,
                lights: vec![],
            }
            .html()
        };
        assert!(knob(0).contains("--a: -135deg"), "{}", knob(0));
        assert!(knob(1).contains("--a: -45deg"), "{}", knob(1));
        assert!(knob(3).contains("--a: 135deg"), "{}", knob(3));
        // Its values, labels and pages are JSON arrays, for admin.js; to
        // assistive tech it is a slider.
        assert!(knob(0).contains(r#"data-values="[&quot;1m&quot;,&quot;5m&quot;,&quot;1h&quot;,&quot;1d&quot;]""#), "{}", knob(0));
        assert!(knob(1).contains(r#"data-hrefs="[&quot;/1m&quot;,&quot;/5m&quot;,&quot;/1h&quot;,&quot;/1d&quot;]""#), "{}", knob(1));
        assert!(knob(1).contains(r#"role="slider" aria-valuemin="0" aria-valuemax="3" aria-valuenow="1""#), "{}", knob(1));
        assert_eq!(Zoom::ALL.map(Zoom::range), ["16 min", "80 min", "16 h", "16 days"]);
    }

    #[test]
    fn the_app_knob_lights_the_selected_app() {
        let knob = |n: usize, current: usize| {
            Knob {
                label: "App",
                value: String::new(),
                sub: "",
                at: None,
                size: "l",
                href: String::new(),
                letter: "",
                param: "app",
                stops: (0..n).map(|i| Stop { value: i.to_string(), label: i.to_string(), href: String::new(), hint: String::new() }).collect(),
                current,
                detents: false,
                hint: String::new(),
                lit: false,
                lights: (0..n.min(LIGHTS))
                    .map(|k| Light { ink: Ink::Cyan, from: k * n / n.min(LIGHTS), at: k * n / n.min(LIGHTS), hint: format!("{k}") })
                    .collect(),
            }
            .html()
        };
        let four = knob(4, 2);
        assert_eq!(four.matches("<circle").count(), 4);
        assert!(four.contains("--a: 45deg") && four.matches(r#"class="c on""#).count() == 1, "{four}");
        assert!(four.contains(r#"data-from="3" data-i="3" data-hint="3""#), "{four}");
        // A hundred apps share 36 lights; the last app lights the last one.
        let many = knob(100, 99);
        assert_eq!(many.matches("<circle").count(), LIGHTS);
        assert!(many.contains("--a: 135deg"), "{many}");
        // Each app lights the light that stands for it: of 100 apps, light 0
        // stands for apps 0–1 and light 1 for apps 2–4.
        let lit = |html: &str| html.split("<circle").position(|c| c.contains(r#"class="c on""#)).map(|i| i - 1);
        assert_eq!(lit(&knob(100, 1)), Some(0));
        assert_eq!(lit(&knob(100, 3)), Some(1));
        for current in 0..100 {
            let k = lit(&knob(100, current)).unwrap();
            assert!(k * 100 / LIGHTS <= current && current < (k + 1) * 100 / LIGHTS, "{current} lights {k}");
        }
    }

    /// Every page and zoom renders against a real shell; changes are forms
    /// that come back to the console.
    #[tokio::test]
    async fn every_page_renders_and_changes_come_back_here() {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let first = crate::deploy::deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        crate::deploy::deploy(&shell, &crate::testing::bundle("2.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let now = crate::platform::now_ms();
        let (apps, _) = super::super::model::load(&shell, now);
        let html = |view: View| Board::new(&shell, &apps, &view, now, None).html();
        let list = html(View::default());
        assert!(list.contains("APPOS-1") && list.contains("Nothing needs you"), "{list}");
        assert_eq!(list.matches(r#"class="c-step""#).count(), CELLS);
        assert!(list.contains(r#"data-app="test""#) && list.contains(r#"href="/admin/apps/test?page=app#console""#));
        // A row's group is words, not a link: a click anywhere on the row selects.
        assert!(list.contains(r#"<span class="c-gcol b">Tests</span>"#) && !list.contains("data-tone") && !list.contains("data-c="), "{list}");
        assert!(list.contains(r#"<i class="" data-hint=""#), "{list}");
        // The App knob: a light for the one app, lit; pushed, the next app, which is itself.
        assert!(list.contains(r#"class="c-det l" href="/admin/apps/test#console""#) && list.contains(r#"<circle class="c on""#), "{list}");
        for page in Page::ALL {
            for zoom in Zoom::ALL {
                let out = html(View { page, zoom, ..View::default() });
                assert!(out.contains(&format!(r#"data-page="{}""#, page.name())), "{page:?}");
            }
        }
        let versions = html(View { page: Page::Versions, version: Some(first.app.label.clone()), ..View::default() });
        assert!(
            versions.contains(r#"action="/admin/apps/test/rollback""#) && versions.contains(&format!(r#"name="label" value="{}""#, first.app.label)),
            "{versions}"
        );
        assert!(versions.contains(r#"name="then" value="/admin/apps/test""#) && versions.contains("Roll back to 1.0</button>"), "{versions}");
        // Without a pick, the versions page shows the version a rollback goes to.
        assert!(html(View { page: Page::Versions, ..View::default() }).contains("Roll back to 1.0</button>"));
        // Controls act at once, like on a device: no reason asked anywhere.
        let list = html(View::default());
        assert!(list.contains(r#"action="/admin/freeze"><input type="hidden" name="frozen" value="1">"#), "{list}");
        assert!(list.contains(r#"action="/admin/stop-all"><input type="hidden" name="then" value="/admin/apps/test">"#), "{list}");
        assert!(list.contains(r#"action="/admin/apps/test/off""#) && list.contains(r#"<button name="memory" value="128""#), "{list}");
        assert!(!list.contains("reason") && !versions.contains("reason"), "{list}");
        // The app's slide is on while it serves; keys carry one word, no explanations.
        assert!(list.contains(r#"<button class="c-slide" role="switch" aria-checked="true""#) && !list.contains("<small>"), "{list}");
        // The step LEDs: only the one whose requests the screen shows is lit, orange.
        assert!(!list.contains("c-led orange"), "{list}");
        let shown = html(View { page: Page::Log, ..View::default() });
        assert_eq!(shown.matches("c-led orange").count(), 1, "{shown}");
        // GROUP narrows the list; a group nobody has leaves it empty.
        assert!(html(View { group: Some("tests".into()), ..View::default() }).contains(r#"<span class="c-gcol b">Tests</span>"#));
        assert!(html(View { group: Some("Sales".into()), ..View::default() }).contains("No app matches"));
        // The toast shows on every page; Undo only while the server can
        // reverse the change, and it names the change by its event.
        let off = shell.state.turn("test", false, Actor::Admin, None, None).await.unwrap();
        let now = crate::platform::now_ms();
        let (apps, _) = super::super::model::load(&shell, now);
        let html = |view: View| Board::new(&shell, &apps, &view, now, None).html();
        let done = |at: i64, undo: i64, page: Page| View {
            page,
            done: Some(Done { msg: "Test is off".into(), at, undo: Some(undo), failed: false }),
            ..View::default()
        };
        for page in Page::ALL {
            let out = html(done(now, off, page));
            assert!(out.contains(r#"action="/admin/undo""#) && out.contains(&format!(r#"name="event" value="{off}""#)), "{page:?}: {out}");
        }
        assert!(!html(done(now - UNDO_MS - 1, off, Page::List)).contains("c-toast"));
        // A change by the CLI, or one a later change took back, has no Undo.
        let on = shell.state.turn("test", true, Actor::Admin, None, None).await.unwrap();
        let deployed = shell.platform.events(Some("test"), 10).unwrap().into_iter().find(|e| e.kind == Kind::Deployed).unwrap().id;
        let now = crate::platform::now_ms();
        let (apps, _) = super::super::model::load(&shell, now);
        let undo = |id: i64| Board::new(&shell, &apps, &done(now, id, Page::List), now, None).html().contains("U Undo");
        assert_eq!((undo(on), undo(off), undo(deployed)), (true, false, false));
    }

    /// Every console page, rendered: each page in both sorts, a change
    /// done and one without an answer, a find with no match, a version
    /// picked, and the requests page with requests. Two apps, one off (it
    /// needs you), deploys frozen: the ticker's item, the list's separators
    /// and Frozen show.
    async fn every_page() -> Vec<(View, String)> {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        let first = crate::deploy::deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        crate::deploy::deploy(&shell, &crate::testing::bundle("2.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let other = crate::bundle::Bundle {
            manifest: "[app]\nid = 'other'\nname = 'Other'\nversion = '1.0'\nowner = 'Ops <ops@example.com>'\ngroup = 'Ops'\n".into(),
            ..crate::testing::bundle("1.0", None)
        };
        crate::deploy::deploy(&shell, &other.pack().unwrap(), Actor::Cli).await.unwrap();
        let off = shell.state.turn("other", false, Actor::Admin, None, None).await.unwrap();
        shell.state.freeze(true, None).await.unwrap();
        let now = crate::platform::now_ms();
        let (apps, _) = super::super::model::load(&shell, now);
        let done = |failed| Done { msg: "Other is off".into(), at: now, undo: Some(off), failed };
        let mut views: Vec<View> = Page::ALL.iter().flat_map(|&page| [Sort::Needs, Sort::Group].map(|sort| View { page, sort, ..View::default() })).collect();
        views.extend([
            View { done: Some(done(false)), ..View::default() },
            View { done: Some(done(true)), ..View::default() },
            View { find: Some("nothing like it".into()), ..View::default() },
            View { page: Page::Versions, version: Some(first.app.label.clone()), ..View::default() },
        ]);
        let mut pages: Vec<(View, String)> = views.into_iter().map(|v| (v.clone(), Board::new(&shell, &apps, &v, now, Some("test")).html())).collect();
        let view = View { page: Page::Log, ..View::default() };
        let mut log = Board::new(&shell, &apps, &view, now, Some("test"));
        let request =
            |outcome| Request { at: now, label: first.app.label.clone().into(), route: "/".into(), outcome, wall_us: 1_000, fuel: 1, io: 0, error: None };
        log.requests = (vec![request(Outcome::Ok), request(Outcome::ServerError)], 2);
        pages.push((view.clone(), log.html()));
        pages
    }

    /// The script's modules, one after another.
    fn script() -> String {
        super::super::MODULES.iter().map(|(_, body)| *body).collect()
    }

    /// The opening tags in `html` with the attribute `name`, from < to >
    /// (the shell escapes > in attribute values).
    fn tags<'h>(html: &'h str, name: &str) -> Vec<&'h str> {
        html.match_indices(&format!(" {name}=\""))
            .map(|(i, _)| {
                let start = html[..i].rfind('<').unwrap();
                &html[start..=i + html[i..].find('>').unwrap()]
            })
            .collect()
    }

    /// An attribute's value in a tag, unescaped.
    fn attr(tag: &str, name: &str) -> Option<String> {
        let (_, rest) = tag.split_once(&format!(" {name}=\""))?;
        let value = &rest[..rest.find('"')?];
        Some(value.replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&"))
    }

    /// An attribute holding a JSON list of texts.
    fn list_attr(tag: &str, name: &str) -> Vec<String> {
        let value = attr(tag, name).unwrap_or_else(|| panic!("no {name} in {tag}"));
        serde_json::from_str(&value).unwrap_or_else(|e| panic!("{name}: {e}: {value}"))
    }

    fn number(tag: &str, name: &str) -> usize {
        attr(tag, name).and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("no number {name} in {tag}"))
    }

    /// The words in `text` that start with `prefix` (letters, digits, -).
    fn words(text: &str, prefix: &str) -> std::collections::BTreeSet<String> {
        text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).filter(|w| w.starts_with(prefix)).map(str::to_string).collect()
    }

    /// Every console class the CSS styles is on a page the console
    /// renders or in its script: no rule is left styling nothing.
    #[tokio::test]
    async fn every_console_class_is_used() {
        let html: String = every_page().await.into_iter().map(|(_, html)| html).collect();
        let used: std::collections::BTreeSet<String> = words(&html, "c-").union(&words(&script(), "c-")).cloned().collect();
        // The CSS's classes, comments left out.
        let rules: String = CSS.split("/*").map(|part| part.split_once("*/").map_or(part, |(_, rest)| rest)).collect();
        let styled: std::collections::BTreeSet<String> = rules
            .split('.')
            .skip(1)
            .filter_map(|part| {
                let name: String = part.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
                name.starts_with("c-").then_some(name)
            })
            .collect();
        assert!(styled.len() > 80, "{styled:?}");
        let unused: Vec<&String> = styled.iter().filter(|c| !used.contains(*c)).collect();
        assert!(unused.is_empty(), "styled but never used: {unused:?}");
    }

    /// What the script looks for is on the pages: each console class it
    /// names, each data-* it reads. A rename on one side fails here.
    #[tokio::test]
    async fn the_script_finds_what_it_looks_for() {
        let html: String = every_page().await.into_iter().map(|(_, html)| html).collect();
        let script = script();
        // Classes, in selectors and classList calls alike.
        let missing: Vec<String> = words(&script, "c-").into_iter().filter(|c| !words(&html, "c-").contains(c)).collect();
        assert!(missing.is_empty(), "the script names classes no page has: {missing:?}");
        // data-* it reads: [data-x in selectors, .dataset.x, json(el, "x").
        let kebab =
            |name: &str| name.chars().flat_map(|c| if c.is_ascii_uppercase() { vec!['-', c.to_ascii_lowercase()] } else { vec![c] }).collect::<String>();
        let after = |marker: &str| -> Vec<String> {
            script
                .match_indices(marker)
                .map(|(i, _)| script[i + marker.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>())
                .filter(|n| !n.is_empty())
                .collect()
        };
        let mut read: std::collections::BTreeSet<String> = after("[data-").into_iter().collect();
        read.extend(after("dataset.").iter().map(|n| kebab(n)));
        read.extend(script.match_indices("json(").filter_map(|(i, _)| {
            let (args, _) = script[i..].split_once(')')?;
            let (_, name) = args.split_once(", \"")?;
            Some(name.trim_end_matches('"').to_string())
        }));
        // The script's own (on <html>), and #live's icon (the page around
        // the console; mod.rs checks it).
        for own in ["script", "live", "icon"] {
            assert!(read.remove(own), "{own}");
        }
        assert!(read.len() > 20, "{read:?}");
        let rendered = words(&html, "data-");
        let missing: Vec<&String> = read.iter().filter(|n| !rendered.contains(&format!("data-{n}"))).collect();
        assert!(missing.is_empty(), "the script reads data-* no page has: {missing:?}");
    }

    /// A knob carries its stops (value, label, page, hint), where it is and
    /// its ARIA; a fader its steps, one button each in a form that posts.
    #[tokio::test]
    async fn knobs_and_faders_carry_their_steps() {
        for (view, html) in every_page().await {
            let knobs = tags(&html, "data-knob");
            assert!(knobs.len() >= 2, "{view:?}");
            for k in knobs {
                let values = list_attr(k, "data-values");
                let n = values.len();
                // Find left no app: App still shows the selected one, with no stops.
                assert!(n > 0 || view.find.is_some(), "{k}");
                for name in ["data-labels", "data-hrefs", "data-hints"] {
                    assert_eq!(list_attr(k, name).len(), n, "{name} in {k}");
                }
                let at = number(k, "data-at");
                assert!(k.contains(r#" role="slider""#) && k.contains(r#" aria-valuemin="0""#), "{k}");
                assert_eq!((number(k, "aria-valuemax"), number(k, "aria-valuenow")), (n.saturating_sub(1), at), "{k}");
                if n > 0 {
                    assert_eq!(attr(k, "aria-valuetext").unwrap(), list_attr(k, "data-labels")[at], "{k}");
                }
                assert!(list_attr(k, "data-hrefs").iter().all(|h| h.starts_with("/admin") && !h.contains('#')), "{k}");
                assert!(attr(k, "href").unwrap().ends_with("#console"), "a push is a console link: {k}");
            }
            // The App knob's lights: each stands for the apps from its
            // data-from on, in order.
            let lights: Vec<(usize, usize)> = tags(&html, "data-from").iter().map(|c| (number(c, "data-from"), number(c, "data-i"))).collect();
            assert!(view.find.is_some() || lights.first().is_some_and(|l| *l == (0, 0)), "{view:?}: {lights:?}");
            assert!(lights.windows(2).all(|w| w[0].0 < w[1].0 && w[1].1 == w[0].1 + 1), "{lights:?}");
            for (i, f) in tags(&html, "data-fader").into_iter().enumerate() {
                let n = list_attr(f, "data-labels").len();
                assert_eq!(list_attr(f, "data-hints").len(), n, "{f}");
                let at = number(f, "data-at");
                assert_eq!((number(f, "aria-valuemax"), number(f, "aria-valuenow")), (n - 1, at), "{f}");
                assert_eq!(attr(f, "aria-valuetext").unwrap(), list_attr(f, "data-labels")[at], "{f}");
                // Its form: from the fader's <form> to the next </form>.
                let start = html.match_indices(r#"<form class="c-fdrf""#).nth(i).unwrap().0;
                let form = &html[start..start + html[start..].find("</form>").unwrap()];
                assert!(form.contains(r#" method="post""#) && attr(form, "action").unwrap().ends_with("/limits"), "{form}");
                let steps: Vec<usize> = tags(form, "data-i").iter().filter(|b| b.starts_with("<button")).map(|b| number(b, "data-i")).collect();
                assert_eq!(steps, (0..n).rev().collect::<Vec<_>>(), "a button per step, top first: {form}");
            }
        }
    }

    /// Each page says where it is (one page key current, the console's own
    /// data) and carries the words the script fills in.
    #[tokio::test]
    async fn each_page_says_where_it_is() {
        for (view, html) in every_page().await {
            let console = tags(&html, "data-page")[0];
            assert_eq!(attr(console, "data-page").unwrap(), view.page.name());
            for name in ["data-app", "data-name", "data-failed"] {
                assert!(attr(console, name).is_some_and(|v| !v.is_empty()), "{name}: {console}");
            }
            assert_eq!(html.matches(r#" aria-current="page""#).count(), 1, "{view:?}");
            assert_eq!(html.matches(r#" data-key="Escape""#).count(), 1, "Esc and / need the List key: {view:?}");
            assert_eq!(tags(&html, "data-default").len(), 1, "the hint line: {view:?}");
            let steps = html.matches(r#"class="c-key flat down" href"#).count();
            assert_eq!(steps, usize::from(view.page == Page::Log), "the step key of the minute shown: {view:?}");
            for slide in tags(&html, "aria-checked") {
                assert!(matches!(attr(slide, "aria-checked").as_deref(), Some("true" | "false")) && slide.contains(r#" role="switch""#), "{slide}");
            }
            let pins = tags(&html, "data-jump");
            assert_eq!(pins.len(), 1);
            assert!(attr(pins[0], "data-jump").unwrap().contains("{id}"));
            for name in ["data-set", "data-pinned"] {
                let text = attr(pins[0], name).unwrap();
                assert!(text.contains("{i}") && text.contains("{name}"), "{name}: {text}");
            }
            let numbers: Vec<usize> = tags(&html, "data-pin").iter().map(|p| number(p, "data-pin")).collect();
            assert_eq!(numbers, (1..=8).collect::<Vec<_>>());
            if view.page == Page::List {
                let of = attr(tags(&html, "data-of")[0], "data-of").unwrap();
                assert!(["{first}", "{last}", "{n}"].iter().all(|w| of.contains(w)), "{of}");
            }
        }
    }

    /// A click anywhere on a list row selects its app (data-href), ↵ opens
    /// it (data-open), Find narrows by data-find; the map's squares and the
    /// versions open theirs.
    #[tokio::test]
    async fn rows_carry_what_a_click_needs() {
        for (view, html) in every_page().await {
            let rows: Vec<&str> = tags(&html, "data-href").into_iter().filter(|r| r.starts_with(r#"<div class="c-row"#)).collect();
            if view.page != Page::List {
                assert!(rows.is_empty(), "{view:?}");
            } else if view.find.is_none() {
                assert_eq!(rows.len(), 2, "{view:?}");
            }
            for r in rows {
                let id = attr(r, "data-app").unwrap();
                assert!(attr(r, "data-href").unwrap().starts_with(&format!("/admin/apps/{id}")), "{r}");
                assert!(attr(r, "data-open").unwrap().contains("page=app"), "{r}");
                let find = attr(r, "data-find").unwrap();
                assert!(find == find.to_lowercase() && find.lines().any(|l| l == id), "{r}");
                assert!(attr(r, "data-status").is_some_and(|s| !s.is_empty()), "{r}");
            }
            let opens: Vec<&str> = tags(&html, "data-open").into_iter().filter(|t| !t.starts_with(r#"<div class="c-row"#)).collect();
            match view.page {
                Page::Map => assert_eq!(opens.iter().filter(|t| t.contains("c-sq") && t.contains(" data-app=\"")).count(), 2, "{opens:?}"),
                Page::Versions => assert_eq!(opens.iter().filter(|t| t.contains("c-ver")).count(), 2, "{opens:?}"),
                _ => assert!(opens.is_empty(), "{view:?}: {opens:?}"),
            }
            for o in opens {
                assert!(attr(o, "data-open").unwrap().starts_with("/admin/apps/"), "{o}");
            }
        }
    }

    /// The screen's numbers come from the cells: the row's looks and
    /// hints, and the readouts under the list.
    #[tokio::test]
    async fn the_screen_counts_from_the_cells() {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        crate::deploy::deploy(&shell, &crate::testing::bundle("1.0", None).pack().unwrap(), Actor::Cli).await.unwrap();
        let now = crate::platform::now_ms();
        let (apps, _) = super::super::model::load(&shell, now);
        let view = View::default();
        let mut board = Board::new(&shell, &apps, &view, now, None);
        let mut cells = vec![Cell::default(); CELLS];
        cells[13] = Cell { requests: 30, ..Cell::default() };
        cells[14] = Cell { requests: 9, failures: 2, ..Cell::default() };
        cells[15] = Cell { requests: 12, failures: 1, stops: 1, slowest_us: 4_000_000 };
        board.cells.insert("test".into(), cells);
        let a = board.chosen().unwrap();
        let sums = board.sums(a);
        assert_eq!((sums.requests, sums.failures, sums.stops), (51, 3, 1));
        let looks: Vec<Heat> = board.row_cells(a).iter().map(|(_, h, _)| *h).collect();
        assert_eq!(looks[12..], [Heat::Idle, Heat::High, Heat::Failed, Heat::Near]);
        let (at, _, c) = board.row_cells(a)[15];
        assert_eq!(at, board.start(15));
        let hint = board.cell_hint(a, at, &c);
        assert!(hint.ends_with(&format!(" · 12 requests · 1 failed · 1 at the {} s limit · slowest 4.0 s", secs(a.limit_us()))), "{hint}");
        let out = board.html();
        assert!(out.contains(r#"<i class="kr" data-hint=""#) && out.contains("· 30 requests"), "{out}");
        assert!(out.contains("51 in 16 min") && out.contains("<b>3</b><span class=\"c-rl\">1 at the limit</span>"), "{out}");
        // A day is named by its middle, so a 23-hour day is still itself.
        assert_eq!(cell_label(0, DAY), local(DAY / 2, "%a %-d %b"));
    }

    /// The App knob's marks stay put: an app turned off jumps to the top of
    /// the list, not to another place on the dial.
    #[tokio::test]
    async fn the_dial_keeps_its_order_when_an_app_turns_red() {
        use crate::platform::Actor;
        let dir = tempfile::tempdir().unwrap();
        let shell = crate::testing::shell(dir.path().to_path_buf());
        for (id, name) in [("zeta", "Zeta"), ("alpha", "Alpha"), ("mid", "Mid")] {
            let bundle = crate::bundle::Bundle {
                manifest: format!("[app]\nid = '{id}'\nname = '{name}'\nversion = '1.0'\nowner = 'Tests'\ngroup = 'Tests'\n"),
                ..crate::testing::bundle("1.0", None)
            };
            crate::deploy::deploy(&shell, &bundle.pack().unwrap(), Actor::Cli).await.unwrap();
        }
        let ids = |apps: &[&App]| apps.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        let now = crate::platform::now_ms();
        let (apps, _) = super::super::model::load(&shell, now);
        let view = View::default();
        assert_eq!(ids(&Board::new(&shell, &apps, &view, now, None).dial()), ["alpha", "mid", "zeta"]);
        shell.state.turn("zeta", false, Actor::Admin, None, None).await.unwrap();
        let (apps, _) = super::super::model::load(&shell, now);
        let board = Board::new(&shell, &apps, &view, now, None);
        assert_eq!(ids(&board.order)[0], "zeta", "turned off, it needs you: first in the list");
        assert_eq!(ids(&board.dial()), ["alpha", "mid", "zeta"]);
    }
}
