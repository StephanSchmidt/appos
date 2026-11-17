//! Rustflags that change the cfg rustc tells cargo for the target, which
//! target.rs would then match wrong. They're refused, not read.

use std::path::Path;

use toml::Value;

use super::read_toml;
use crate::deps::Env;
use crate::deps::files::Files;

/// A rustflag that changes the target's cfg, and where it's set: the
/// environment, or a config.toml cargo reads for a build in `app`. Cargo
/// uses one of them; any is reason enough.
pub fn changing_cfg(files: &dyn Files, app: &Path, cargo_home: &Path, env: &Env<'_>) -> Option<String> {
    let mut found: Vec<(String, Vec<String>)> = vec![];
    if let Some(flags) = env("CARGO_ENCODED_RUSTFLAGS") {
        found.push(("CARGO_ENCODED_RUSTFLAGS".into(), flags.split('\x1f').map(str::to_string).collect()));
    }
    for key in ["RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS", "CARGO_TARGET_WASM32_WASIP2_RUSTFLAGS"] {
        if let Some(flags) = env(key) {
            found.push((key.into(), flags.split_whitespace().map(str::to_string).collect()));
        }
    }
    let dirs = app.ancestors().map(|d| d.join(".cargo")).chain([cargo_home.to_path_buf()]);
    for path in dirs.flat_map(|d| [d.join("config.toml"), d.join("config")]) {
        let Ok(config) = read_toml(files, &path) else { continue };
        let build = config.get("build").and_then(|b| b.get("rustflags"));
        let targets = config.get("target").and_then(Value::as_table).into_iter().flatten().filter_map(|(_, t)| t.get("rustflags"));
        for flags in build.into_iter().chain(targets) {
            let flags = match flags {
                Value::String(flags) => flags.split_whitespace().map(str::to_string).collect(),
                flags => flags.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
            };
            found.push((path.display().to_string(), flags));
        }
    }
    found.into_iter().find_map(|(source, flags)| changes_cfg(&flags).map(|flag| format!("`{flag}` in {source}")))
}

/// The first flag that changes `rustc --print cfg`: --cfg, -Z, -O and the
/// codegen options for target features, the CPU, panics, debug assertions
/// and optimization.
fn changes_cfg(flags: &[String]) -> Option<String> {
    let mut flags = flags.iter().map(String::as_str);
    while let Some(flag) = flags.next() {
        let codegen = match flag {
            "--cfg" => return Some(format!("--cfg {}", flags.next().unwrap_or_default())),
            "-O" => return Some(flag.into()),
            _ if flag.starts_with("--cfg=") || flag.starts_with("-Z") => return Some(flag.into()),
            "-C" | "--codegen" => flags.next(),
            _ => flag.strip_prefix("--codegen=").or_else(|| flag.strip_prefix("-C")),
        };
        if let Some(option) = codegen {
            let key = option.split('=').next().unwrap_or(option).replace('_', "-");
            if ["target-feature", "target-cpu", "panic", "debug-assertions", "opt-level"].contains(&key.as_str()) {
                return Some(format!("-C {option}"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_that_change_the_cfg_are_found_however_they_are_written() {
        let first = |flags: &str| changes_cfg(&flags.split(' ').map(str::to_string).collect::<Vec<_>>());
        for (flags, flag) in [
            ("--cfg foo", "--cfg foo"),
            ("--cfg=foo", "--cfg=foo"),
            ("-O", "-O"),
            ("-Zbuild-std", "-Zbuild-std"),
            ("-C target-feature=+simd128", "-C target-feature=+simd128"),
            ("-Ctarget-cpu=mvp", "-C target-cpu=mvp"),
            ("--codegen panic=unwind", "-C panic=unwind"),
            ("--codegen=debug_assertions=off", "-C debug_assertions=off"),
            ("-D warnings -C opt-level=0", "-C opt-level=0"),
        ] {
            assert_eq!(first(flags).as_deref(), Some(flag), "{flags}");
        }
        for harmless in ["-D warnings", "-C link-arg=-s", "-Ccodegen-units=1", "--cap-lints warn", "-C"] {
            assert_eq!(first(harmless), None, "{harmless}");
        }
    }
}
