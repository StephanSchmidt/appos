//! The admin screen's colours as numbers. Every colour is a step on a
//! 12-step OKLCH scale, and a step has the same job in every scale and theme
//! (Radix's step jobs):
//!
//! - 1, 2: page backgrounds: the page, and sunk areas like table heads
//! - 3, 4, 5: component backgrounds: normal, hovered, pressed or selected
//! - 6: borders and dividers of static things, hairlines about 1.3:1 on white
//! - 7: borders of interactive things
//! - 8: strong borders
//! - 9: the solid fill (buttons, strong badges); 10: its hover
//! - 11: text in the scale's hue, muted text for gray; 12: high-contrast text
//!
//! The lightness of a step is the same in every hue, so a step has about the
//! same contrast in every hue. Chroma follows a curve per theme and is cut
//! to what sRGB can show. The page gets every step as a custom property
//! (`--gray-1` to `--critical-12`), a `light-dark()` pair of both themes.

use std::fmt;
use std::fmt::Write as _;

/// A colour in OKLCH: lightness from 0 to 1, chroma, hue in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

impl Oklch {
    const fn new(l: f64, c: f64, h: f64) -> Self {
        Self { l, c, h }
    }

    /// Linear sRGB, not clamped: a component outside 0 to 1 is out of gamut.
    pub(super) fn linear_srgb(self) -> [f64; 3] {
        let (a, b) = (self.c * self.h.to_radians().cos(), self.c * self.h.to_radians().sin());
        let l = (self.l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
        let m = (self.l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
        let s = (self.l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
        [
            4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
        ]
    }

    pub(super) fn in_gamut(self) -> bool {
        self.linear_srgb().iter().all(|x| (-1e-4..=1.0 + 1e-4).contains(x))
    }

    /// The same lightness and hue with as much of the chroma as sRGB can
    /// show, rounded to what the CSS says.
    fn fit(self) -> Self {
        let l = (self.l * 1000.0).round() / 1000.0;
        let (mut c, mut over) = (self.c, self.c);
        if !Self::new(l, c, self.h).in_gamut() {
            c = 0.0;
            for _ in 0..40 {
                let mid = (c + over) / 2.0;
                if Self::new(l, mid, self.h).in_gamut() { c = mid } else { over = mid }
            }
        }
        Self::new(l, (c * 1000.0).floor() / 1000.0, self.h)
    }
}

impl fmt::Display for Oklch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "oklch({:.1}% {:.3} {})", self.l * 100.0, self.c, self.h)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Theme {
    Light = 0,
    Dark = 1,
}

/// Lightness per step, light theme then dark. Steps 9 and 10 (0 here) come
/// from the scale's solid fill. Dark step 11 is 0.78: at 0.71 it passes
/// WCAG 2 (7:1), but APCA rates muted text and links on the dark page only
/// Lc 50; at 0.78 they get Lc 62, light theme's are 77 and more.
const LIGHTNESS: [[f64; 12]; 2] = [
    [0.984, 0.970, 0.958, 0.944, 0.930, 0.915, 0.875, 0.800, 0.0, 0.0, 0.500, 0.230],
    [0.165, 0.190, 0.222, 0.246, 0.270, 0.290, 0.345, 0.430, 0.0, 0.0, 0.780, 0.950],
];

/// Chroma per step as a share of the scale's peak, light then dark: little
/// near white and black, all of it in the solid fill.
const HUE_CHROMA: [[f64; 12]; 2] =
    [[0.03, 0.07, 0.12, 0.18, 0.25, 0.32, 0.45, 0.65, 1.0, 1.0, 0.95, 0.55], [0.10, 0.16, 0.26, 0.34, 0.42, 0.50, 0.60, 0.72, 1.0, 1.0, 0.75, 0.30]];

/// Greys keep a small, nearly even chroma: warm, never tinted.
const GRAY_CHROMA: [[f64; 12]; 2] =
    [[0.4, 0.5, 0.6, 0.6, 0.7, 0.7, 0.8, 0.9, 1.0, 1.0, 1.0, 0.8], [0.6, 0.7, 0.7, 0.8, 0.8, 0.8, 0.9, 0.9, 1.0, 1.0, 0.9, 0.4]];

const WHITE: Oklch = Oklch::new(1.0, 0.0, 0.0);
/// Dark text on light fills like yellow, in both themes: light gray-12.
const DARK: Oklch = Oklch::new(0.23, 0.008, 80.0);

/// Cards and panels: white on the light page, a step lighter than the dark
/// page.
pub(super) const SURFACE: [Oklch; 2] = [WHITE, Oklch::new(0.205, 0.007, 80.0)];

pub(super) struct Scale {
    pub name: &'static str,
    hue: f64,
    /// The hue of dark steps (lightness under 0.6). Dark yellow turns olive,
    /// so caution moves toward orange there.
    dark_hue: f64,
    /// The chroma of the solid fill; other steps take a share of it.
    peak: f64,
    chroma: &'static [[f64; 12]; 2],
    /// Lightness of the solid fill, step 9, light then dark theme. Step 10,
    /// its hover, is 0.04 darker.
    solid: [f64; 2],
    /// Text on the solid fill: white, or dark on light fills (yellow, orange).
    pub on_solid: Oklch,
}

impl Scale {
    /// Step 1 to 12 in a theme.
    pub(super) fn step(&self, theme: Theme, n: usize) -> Oklch {
        let t = theme as usize;
        let l = match n {
            9 => self.solid[t],
            10 => self.solid[t] - 0.04,
            _ => LIGHTNESS[t][n - 1],
        };
        let h = if l < 0.6 { self.dark_hue } else { self.hue };
        Oklch::new(l, self.peak * self.chroma[t][n - 1], h).fit()
    }
}

/// Warm greys at the hue of the old cream, much less of it.
pub(super) const GRAY: Scale = Scale { name: "gray", hue: 80.0, dark_hue: 80.0, peak: 0.010, chroma: &GRAY_CHROMA, solid: [0.56, 0.56], on_solid: WHITE };
/// Blue-violet: links, focus, the main chart, the primary button. Also the
/// info tone.
pub(super) const ACCENT: Scale = Scale { name: "accent", hue: 270.0, dark_hue: 270.0, peak: 0.20, chroma: &HUE_CHROMA, solid: [0.54, 0.56], on_solid: WHITE };
pub(super) const SUCCESS: Scale = Scale { name: "success", hue: 150.0, dark_hue: 150.0, peak: 0.15, chroma: &HUE_CHROMA, solid: [0.52, 0.54], on_solid: WHITE };
pub(super) const CAUTION: Scale = Scale { name: "caution", hue: 95.0, dark_hue: 75.0, peak: 0.17, chroma: &HUE_CHROMA, solid: [0.885, 0.86], on_solid: DARK };
pub(super) const WARNING: Scale = Scale { name: "warning", hue: 60.0, dark_hue: 60.0, peak: 0.17, chroma: &HUE_CHROMA, solid: [0.72, 0.70], on_solid: DARK };
pub(super) const CRITICAL: Scale =
    Scale { name: "critical", hue: 27.0, dark_hue: 27.0, peak: 0.20, chroma: &HUE_CHROMA, solid: [0.555, 0.58], on_solid: WHITE };

pub(super) const SCALES: [&Scale; 6] = [&GRAY, &ACCENT, &SUCCESS, &CAUTION, &WARNING, &CRITICAL];

/// A risk band's colours, escalating: grey, beige, orange, red (Stephan,
/// 2026-11-13). Low is as quiet as plain chrome; each band up gains warmth
/// and stands out more from the page: darker on the light page, brighter on
/// the dark one.
pub(super) struct RiskBand {
    pub name: &'static str,
    /// Light theme, then dark.
    fill: [Oklch; 2],
    text: [Oklch; 2],
}

impl RiskBand {
    pub(super) fn fill(&self, theme: Theme) -> Oklch {
        self.fill[theme as usize].fit()
    }

    pub(super) fn text(&self, theme: Theme) -> Oklch {
        self.text[theme as usize].fit()
    }
}

/// Low to critical, in the order of `manifest::Band`.
pub(super) const RISK: [RiskBand; 4] = [
    RiskBand {
        name: "low",
        fill: [Oklch::new(0.935, 0.006, 80.0), Oklch::new(0.29, 0.006, 80.0)],
        text: [Oklch::new(0.42, 0.012, 80.0), Oklch::new(0.86, 0.01, 80.0)],
    },
    RiskBand {
        name: "medium",
        fill: [Oklch::new(0.885, 0.05, 75.0), Oklch::new(0.46, 0.035, 80.0)],
        text: [Oklch::new(0.40, 0.07, 65.0), Oklch::new(0.93, 0.045, 80.0)],
    },
    RiskBand { name: "high", fill: [Oklch::new(0.72, 0.16, 55.0), Oklch::new(0.645, 0.16, 55.0)], text: [DARK, DARK] },
    RiskBand { name: "critical", fill: [Oklch::new(0.555, 0.20, 27.0), Oklch::new(0.70, 0.19, 27.0)], text: [WHITE, DARK] },
];

/// The heat strip's speed ramp: the slowest request of a few minutes, from
/// fast (a quiet grey just off the empty track) through yellow and orange to
/// red at the time limit. Neighbouring hues with lightness in one direction:
/// darker in light, brighter in dark, so slower always stands out more.
pub(super) const HEAT: [[Oklch; 6]; 2] = [
    [
        Oklch::new(0.86, 0.012, 80.0),
        Oklch::new(0.815, 0.03, 85.0),
        Oklch::new(0.79, 0.14, 92.0),
        Oklch::new(0.73, 0.16, 70.0),
        Oklch::new(0.65, 0.18, 50.0),
        Oklch::new(0.555, 0.20, 27.0),
    ],
    [
        Oklch::new(0.37, 0.012, 80.0),
        Oklch::new(0.44, 0.035, 85.0),
        Oklch::new(0.52, 0.11, 88.0),
        Oklch::new(0.60, 0.14, 70.0),
        Oklch::new(0.65, 0.17, 50.0),
        Oklch::new(0.70, 0.19, 27.0),
    ],
];

/// Monogram tiles (apps without an icon.svg) get a hue from the app's id, at
/// one lightness and chroma so the dark initials always read. Only cool
/// hues, cyan to pink: warm ones (red to yellow) mean problems and risk,
/// green means fine, and the accent's blue-violet sits between them.
const MONOGRAM_HUES: [f64; 6] = [195.0, 220.0, 245.0, 295.0, 320.0, 345.0];

fn monogram_tile(hue: f64) -> Oklch {
    Oklch::new(0.80, 0.12, hue).fit()
}

/// A tile colour per app id, in the order given: the hue its id hashes to,
/// or the next free one when another app has it, so up to six apps all
/// differ. The same ids give the same colours on every run and machine.
pub(super) fn monograms<'a>(ids: impl IntoIterator<Item = &'a str>) -> Vec<Oklch> {
    const N: usize = MONOGRAM_HUES.len();
    let mut taken = [false; N];
    ids.into_iter()
        .map(|id| {
            // FNV-1a, not std's hasher, which is seeded per run.
            let hash = id.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
            let first = hash as usize % N;
            let i = (0..N).map(|k| (first + k) % N).find(|i| !taken[*i]).unwrap_or(first);
            taken[i] = true;
            if taken.iter().all(|t| *t) {
                taken = [false; N];
            }
            monogram_tile(MONOGRAM_HUES[i])
        })
        .collect()
}

/// The custom properties: every step of every scale, `--{name}-contrast` for
/// text on its solid fill, the risk bands (`--risk-low`, `--risk-low-text`),
/// `--surface`, and `--dark-text` for text on light fills in both themes.
pub(super) fn css() -> String {
    let pair = |light: Oklch, dark: Oklch| if light == dark { light.to_string() } else { format!("light-dark({light}, {dark})") };
    let mut css = String::from(":root {\n");
    for s in SCALES {
        for n in 1..=12 {
            let _ = writeln!(css, "  --{}-{n}: {};", s.name, pair(s.step(Theme::Light, n), s.step(Theme::Dark, n)));
        }
        let _ = writeln!(css, "  --{}-contrast: {};", s.name, s.on_solid);
    }
    for (i, (light, dark)) in HEAT[0].iter().zip(&HEAT[1]).enumerate() {
        let _ = writeln!(css, "  --heat-{}: {};", i + 1, pair(light.fit(), dark.fit()));
    }
    for b in &RISK {
        let _ = writeln!(css, "  --risk-{}: {};", b.name, pair(b.fill(Theme::Light), b.fill(Theme::Dark)));
        let _ = writeln!(css, "  --risk-{}-text: {};", b.name, pair(b.text(Theme::Light), b.text(Theme::Dark)));
    }
    let _ = writeln!(css, "  --surface: {};\n  --dark-text: {DARK};\n}}", pair(SURFACE[0], SURFACE[1]));
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    const THEMES: [Theme; 2] = [Theme::Light, Theme::Dark];

    #[test]
    fn known_colours_convert_to_srgb() {
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
        assert!(close(WHITE.linear_srgb(), [1.0, 1.0, 1.0]));
        // sRGB red and blue, from the CSS Color 4 spec's conversion.
        assert!(close(Oklch::new(0.627_955, 0.257_683, 29.233_885).linear_srgb(), [1.0, 0.0, 0.0]));
        assert!(close(Oklch::new(0.452_014, 0.313_214, 264.052_021).linear_srgb(), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn every_step_is_in_srgb_and_steps_go_from_the_page_to_text() {
        for s in SCALES {
            for theme in THEMES {
                let steps: Vec<Oklch> = (1..=12).map(|n| s.step(theme, n)).collect();
                assert!(steps.iter().all(|c| c.in_gamut()), "{} {theme:?} out of sRGB", s.name);
                // Backgrounds and borders move away from the page, step by step.
                let away = |a: &Oklch, b: &Oklch| match theme {
                    Theme::Light => b.l < a.l,
                    Theme::Dark => b.l > a.l,
                };
                assert!(steps[..8].windows(2).all(|w| away(&w[0], &w[1])), "{} {theme:?} steps 1-8 out of order", s.name);
                assert!(away(&steps[10], &steps[11]), "{} {theme:?}: step 12 must be stronger text than 11", s.name);
            }
        }
    }

    /// WCAG 2 relative luminance.
    fn luminance(c: Oklch) -> f64 {
        let [r, g, b] = c.linear_srgb().map(|x| x.clamp(0.0, 1.0));
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// WCAG 2 contrast ratio, from 1 to 21.
    fn contrast(a: Oklch, b: Oklch) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// WCAG 2 AA: text at least 4.5:1, UI parts like the focus ring 3:1;
    /// checked by step job, so every use that keeps to the jobs passes.
    #[test]
    fn text_and_focus_have_wcag_contrast_in_both_themes() {
        let mut low = Vec::new();
        let mut check = |what: String, fg: Oklch, bg: Oklch, min: f64| {
            let ratio = contrast(fg, bg);
            if ratio < min {
                low.push(format!("{what}: {ratio:.2}, needs {min}"));
            }
        };
        for theme in THEMES {
            let surface = SURFACE[theme as usize];
            for s in SCALES {
                // Text steps on the page, the surface, and every background and component step.
                let mut grounds: Vec<(String, Oklch)> =
                    (1..=5).flat_map(|n| [(format!("gray-{n}"), GRAY.step(theme, n)), (format!("{}-{n}", s.name), s.step(theme, n))]).collect();
                grounds.dedup_by(|a, b| a.0 == b.0);
                grounds.push(("surface".to_string(), surface));
                for (ground, bg) in grounds {
                    for text in [11, 12] {
                        check(format!("{theme:?} {}-{text} on {ground}", s.name), s.step(theme, text), bg, 4.5);
                    }
                }
                // Text on the solid fill and its hover.
                for fill in [9, 10] {
                    check(format!("{theme:?} {}-contrast on {}-{fill}", s.name, s.name), s.on_solid, s.step(theme, fill), 4.5);
                }
            }
            // The focus ring, accent-9, against the page, sunk areas and the surface.
            for (ground, bg) in [("gray-1", GRAY.step(theme, 1)), ("gray-2", GRAY.step(theme, 2)), ("surface", surface)] {
                check(format!("{theme:?} focus on {ground}"), ACCENT.step(theme, 9), bg, 3.0);
            }
        }
        assert!(low.is_empty(), "too little contrast:\n{}", low.join("\n"));
    }

    /// Risk badges: readable, and each band stands out from the page more
    /// than the one below it, in both themes.
    #[test]
    fn riskier_bands_stand_out_more() {
        for theme in THEMES {
            let page = GRAY.step(theme, 1);
            for b in &RISK {
                assert!(b.fill(theme).in_gamut() && b.text(theme).in_gamut(), "{} {theme:?}", b.name);
                let ratio = contrast(b.text(theme), b.fill(theme));
                assert!(ratio >= 4.5, "{theme:?} {} text on its fill: {ratio:.2}", b.name);
            }
            let salience: Vec<f64> = RISK.iter().map(|b| contrast(b.fill(theme), page)).collect();
            assert!(salience.windows(2).all(|w| w[0] < w[1]), "{theme:?}: {salience:.2?}");
        }
    }

    /// Slower stands out more: each heat step is further from the surface
    /// than the one before, and even the fastest differs from the empty track.
    #[test]
    fn the_heat_ramp_goes_one_way() {
        for theme in THEMES {
            let ramp: Vec<Oklch> = HEAT[theme as usize].iter().map(|c| c.fit()).collect();
            let away: Vec<f64> = ramp.iter().map(|c| contrast(*c, SURFACE[theme as usize])).collect();
            assert!(away.windows(2).all(|w| w[0] < w[1]), "{theme:?}: {away:.2?}");
            let track = contrast(ramp[0], GRAY.step(theme, 3));
            assert!(track >= 1.3, "{theme:?}: fastest vs the empty track {track:.2}");
            assert!(contrast(ramp[5], SURFACE[theme as usize]) >= 3.0, "{theme:?}: at the limit");
        }
    }

    #[test]
    fn monograms_read_and_keep_clear_of_status_and_risk_colours() {
        let taken: Vec<f64> =
            SCALES.iter().filter(|s| s.name != "gray").flat_map(|s| [s.hue, s.dark_hue]).chain(RISK.iter().flat_map(|b| [b.fill[0].h, b.fill[1].h])).collect();
        for h in MONOGRAM_HUES {
            let tile = monogram_tile(h);
            assert!(contrast(DARK, tile) >= 4.5, "initials on hue {h}: {:.2}", contrast(DARK, tile));
            assert!(tile.c >= 0.08, "hue {h} fits only chroma {}", tile.c);
            for t in &taken {
                let d = (h - t).rem_euclid(360.0);
                assert!(d.min(360.0 - d) >= 20.0, "hue {h} is close to {t}");
            }
        }
        let six = monograms(["a", "b", "c", "d", "e", "f"]);
        assert!(six.iter().enumerate().all(|(i, c)| !six[..i].contains(c)), "six apps, six colours");
        assert_eq!(monograms(["leads"]), monograms(["leads"]));
    }

    /// Hairlines (step 6) stay soft, about 1.3:1 on the surface (Linear's
    /// calm chrome), but visible.
    #[test]
    fn hairlines_stay_soft() {
        for theme in THEMES {
            let ratio = contrast(GRAY.step(theme, 6), SURFACE[theme as usize]);
            assert!((1.2..=1.45).contains(&ratio), "{theme:?} gray-6 on surface: {ratio:.2}");
        }
    }

    #[test]
    fn the_css_has_every_step_once() {
        let css = css();
        for s in SCALES {
            for n in 1..=12 {
                assert_eq!(css.matches(&format!("--{}-{n}:", s.name)).count(), 1);
            }
        }
        assert!(css.contains("--gray-1: light-dark(oklch(98.4% 0.004 80), oklch(16.5% 0.006 80));"), "{css}");
    }
}
