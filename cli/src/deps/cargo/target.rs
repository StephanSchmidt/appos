//! Whether a `[target.<spec>]` table holds for wasm32-wasip2: the target's
//! name, or a cfg() of it, as cargo-platform parses and matches them.

pub const TARGET: &str = "wasm32-wasip2";

/// `rustc --print cfg --target wasm32-wasip2` (1.100), which cargo matches
/// `[target.'cfg(..)']` against. debug_assertions too: cargo asks rustc
/// without the profile's flags.
///
/// Open: a rustc that changes the target's defaults makes this stale.
/// Asking rustc would fix it, but this runs no cargo and no rustc, so the
/// fix has to be another way.
const CFG: &[&str] = &[
    "debug_assertions",
    "panic=\"abort\"",
    "target_abi=\"\"",
    "target_arch=\"wasm32\"",
    "target_endian=\"little\"",
    "target_env=\"p2\"",
    "target_family=\"wasm\"",
    "target_feature=\"bulk-memory\"",
    "target_feature=\"crt-static\"",
    "target_feature=\"multivalue\"",
    "target_feature=\"mutable-globals\"",
    "target_feature=\"nontrapping-fptoint\"",
    "target_feature=\"reference-types\"",
    "target_feature=\"sign-ext\"",
    "target_has_atomic=\"16\"",
    "target_has_atomic=\"32\"",
    "target_has_atomic=\"64\"",
    "target_has_atomic=\"8\"",
    "target_has_atomic=\"ptr\"",
    "target_has_atomic_primitive_alignment=\"16\"",
    "target_has_atomic_primitive_alignment=\"32\"",
    "target_has_atomic_primitive_alignment=\"64\"",
    "target_has_atomic_primitive_alignment=\"8\"",
    "target_has_atomic_primitive_alignment=\"ptr\"",
    "target_os=\"wasi\"",
    "target_pointer_width=\"32\"",
    "target_vendor=\"unknown\"",
];

/// Whether a `[target.<spec>]` holds for wasm32-wasip2.
pub fn matches(spec: &str) -> bool {
    match spec.strip_prefix("cfg(").and_then(|s| s.strip_suffix(')')) {
        Some(cfg) => {
            let mut p = Cfg(cfg);
            p.expr().is_some_and(|v| {
                p.space();
                p.0.is_empty() && v
            })
        }
        None => spec == TARGET,
    }
}

/// A cfg expression being read: `all(..)`, `any(..)`, `not(..)`, `true`,
/// `false`, a name or `key = "value"`; idents may be raw (`r#name`). None
/// where cargo-platform has a parse error, which matches nothing.
struct Cfg<'a>(&'a str);

impl<'a> Cfg<'a> {
    /// Only spaces separate tokens, as in cargo-platform's tokenizer.
    fn space(&mut self) {
        self.0 = self.0.trim_start_matches(' ');
    }

    fn eat(&mut self, c: char) -> bool {
        self.space();
        match self.0.strip_prefix(c) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }

    /// An identifier, and whether it is raw.
    fn ident(&mut self) -> Option<(bool, &'a str)> {
        self.space();
        let (raw, s) = self.0.strip_prefix("r#").map_or((false, self.0), |s| (true, s));
        if !s.starts_with(|c: char| c == '_' || c.is_ascii_alphabetic()) {
            return None;
        }
        let end = s.find(|c: char| !(c == '_' || c.is_ascii_alphanumeric())).unwrap_or(s.len());
        self.0 = &s[end..];
        Some((raw, &s[..end]))
    }

    fn string(&mut self) -> Option<&'a str> {
        self.space();
        let (value, rest) = self.0.strip_prefix('"')?.split_once('"')?;
        self.0 = rest;
        Some(value)
    }

    fn expr(&mut self) -> Option<bool> {
        match self.ident()? {
            (false, op @ ("all" | "any")) => {
                if !self.eat('(') {
                    return None;
                }
                let mut values = vec![];
                while !self.eat(')') {
                    values.push(self.expr()?);
                    if !self.eat(',') {
                        if !self.eat(')') {
                            return None;
                        }
                        break;
                    }
                }
                Some(if op == "all" { values.iter().all(|v| *v) } else { values.iter().any(|v| *v) })
            }
            (false, "not") => {
                if !self.eat('(') {
                    return None;
                }
                let value = self.expr()?;
                self.eat(')').then_some(!value)
            }
            (_, name) if self.eat('=') => {
                let value = self.string()?;
                Some(CFG.contains(&format!("{name}=\"{value}\"").as_str()))
            }
            (_, "true") => Some(true),
            (_, "false") => Some(false),
            (_, name) => Some(CFG.contains(&name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_specs_hold_as_cargo_platform_matches_them_for_wasm32_wasip2() {
        for yes in [
            "wasm32-wasip2",
            r#"cfg(target_arch = "wasm32")"#,
            r#"cfg(target_family="wasm")"#,
            r#"cfg( target_os = "wasi" )"#,
            r#"cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))"#,
            r#"cfg(any(unix, target_os = "wasi",))"#,
            "cfg(not(windows))",
            r#"cfg(not(target_has_atomic = "128"))"#,
            r#"cfg(target_abi = "")"#,
            r#"cfg(panic = "abort")"#,
            "cfg(all())",
            "cfg(true)",
            // cargo-platform compares names without the r#.
            "cfg(r#true)",
            r#"cfg(r#target_os = "wasi")"#,
            "cfg(debug_assertions)",
        ] {
            assert!(matches(yes), "{yes}");
        }
        for no in [
            "x86_64-unknown-linux-gnu",
            "wasm32-wasip1",
            "cfg(windows)",
            "cfg(unix)",
            "cfg(any())",
            "cfg(false)",
            r#"cfg(all(target_arch = "wasm32", target_os = "unknown"))"#,
            r#"cfg(true = "x")"#,
            // What cargo-platform can't parse.
            r#"cfg(target_os = "wasi") extra"#,
            r#"cfg(any(windows, target_os = "wasi")"#,
            "cfg(not(windows)",
            "cfg(r#all())",
            "cfg(not)",
            r#"cfg(target_os = wasi)"#,
            "cfg(\ttarget_os = \"wasi\")",
            "cfg()",
            "cfg(r#)",
        ] {
            assert!(!matches(no), "{no}");
        }
    }
}
