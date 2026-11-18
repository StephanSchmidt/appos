//! The admin screen's colours, CSS and small visual parts: risk labels and
//! app icons.

use std::sync::LazyLock;

use crate::manifest::{Band, MAX_RISK, Risk};
use crate::platform::Outcome;
use crate::web::escape;

use super::model::{App, Tone};
use super::tokens;

/// The charts' series: volume in the accent, failures in status colours.
pub(super) const SERVED: &str = "var(--accent-9)";
pub(super) const FAILED: &str = "var(--critical-9)";

/// What a failure asks for, as the failures chart groups the kinds, bottom
/// of a stack first: fix the app, make it faster or raise its limit, or
/// spread the load. Status colours, most pressing first; adjacent pairs pass
/// the dataviz CVD and normal-vision floors in both themes.
pub(super) const CAUSES: [(&str, &str); 3] =
    [("Crashed or error page", "var(--critical-9)"), ("Timed out or over budget", "var(--warning-9)"), ("Busy or conflict", "var(--gray-9)")];

/// Failed outcomes, their words, and their cause in CAUSES.
pub(super) const KINDS: &[(Outcome, &str, usize)] = &[
    (Outcome::Crashed, "Crashed", 0),
    (Outcome::ServerError, "Error page (5xx)", 0),
    (Outcome::Timeout, "Timed out", 1),
    (Outcome::OverBudget, "Over budget", 1),
    (Outcome::Busy, "Busy", 2),
    (Outcome::Conflict, "Conflict", 2),
];
/// The slowest request of 2½ minutes, from fast to at the time limit (the
/// heat strip's speed ramp, in tokens.rs; risk has its own).
pub(super) const HEAT: &[&str] = &["var(--heat-1)", "var(--heat-2)", "var(--heat-3)", "var(--heat-4)", "var(--heat-5)", "var(--heat-6)"];
/// A heat cell where a request failed: past the ramp, black in light and
/// near white in dark.
pub(super) const HEAT_FAILED: &str = "light-dark(var(--gray-12), var(--gray-11))";

pub(super) const ROLLBACK_PATH: &str = "M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5";
/// A power sign: the kill switch.
pub(super) const POWER_PATH: &str = "M12 2v10M18.4 6.6a9 9 0 1 1-12.8 0";
/// A pause sign: a deploy held after an automatic rollback.
pub(super) const HELD_PATH: &str = "M6 4h4v16H6zM14 4h4v16h-4z";
/// A play sign: a held version made live by hand.
pub(super) const LIVE_PATH: &str = "M7 4l12 8-12 8z";
/// A stopwatch: a limit hit.
pub(super) const LIMIT_PATH: &str = "M12 21a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM12 9v4l2 2M10 2h4";
/// A shield: risk.
pub(super) const RISK_PATH: &str = "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z";
/// A padlock: the policy, who may do what.
pub(super) const POLICY_PATH: &str = "M7 11V7a5 5 0 0 1 10 0v4M5 11h14v10H5z";
/// An eye: an admin saw it.
pub(super) const SEEN_PATH: &str = "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z";
/// A snowflake: deploys frozen.
pub(super) const FROZEN_PATH: &str = "M12 2v20M2 12h20M4.9 4.9l14.2 14.2M19.1 4.9 4.9 19.1M9 3l3 3 3-3M9 21l3-3 3 3";
pub(super) const REFUSED_PATH: &str = "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM4.9 4.9l14.2 14.2";

/// A tone's colour scale in tokens.rs.
fn scale(tone: Tone) -> &'static str {
    match tone {
        Tone::Critical => "critical",
        Tone::Warning => "warning",
        Tone::Caution => "caution",
        Tone::Info => "accent",
        Tone::Success => "success",
        Tone::Neutral => "gray",
    }
}

/// A tone's icon, so colour is never the only signal: an octagon for
/// critical (stop), a triangle for warning, a circle with ! for caution and
/// with i for info, a check for success, a ring for neutral.
/// A line icon from one of the paths above.
pub(super) fn path_icon(path: &str, size: u32) -> String {
    format!(
        r#"<svg width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="{path}"/></svg>"#
    )
}

pub(super) fn tone_icon(tone: Tone, size: u32) -> String {
    let path = match tone {
        Tone::Critical => "M7.9 2h8.2L22 7.9v8.2L16.1 22H7.9L2 16.1V7.9zM12 8v4M12 16h.01",
        Tone::Warning => "M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0zM12 9v4M12 17h.01",
        Tone::Caution => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 8v4M12 16h.01",
        Tone::Info => "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20zM12 16v-4M12 8h.01",
        Tone::Success => "M20 6 9 17l-5-5",
        Tone::Neutral => "M12 20a8 8 0 1 0 0-16 8 8 0 0 0 0 16z",
    };
    format!(
        r#"<svg width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="{path}"/></svg>"#
    )
}

/// A status as a badge: the tone's icon and a word or two. Base is the tone's
/// tint with its text step (Stripe); strong is the solid fill, for what is
/// urgent.
pub(super) fn badge(tone: Tone, label: &str, strong: bool) -> String {
    let strong = if strong { " strong" } else { "" };
    format!(r#"<span class="badge tone-{}{strong}">{}{}</span>"#, tone.name(), tone_icon(tone, 12), escape(label))
}

/// `.tone-critical` and the others: a tone's steps as `--tone-2` and so on,
/// for the rules below.
fn tones_css() -> String {
    Tone::ALL
        .iter()
        .map(|t| {
            let s = scale(*t);
            format!(
                ".tone-{} {{ --tone-2: var(--{s}-2); --tone-3: var(--{s}-3); --tone-6: var(--{s}-6); --tone-9: var(--{s}-9); --tone-11: var(--{s}-11); --tone-contrast: var(--{s}-contrast); }}\n",
                t.name()
            )
        })
        .collect()
}

const SHIELD: &str = r#"<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>"#;

/// A running app's risk as a badge: a shield, the band and the level, the
/// band's colour on the risk ramp. "Unrated" with a dashed edge while one of
/// its grants has no level.
pub(super) fn risk_label(risk: &Risk, a: &App) -> Option<String> {
    let r = a.running.as_ref()?;
    Some(risk_chip(risk.of_app(&r.manifest)))
}

/// An app's risk level as a badge, or "Unrated".
pub(super) fn risk_chip(level: Option<u8>) -> String {
    level_chip(level, ": the level of its riskiest grant", "A grant it uses has no level in the risk file")
}

/// A grant's risk level as a badge, or "Unrated".
pub(super) fn grant_chip(level: Option<u8>) -> String {
    level_chip(level, "", "No level in the risk file")
}

fn level_chip(level: Option<u8>, why: &str, unrated: &str) -> String {
    match level {
        Some(level) => {
            let band = Band::of(level);
            format!(
                r#"<span class="risk risk-{}" title="Risk {level} of {MAX_RISK}, {}{why}">{SHIELD}{} {level}</span>"#,
                band.name().to_lowercase(),
                band.name().to_lowercase(),
                band.name()
            )
        }
        None => format!(r#"<span class="risk unrated" title="{unrated}">{SHIELD}Unrated</span>"#),
    }
}

/// A grant with its level, coloured by band.
pub(super) fn grant_label(grant: &str, level: Option<u8>) -> String {
    match level {
        Some(level) => {
            let band = Band::of(level);
            format!(
                r#"<span class="grant risk-{}" title="Risk {level} of {MAX_RISK}, {}">{}<b>{level}</b></span>"#,
                band.name().to_lowercase(),
                band.name().to_lowercase(),
                escape(grant)
            )
        }
        None => format!(r#"<span class="grant unrated" title="No level in the risk file">{}<b>?</b></span>"#, escape(grant)),
    }
}

/// The bands with their levels, as a legend.
pub(super) fn risk_legend() -> String {
    Band::ALL.iter().map(|b| format!(r#"<span class="risk risk-{}">{} {}</span>"#, b.name().to_lowercase(), b.name(), b.levels())).collect()
}

/// `.risk-low` and the others.
fn bands_css() -> String {
    Band::ALL.iter().map(|b| format!(".risk-{0} {{ background: var(--risk-{0}); color: var(--risk-{0}-text); }}\n", b.name().to_lowercase())).collect()
}

/// An owner as app.toml names them, like "Jane Doe <jane@example.com>":
/// the name and the address, if there is one.
pub(super) fn mail_of(owner: &str) -> (&str, Option<&str>) {
    let owner = owner.trim();
    match owner.strip_suffix('>').and_then(|o| o.rsplit_once('<')) {
        Some((name, mail)) if is_mail(mail.trim()) => (name.trim(), Some(mail.trim())),
        _ if is_mail(owner) => (owner, Some(owner)),
        _ => (owner, None),
    }
}

/// An owner as app.toml names them, like "Jane Doe <jane@example.com>":
/// the name, linked to write to them when there is an address.
pub(super) fn owner_html(owner: &str) -> String {
    let (name, mail) = mail_of(owner);
    match mail {
        Some(mail) => {
            let name = if name.is_empty() { mail } else { name };
            format!(r#"<a class="owner" href="mailto:{0}" title="Write to {0}">{1}</a>"#, escape(mail), escape(name))
        }
        None => format!(r#"<span class="owner">{}</span>"#, escape(name)),
    }
}

/// A plain address, local@domain.tld, of characters a mailto: link takes
/// as they are.
fn is_mail(s: &str) -> bool {
    let ok = |part: &str, more: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || more.contains(c));
    s.split_once('@').is_some_and(|(local, domain)| ok(local, "._%+-") && ok(domain, ".-") && domain.contains('.'))
}

// ---- icons -----------------------------------------------------------------

pub(super) fn initials(name: &str) -> String {
    let mut words = name.split_whitespace().filter_map(|w| w.chars().next());
    let first: String = words.next().into_iter().chain(words.next()).collect();
    first.to_uppercase()
}

/// The app's icon.svg on a white tile, or its initials on a tile coloured by
/// its id, with rounded corners like a phone's home screen.
pub(super) fn icon_html(app: &App, size: u32) -> String {
    let (inner, bg) = if app.running.as_ref().is_some_and(|a| a.icon.is_some()) {
        (format!(r#"<img src="/admin/icons/{}" alt="" width="{s}" height="{s}">"#, escape(&app.id), s = size * 3 / 5), "#fff".to_string())
    } else {
        (escape(&initials(&app.name)), app.tile.to_string())
    };
    format!(
        r#"<span class="icon" style="width:{size}px;height:{size}px;font-size:{}px;border-radius:{}px;background:{bg}" aria-hidden="true">{inner}</span>"#,
        size * 2 / 5,
        size * 22 / 100,
    )
}

/// The tab's icon: the AppOS mark, with a red dot while something is broken
/// and an amber one while something needs a look, as a data: URL.
pub(super) fn favicon(state: Tone) -> String {
    let dot = match state {
        Tone::Critical => Some(tokens::CRITICAL.step(tokens::Theme::Light, 9)),
        Tone::Warning | Tone::Caution => Some(tokens::WARNING.step(tokens::Theme::Light, 9)),
        _ => None,
    };
    let dot = dot.map(|c| format!("<circle cx='24' cy='24' r='7' fill='{c}' stroke='white' stroke-width='2.5'/>")).unwrap_or_default();
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect x='3' y='3' width='22' height='22' rx='6' fill='#ffe45c' stroke='#2b2a26' stroke-width='2'/>{dot}</svg>"
    );
    let svg = svg.replace('%', "%25").replace('#', "%23").replace('<', "%3C").replace('>', "%3E");
    format!("data:image/svg+xml,{svg}")
}

/// The whole stylesheet: the colour tokens, the tones, the risk bands, then
/// the rules.
/// The page's CSS. The admin's look sits in a layer, so the console's
/// rules, outside it, win over its button and input defaults without
/// selectors made only to outrank them. The font and the page transition
/// stay outside the layer too.
pub(super) static CSS: LazyLock<String> =
    LazyLock::new(|| format!("{OUTSIDE}@layer admin {{\n{}{}{}{RULES}}}\n{}", tokens::css(), tones_css(), bands_css(), super::console::CSS));

const OUTSIDE: &str = r#"
@font-face { font-family: "AppOS Condensed"; font-weight: 800; src: local("HelveticaNeue-CondensedBlack"), local("Helvetica Neue Condensed Black"); }
@media (prefers-reduced-motion: no-preference) { @view-transition { navigation: auto; } }
"#;

const RULES: &str = r#"
/* Fonts the OS has: Helvetica Neue on macOS, Ubuntu Sans on Linux. Colours come from the scales in tokens.rs.
   Calm by default, urgent by exception: hairlines, no hard shadows, depth from the page and the surface; only the
   attention sentence is loud. Corners: cards 12px, panels and buttons 8px, badges 6px.
   One type scale: 12 labels and axes, 13.5 secondary text and tables, 15 body, 20 card titles and the attention
   sentence, 28 the app in detail, 48 the numbers. Spacing on a 4px grid (1-2px only for hairline gaps). */
:root { color-scheme: light dark; --bg: var(--gray-1); --sunk: var(--gray-2); --head: var(--gray-1); --ink: var(--gray-12); --muted: var(--gray-11);
  --faint: var(--gray-9); --line: var(--gray-6); --line-soft: var(--gray-5); --edge: var(--gray-7); --link: var(--accent-11); --focus: var(--accent-9);
  --lemon: #ffe45c; --red: var(--critical-9);
  --sans: "Helvetica Neue", "Ubuntu Sans", "Nimbus Sans", "Liberation Sans", Arial, sans-serif;
  --display: "AppOS Condensed", "Ubuntu Sans", "Nimbus Sans Narrow", "Liberation Sans Narrow", sans-serif;
  --mono: ui-monospace, "SF Mono", Menlo, "Ubuntu Sans Mono", "DejaVu Sans Mono", monospace; }
:root[data-scheme="light"] { color-scheme: light; }
:root[data-scheme="dark"] { color-scheme: dark; }
* { box-sizing: border-box; }
/* Every number in its column: digits all the same width. */
body { margin: 0; background: var(--bg); color: var(--ink); font: 15px/1.5 var(--sans); font-variant-numeric: tabular-nums;
  -webkit-font-smoothing: antialiased; }
a { color: inherit; text-decoration: none; }
/* One focus ring for everything focusable: the accent's step 9, 3:1 on every ground (tokens.rs). */
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
code, pre { font-family: var(--mono); font-size: 12px; }
pre { margin: 0; white-space: pre-wrap; word-break: break-word; }
h1, h2, h3, .brand, .kpi b, .summary > p, .rules li > b, .icon { font-family: var(--display); font-weight: 800; font-stretch: condensed; letter-spacing: -.01em; }
/* Big standalone numbers keep proportional digits; tabular ones only line up columns. */
h1, h2, h3, .kpi b, .summary > p, .rules li > b { font-variant-numeric: proportional-nums; }
.wrap { max-width: 1360px; margin: 0 auto; padding: 24px 40px 64px; display: flex; flex-direction: column; gap: 20px; }
/* The header is quieter than the work below it: small, muted, one current section. */
header { display: flex; flex-wrap: wrap; align-items: center; gap: 12px 20px; font-size: 13.5px; color: var(--muted); }
.brand { display: flex; align-items: center; gap: 12px; font-size: 28px; line-height: 1; color: var(--ink); }
.mark { width: 24px; height: 24px; border-radius: 6px; background: var(--lemon); box-shadow: inset 0 0 0 1.5px var(--dark-text); }
.platform { margin-left: -8px; font-weight: 500; }
.platform::before { content: "/"; margin-right: 12px; color: var(--gray-7); }
nav { display: flex; gap: 4px; }
nav a { padding: 4px 12px; border-radius: 6px; font-weight: 500; }
nav a:hover { color: var(--ink); }
nav a[aria-current="page"] { background: var(--gray-3); color: var(--ink); }
.where { margin-left: auto; display: inline-flex; align-items: center; gap: 12px; }
main { display: flex; flex-direction: column; gap: inherit; }
/* Live, from admin.js: a dot while the stream is open, words when not;
   without the script, only the time. */
.where :is(.live, .lost, .stale) { display: none; align-items: center; gap: 8px; font-weight: 600; }
.where :is(.live, .lost, .stale)::before { content: ""; width: 8px; height: 8px; border-radius: 50%; background: currentColor; }
[data-live="on"] .where .live { display: inline-flex; color: var(--success-11); }
[data-live="off"] .where .lost { display: inline-flex; color: var(--warning-11); }
[data-live="stale"] .where .stale { display: inline-flex; color: var(--caution-11); }
/* Motion (D10): pages cross-fade (OUTSIDE), hover and focus ease in over
   120 ms; nothing else moves, and nothing at all with reduced motion. */
@media (prefers-reduced-motion: no-preference) {
  a, button, .kpi, .risk, .flag, .spark i, .cols > div, input, select, .msg {
    transition: background-color 120ms ease-out, color 120ms ease-out, box-shadow 120ms ease-out; }
}
.card { background: var(--surface); padding: 20px 24px; border: 1px solid var(--line); border-radius: 12px; }
h1 { margin: 0; font-size: 28px; line-height: 1.1; }
h2 { margin: 0; font-size: 20px; line-height: 1.2; }
h2 small, h4 small { font: 400 13.5px var(--sans); letter-spacing: 0; color: var(--muted); }
/* A card's heading. */
.cardhead { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; }
h3 { margin: 0; font-size: 28px; line-height: 1; }
h4 { margin: 0; font-size: 15px; font-weight: 700; }
.sub { color: var(--muted); font-size: 13.5px; }
.sub a { color: var(--link); text-decoration: underline; text-underline-offset: 3px; }
.sub a:hover { color: var(--accent-12); }
.empty { margin: 8px 0; color: var(--muted); }
/* The first visit: the three commands that bring the first app. */
.first { display: flex; flex-direction: column; gap: 12px; }
.first p { margin: 0; max-width: 64ch; }
.first p code { padding: 1px 4px; border-radius: 4px; background: var(--sunk); font-size: 13.5px; white-space: nowrap; }
.cmd { align-self: flex-start; max-width: 100%; padding: 12px 20px; overflow-x: auto; background: var(--sunk); border-radius: 8px;
  font-size: 13.5px; line-height: 24px; color: var(--ink); white-space: pre; }
.cmd code { font-size: inherit; }
/* Before the first app: how it gets here beside the rules it will run under, a figure and a paragraph each. */
.start { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 20px; }
.rules ul { display: grid; grid-template-columns: max-content minmax(0, 1fr); column-gap: 24px; margin: 8px 0 0; padding: 0; list-style: none; }
.rules li { display: grid; grid-column: 1 / -1; grid-template-columns: subgrid; align-items: baseline; padding: 16px 0; }
.rules li + li { border-top: 1px solid var(--line-soft); }
.rules li:last-child { padding-bottom: 0; }
.rules li > b { font-size: 28px; line-height: 1; }
.rules p { margin: 0; }
.rules strong { font-weight: 600; }
.rules .grant { margin: 4px 4px 4px 0; }

.summary { display: flex; gap: 16px; align-items: center; background: var(--tone-2); border-color: var(--tone-6); }
.summary.tone-success { background: var(--surface); border-color: var(--line); }
.summary > p { margin: 0; font-size: 20px; line-height: 1.3; }
.sign { width: 40px; height: 40px; display: grid; place-items: center; flex: none; border-radius: 10px; background: var(--tone-9); color: var(--tone-contrast); }

/* Nothing going on: grey icon, muted name; text keeps its contrast. */
.dim { color: var(--muted); }
.dim .icon { opacity: .55; filter: grayscale(1); }
.badge { display: inline-flex; align-items: center; gap: 4px; padding: 0 8px 0 4px; border-radius: 6px; background: var(--tone-3); color: var(--tone-11);
  font: 600 12px/20px var(--sans); white-space: nowrap; }
.badge svg { flex: none; }
.badge.strong { background: var(--tone-9); color: var(--tone-contrast); }
.icon { display: grid; place-items: center; flex: none; line-height: 1; color: var(--dark-text); box-shadow: inset 0 0 0 1px var(--line); }
.icon img { display: block; }

/* The platform's numbers in a row under the apps, one card, its cells split by hairlines (the gap shows the card's
   line colour through). */
.kpis { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); grid-auto-rows: 1fr; gap: 1px; padding: 0; overflow: hidden; background: var(--line-soft); }
.kpi { display: flex; flex-direction: column; gap: 8px; padding: 20px 24px; background: var(--surface); font-size: 13.5px; }
/* A tile opens its events in the log. */
a.kpi:hover { background: var(--sunk); }
a.kpi:hover .label { color: var(--ink); }
a.kpi:focus-visible { outline-offset: -2px; }
.kpi .label, .kpi .context, .delta { color: var(--muted); }
.figure { display: flex; flex-wrap: wrap; align-items: baseline; column-gap: 12px; }
.kpi b { font-size: 48px; line-height: 1; }
.delta { font-variant-numeric: tabular-nums; white-space: nowrap; }
/* A bar per 24 hours on a baseline; the newest, the big number, in the accent. */
.spark { display: flex; gap: 2px; height: 28px; margin-top: auto; box-shadow: inset 0 -1px 0 var(--line-soft); }
.spark span { flex: 1; min-width: 0; display: flex; align-items: flex-end; }
.spark i { width: 100%; border-radius: 1px 1px 0 0; background: var(--gray-8); }
.spark span:hover i { background: var(--gray-10); }
.spark .now i { background: var(--accent-9); }

.row { display: flex; flex-wrap: wrap; gap: 20px; align-items: flex-start; }
.chart { flex: 1 1 520px; min-width: 0; display: flex; flex-direction: column; gap: 16px; }
.plot { display: flex; gap: 8px; }
/* "No requests …" over an empty plot, still read by screen readers. */
.stack { display: grid; }
.stack > * { grid-area: 1 / 1; }
.nodata { z-index: 1; height: 128px; margin: 24px 0 0; padding: 0 24px; display: grid; place-items: center; text-align: center; color: var(--muted); }
.nodata span { padding: 4px 12px; background: var(--surface); }
/* Zero-high labels spread over the plot's height, each centred on its gridline. */
.yaxis { display: flex; flex-direction: column; justify-content: space-between; height: 128px; margin-top: 24px; font-size: 12px; color: var(--muted); } /* PLOT in charts.rs */
.yaxis span { display: flex; align-items: center; justify-content: flex-end; height: 0; }
/* Above every plot, room for the flags of changes, so charts with and without them line up. */
.bars { position: relative; flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; padding-top: 24px; }
.cols { display: flex; align-items: flex-end; gap: 4px; height: 128px;
  background: linear-gradient(var(--line-soft), var(--line-soft)) 0 0 / 100% 1px no-repeat,
    linear-gradient(var(--line-soft), var(--line-soft)) 0 50% / 100% 1px no-repeat,
    linear-gradient(var(--gray-8), var(--gray-8)) 0 100% / 100% 1px no-repeat; }
.cols > div { flex: 1 1 0; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; gap: 2px; }
.cols > div:hover { background: var(--sunk); }
/* Square at the baseline, a 4px rounded end on top, a 2px gap between segments. */
.cols i { display: block; }
.cols i:first-child { border-radius: 4px 4px 0 0; }
.xaxis { display: flex; justify-content: space-between; font-size: 12px; color: var(--muted); }
/* A change: a flag (its title says what) and a 2px line down to the baseline, in the gap before the minute it
   happened in. The line lets the columns' own titles through. */
.mk { position: absolute; top: 0; width: 0; height: 152px; color: var(--tone-9); pointer-events: none; } /* 24px + PLOT */
.mk::before { content: ""; position: absolute; top: 18px; bottom: 0; left: -1px; width: 2px; background: currentColor; }
.mk.dashed::before { background: repeating-linear-gradient(currentColor 0 4px, transparent 4px 7px); }
.mk .flag { position: absolute; top: 0; left: -9px; pointer-events: auto; box-shadow: 0 0 0 2px var(--surface); }
/* A flag opens the log at its change. */
.mk .flag:hover { box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px var(--tone-9); }
.flag { display: inline-grid; place-items: center; width: 18px; height: 18px; flex: none; border-radius: 50%; background: var(--tone-9); color: var(--tone-contrast); }
.legend { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; font-size: 12px; color: var(--muted); }
.legend > span { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 8px; }
.sw { display: inline-block; width: 12px; height: 12px; flex: none; border-radius: 3px; box-shadow: inset 0 0 0 1px var(--gray-7); }

.heat { flex: 1 1 420px; min-width: 0; padding: 20px 12px; display: flex; flex-direction: column; gap: 4px; }
/* Side by side, the list of apps stays in view while the detail scrolls. */
@media (min-width: 1100px) { .heat { position: sticky; top: 16px; } }
.heat h2 { margin: 0 12px 8px; }
.hrow { display: grid; grid-template-columns: minmax(96px, 1fr) minmax(0, 236px); gap: 12px; align-items: center;
  padding: 8px 12px; border-radius: 8px; }
a.hrow:hover { background: var(--sunk); }
.hrow.on, a.hrow.on:hover { background: var(--accent-3); }
.hrow.head { padding-top: 0; padding-bottom: 0; font-size: 12px; color: var(--muted); }
.ends { display: flex; justify-content: space-between; }
.hname { display: flex; align-items: center; gap: 8px; min-width: 0; font-weight: 600; }
.hname > span:last-child { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.strip { display: grid; grid-template-columns: repeat(24, minmax(0, 1fr)); gap: 2px; }
.strip i { height: 20px; border-radius: 3px; background: var(--gray-3); }
.heat .legend { flex-direction: column; align-items: flex-start; padding: 12px 12px 0; }

.detail { flex: 1.6 1 560px; min-width: 0; background: var(--surface); overflow: hidden; border: 1px solid var(--line); border-radius: 12px; }
.dhead { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 16px; padding: 20px 24px 4px; }
.dname { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
.limits { margin-left: auto; font: 12px var(--mono); color: var(--muted); background: var(--sunk); border-radius: 6px; padding: 4px 8px; }
/* Relative, so an sr-only label in a wide table stays inside the scroll box
   instead of widening the page. */
.scroll { position: relative; overflow-x: auto; scrollbar-width: thin; scrollbar-color: var(--gray-7) transparent; }
/* Every table alike: header and rows 36px high (8px, a 20px line, 8px) and
   a hairline; numbers right; badges, code and icons kept to the line. */
table { width: 100%; border-collapse: collapse; font-size: 13.5px; }
th { font-size: 12px; font-weight: 500; line-height: 20px; color: var(--muted); text-align: left; padding: 8px 12px; border-bottom: 1px solid var(--line); white-space: nowrap; }
td { padding: 8px 12px; line-height: 20px; border-bottom: 1px solid var(--line-soft); vertical-align: top; }
td code, td .badge, td .risk, td .grant, td .to-risk { vertical-align: top; }
tr:last-child td { border-bottom: 0; }
th:first-child, td:first-child { padding-left: 24px; }
th:last-child, td:last-child { padding-right: 24px; }
th.num, .num { text-align: right; }
.routes { min-width: 640px; }
.routes td:first-child, .who-app { white-space: nowrap; }
.bad { color: var(--critical-11); font-weight: 700; }
/* A part of the detail: a heading over a table as wide as the card. */
.part { padding-top: 16px; }
.part h4 { padding: 0 24px 4px; }
.part > .empty { margin: 4px 24px 8px; }
.more { display: inline-block; margin: 12px 24px 20px; font-size: 13.5px; font-weight: 600; color: var(--accent-11); }
.more:hover { text-decoration: underline; }
.part:last-child { padding-bottom: 8px; }
/* The app's own chart: plot and legend inside the detail's padding. */
.graph { display: flex; flex-direction: column; gap: 16px; padding: 16px 24px 4px; }
.part.graph h4 { padding: 0; }
/* The live version leads, tinted like the selected app. */
.versions { min-width: 520px; }
.versions td { white-space: nowrap; }
.versions tr.live td { background: var(--accent-2); }
.versions tr.live code { font-weight: 700; color: var(--accent-12); }
/* A reason or error reads as part of the row above it, and doesn't widen
   the columns. */
tr.has-reason td { border-bottom: 0; }
tr.reason td { padding-top: 0; white-space: normal; color: var(--muted); }
tr.reason div, tr.reason pre { contain: inline-size; }
tr.reason pre { display: -webkit-box; -webkit-line-clamp: 4; -webkit-box-orient: vertical; overflow: hidden; color: var(--ink); }
.failures { min-width: 520px; }
.failures td { white-space: nowrap; }
.failures td:last-child { width: 100%; }
/* A grant's row tinted when it sets the app's risk. */
.grants { min-width: 560px; }
.grants td { white-space: nowrap; }
.grants td:nth-child(3) { width: 100%; color: var(--muted); }
.grants tr.sets td { background: var(--accent-2); }
.uses { min-width: 560px; }
.uses td { white-space: nowrap; }
.uses td:nth-child(3) { width: 100%; }
.uses td a { color: var(--link); text-decoration: underline; text-underline-offset: 3px; }
.uses td a:hover { color: var(--accent-12); }
.grants tr.sets code { font-weight: 700; color: var(--accent-12); }
.partfoot { margin: 12px 24px 8px; font-size: 13.5px; color: var(--muted); }
.partfoot a, .grants .when { color: var(--link); text-decoration: underline; text-underline-offset: 3px; }
.partfoot a:hover, .grants .when:hover { color: var(--accent-12); }
.versions .act { width: 100%; padding-top: 4px; padding-bottom: 4px; text-align: right; }
/* An action in a row is secondary: the row is what you read. */
.act button, .act .button { min-height: 28px; padding: 0 12px; }
form { margin: 0; }
/* Buttons: secondary unless said otherwise; primary (the accent) for the
   one action a page is for. */
button, .button { min-height: 32px; padding: 0 16px; border: 0; border-radius: 8px; font: 600 13.5px var(--sans); cursor: pointer;
  background: var(--surface); color: var(--ink); box-shadow: inset 0 0 0 1px var(--gray-7); }
.button { display: inline-flex; align-items: center; }
button:hover, .button:hover { background: light-dark(var(--gray-3), var(--gray-5)); }
button.primary, .button.primary { background: var(--accent-9); color: var(--accent-contrast); box-shadow: none; }
button.primary:hover, .button.primary:hover { background: var(--accent-10); }
.user { display: inline-flex; align-items: center; gap: 8px; color: var(--ink); }
.user form { display: contents; }
.looks { padding: 2px 8px; border: 1px solid var(--line); border-radius: 6px; color: var(--muted); font-size: 12px; white-space: nowrap; }
form[inert] > * { opacity: .4; }
.user button { min-height: 28px; padding: 0 12px; border: 1px solid var(--line); border-radius: 6px; background: none; color: var(--muted); font: inherit; cursor: pointer; }
.user button:hover { color: var(--ink); }
.scheme { display: flex; gap: 2px; padding: 2px; background: var(--gray-3); border-radius: 8px; }
.scheme button, .scheme button:hover { display: grid; place-items: center; min-height: 28px; padding: 0 8px; border: 0; border-radius: 6px; background: none; color: var(--muted);
  box-shadow: none; }
.scheme button:hover { color: var(--ink); }
.scheme button[aria-pressed="true"] { background: var(--surface); color: var(--ink); box-shadow: 0 0 0 1px var(--line); }

/* The inventory: an app a row; the app stays in view while the rest
   scrolls sideways. */
.inventory { padding: 20px 0 4px; overflow: hidden; }
.inventory .cardhead { padding: 0 24px 12px; }
.inventory table { min-width: 1080px; }
.inventory tbody th { font-size: 13.5px; font-weight: 400; color: var(--ink); border-bottom: 1px solid var(--line-soft); }
.inventory tr:last-child th { border-bottom: 0; }
.inventory th:first-child { position: sticky; left: 0; z-index: 1; background: var(--surface); }
.inventory td { white-space: nowrap; }
.inventory td.chips { width: 100%; min-width: 200px; white-space: normal; }
.inventory .names { display: flex; flex-direction: column; line-height: 20px; }
.inventory .id { font-size: 12px; line-height: 16px; color: var(--muted); }
.inventory .version:hover code, .inventory .when:hover, .inventory .owner:hover { color: var(--accent-11); text-decoration: underline; text-underline-offset: 3px; }
.change { display: flex; align-items: center; gap: 8px; }
.change svg { flex: none; color: var(--tone-11); }
.when { margin-left: 4px; color: var(--muted); }
.inventory tbody th { vertical-align: top; }
a.bad { text-decoration: underline; text-underline-offset: 3px; }
.who-app { display: flex; align-items: center; gap: 8px; margin: -2px 0; }
.chips { padding-bottom: 4px; }
.who-app b { font-weight: 600; }
/* 20px high like a badge: an 18px line in a 1px border. */
.grant, .risk { display: inline-flex; align-items: center; gap: 4px; padding: 0 8px; border: 1px solid transparent; border-radius: 6px;
  font: 600 12px/18px var(--sans); white-space: nowrap; }
.risk svg { flex: none; }
.grant { margin: 0 4px 4px 0; font: 500 12px/18px var(--mono); }
.grant b { margin-left: 4px; font-family: var(--sans); font-weight: 700; }
.unrated { background: none; color: var(--muted); border: 1px dashed var(--gray-8); }
.bands { display: inline-flex; flex-wrap: wrap; gap: 4px; vertical-align: 1px; margin-left: 4px; }
.none { color: var(--faint); }

.events { padding-bottom: 8px; }
.events .more { margin: 8px 0 12px; }
.to-risk { display: inline-block; border-radius: 6px; }
.to-risk:hover .risk { box-shadow: 0 0 0 1px currentColor; }
.events h2 { margin-bottom: 8px; }
.ev { display: flex; gap: 16px; align-items: flex-start; padding: 12px 0; border-bottom: 1px solid var(--line-soft); font-size: 13.5px; }
.ev:last-child { border-bottom: 0; }
.evicon { width: 32px; height: 32px; display: grid; place-items: center; flex: none; border-radius: 8px; background: var(--tone-3); color: var(--tone-11); }
.evtext { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
.evtext b { font-weight: 600; }
.evtext .sub { font-size: inherit; }
.ev pre { color: var(--muted); }
.foot { margin: 4px 0 0; font-size: 12px; color: var(--muted); }
/* A warning in a card; as a banner it carries its button on the right. */
.warn { display: flex; gap: 12px; padding: 12px 16px; border-radius: 8px; background: var(--tone-2); box-shadow: inset 0 0 0 1px var(--tone-6); }
.warn svg { flex: none; margin-top: 4px; color: var(--tone-11); }
.warn p { margin: 0; }
.banner { margin: 4px 24px 8px; }
.detail > .banner { margin-top: 16px; }
.banner form { flex: none; align-self: center; margin-left: auto; }
.dhead .switch { gap: 8px; padding: 0 12px; color: var(--critical-11); }
.dhead .switch svg { flex: none; }
/* The log: a table as wide as its card, a band per day; on wide screens the header stays in view. */
.logcard { padding: 20px 0 8px; }
.logcard .cardhead { margin: 0 24px 12px; }
.logcard > .empty { margin: 8px 24px 12px; }
.policy > .foot { margin: 12px 24px 8px; }
/* The token page: inset like its head, in lines short enough to read. */
.token > :not(.cardhead) { margin: 0 24px 12px; max-width: 72ch; }
.token > form { margin-bottom: 8px; }
.token > .cmd { display: block; max-width: none; white-space: pre-wrap; overflow-wrap: anywhere; }
.policy tbody th { width: 30%; min-width: 220px; font-size: 13.5px; font-weight: 400; color: var(--ink); white-space: normal; }
.policy .names { display: flex; flex-direction: column; line-height: 20px; }
.policy .id { font-size: 12px; line-height: 16px; color: var(--muted); }
.policy td.may ul { margin: 0; padding: 0; list-style: none; }
.policy td.may li { line-height: 28px; }
.policy td.may li .grant { margin-bottom: 0; vertical-align: baseline; }
.policy tbody tr:last-child th { border-bottom: 0; }
.policy td.may a { color: var(--link); text-decoration: underline; text-underline-offset: 3px; }
.log { min-width: 1000px; }
.log td { white-space: nowrap; }
.log td.what { width: 100%; white-space: normal; }
/* Linked to (#event-12), a row shows below the sticky header. */
.log tr[id] { scroll-margin-top: 48px; }
.log tr:target td { background: var(--accent-2); }
.log tr.day th { padding-top: 20px; font-size: 13.5px; font-weight: 700; color: var(--ink); }
.time { font-variant-numeric: tabular-nums; color: var(--muted); }
.event { display: inline-flex; align-items: center; gap: 8px; vertical-align: top; font-weight: 600; }
.event svg { flex: none; color: var(--tone-11); }
.quiet { color: var(--muted); }
/* A row opens to the whole text: the message carries a chevron that turns. */
.what summary { display: flex; flex-direction: column; list-style: none; cursor: pointer; }
.what summary::-webkit-details-marker { display: none; }
.what .msg::after { content: ""; display: inline-block; width: 6px; height: 6px; margin-left: 8px; vertical-align: 3px; border: solid var(--muted);
  border-width: 0 1.5px 1.5px 0; transform: rotate(45deg); }
.what details[open] .msg::after { vertical-align: 0; transform: rotate(-135deg); }
.what summary:hover .msg { color: var(--accent-11); }
.because { font-size: 12px; color: var(--muted); overflow-wrap: anywhere; }
.whole { display: flex; flex-direction: column; gap: 8px; padding: 8px 0 4px; font-size: 12px; }
.whole p { margin: 0; }
.whole pre { margin: 0; padding: 8px 12px; border-radius: 6px; background: var(--sunk); font: 12px/1.5 var(--mono); white-space: pre-wrap; overflow-wrap: anywhere; }
.facts { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 4px 16px; margin: 0; align-items: center; }
.facts dt { color: var(--muted); }
.facts dd { margin: 0; }
.behind { margin: 0; padding: 0; list-style: none; }
.behind li { display: grid; grid-template-columns: 64px minmax(0, max-content) 64px minmax(0, 1fr); gap: 12px; padding: 4px 0; border-top: 1px solid var(--line-soft); }
.behind .err { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--mono); color: var(--muted); }
.pages { display: flex; gap: 8px; padding: 12px 24px 12px; }
/* Filters: one row above the table, a label over each field; the search takes what is left. */
.filters { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 12px; padding: 4px 24px 20px; border-bottom: 1px solid var(--line); }
.field { display: flex; flex-direction: column; gap: 4px; min-width: 0; font-size: 12px; font-weight: 600; color: var(--muted); }
.field.grow { flex: 1 1 128px; }
.field input[type="date"] { padding: 0 8px; }
input, select { height: 32px; padding: 0 12px; border: 0; border-radius: 8px; background: var(--surface); color: var(--ink); font: 13.5px var(--sans);
  box-shadow: inset 0 0 0 1px var(--gray-7); }
input:hover, select:hover { box-shadow: inset 0 0 0 1px var(--gray-8); }
input::placeholder { color: var(--muted); }
.select { position: relative; display: flex; }
.select select { flex: 1; min-width: 0; padding-right: 32px; appearance: none; cursor: pointer; }
.select::after { content: ""; position: absolute; top: 12px; right: 14px; width: 6px; height: 6px; border: solid var(--muted); border-width: 0 1.5px 1.5px 0;
  transform: rotate(45deg); pointer-events: none; }
.go { display: flex; align-items: center; gap: 16px; min-height: 32px; }
.go a, .logcard > .empty a { color: var(--link); text-decoration: underline; text-underline-offset: 3px; }
.go a:hover, .logcard > .empty a:hover { color: var(--accent-12); }
.logcard > .empty a { margin-left: 4px; }
@media (min-width: 1100px) {
  .logcard .scroll { overflow: visible; }
  .log thead th { position: sticky; top: 0; z-index: 1; border-bottom: 0; background: var(--surface); box-shadow: inset 0 -1px 0 var(--line); }
}

/* For screen readers only: the charts' numbers as tables. */
.sr-only { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap; border: 0; }

@media (max-width: 1000px) { .kpis { grid-template-columns: repeat(2, minmax(0, 1fr)); } .start { grid-template-columns: minmax(0, 1fr); } }
/* Narrow, an event is a block: when, what and how bad; the app; what happened; version and who. */
@media (max-width: 900px) {
  .log { min-width: 0; }
  .log thead { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  .log tbody, .log tr.day, .log tr.day th { display: block; }
  .log tr.day th { padding: 20px 24px 8px; border-bottom: 1px solid var(--line); }
  .log tr[id] { display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 8px 12px; padding: 12px 24px;
    grid-template-areas: "time event sev" "app app app" "what what what" "ver ver by"; border-bottom: 1px solid var(--line-soft); }
  .log tr[id] td { display: block; padding: 0; border: 0; white-space: normal; }
  .log td:nth-child(1) { grid-area: time; }
  .log td:nth-child(2) { grid-area: event; }
  .log td:nth-child(3) { grid-area: app; }
  .log td:nth-child(4) { grid-area: what; }
  .log td:nth-child(5) { grid-area: ver; }
  .log td:nth-child(6) { grid-area: sev; }
  .log td:nth-child(7) { grid-area: by; color: var(--muted); }
}
/* Phones: a 16px gutter, tighter cards, two apps a row, the sentence under its sign; wide tables scroll inside their card. */
@media (max-width: 700px) {
  .wrap { padding: 16px 16px 48px; gap: 16px; }
  .card { padding: 16px; }
  .kpis { padding: 0; }
  .kpi { padding: 16px; }
  .heat { padding: 16px 4px; }
  .inventory { padding: 16px 0 4px; }
  .summary { flex-direction: column; align-items: flex-start; gap: 12px; }
  .rules ul, .rules li { grid-template-columns: minmax(0, 1fr); row-gap: 8px; }
  .hrow { grid-template-columns: minmax(0, 1fr) minmax(0, 45%); }
  .inventory .cardhead, .dhead, .part h4 { padding-left: 16px; padding-right: 16px; }
  .graph { padding-left: 16px; padding-right: 16px; }
  .logcard .cardhead { margin: 0 16px 12px; }
  .token > :not(.cardhead) { margin-left: 16px; margin-right: 16px; }
  .log tr.day th { padding-left: 16px; padding-right: 16px; }
  .log tr[id] { padding-left: 16px; padding-right: 16px; }
  .logcard > .empty { margin: 8px 16px 12px; }
  .pages { padding-left: 16px; padding-right: 16px; }
  .filters { padding: 4px 16px 16px; }
  .field { flex: 1 1 128px; }
  .field.grow { flex-basis: 100%; }
  .more, .part > .empty, .partfoot, .banner { margin-left: 16px; margin-right: 16px; }
  th:first-child, td:first-child { padding-left: 16px; }
  th:last-child, td:last-child { padding-right: 16px; }
  .limits { margin-left: 0; }
}
"#;

#[cfg(test)]
mod tests {
    use super::{RULES, owner_html};

    /// The px values of a property in the rules, as written.
    fn values(prop: &str) -> Vec<f64> {
        let mut out = Vec::new();
        for decl in [RULES, super::super::console::CSS].concat().split([';', '{', '}']) {
            let Some((name, value)) = decl.split_once(':') else { continue };
            if name.trim() != prop {
                continue;
            }
            // "font: 600 12px/20px var(--sans)" has its size before the slash.
            let value = if prop == "font" { value.split('/').next().unwrap_or("") } else { value };
            out.extend(value.split_whitespace().filter_map(|v| v.strip_suffix("px")?.parse::<f64>().ok()));
        }
        out
    }

    /// The name links to the address; anything else stays text, escaped.
    #[test]
    fn an_owner_is_a_name_to_write_to() {
        assert_eq!(
            owner_html("Jane Doe <jane@example.com>"),
            r#"<a class="owner" href="mailto:jane@example.com" title="Write to jane@example.com">Jane Doe</a>"#
        );
        assert_eq!(
            owner_html(" growth+apps@example.com "),
            r#"<a class="owner" href="mailto:growth+apps@example.com" title="Write to growth+apps@example.com">growth+apps@example.com</a>"#
        );
        assert_eq!(owner_html("<ops@example.com>"), r#"<a class="owner" href="mailto:ops@example.com" title="Write to ops@example.com">ops@example.com</a>"#);
        assert_eq!(owner_html("Growth team"), r#"<span class="owner">Growth team</span>"#);
        assert_eq!(owner_html(r#"Eve <a"b@x.com>"#), r#"<span class="owner">Eve &lt;a&quot;b@x.com&gt;</span>"#);
        assert_eq!(owner_html("Eve <javascript:x@y.z>"), r#"<span class="owner">Eve &lt;javascript:x@y.z&gt;</span>"#);
    }

    #[test]
    fn every_font_size_is_on_the_scale() {
        let scale = [12.0, 13.5, 15.0, 20.0, 28.0, 48.0];
        let sizes = [values("font-size"), values("font")].concat();
        assert!(sizes.len() > 20);
        for size in sizes {
            assert!(scale.contains(&size), "{size}px is not on the type scale {scale:?}");
        }
    }

    #[test]
    fn spacing_is_on_the_4px_grid() {
        for prop in ["padding", "padding-top", "padding-bottom", "margin", "margin-top", "margin-left", "margin-right", "gap"] {
            for v in values(prop) {
                assert!(v % 4.0 == 0.0 || v.abs() <= 2.0, "{prop}: {v}px is off the 4px grid");
            }
        }
    }
}
