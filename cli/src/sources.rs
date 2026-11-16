//! Source snapshots for `appos dev`. A single maximum mtime misses deleted
//! files and edits below that maximum (or with preserved timestamps).

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    link: Option<PathBuf>,
    digest: Option<[u8; 32]>,
}

fn digest(path: &Path) -> Option<[u8; 32]> {
    let mut input = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        let read = input.read(&mut buffer).ok()?;
        if read == 0 {
            return Some(hash.finalize().into());
        }
        hash.update(&buffer[..read]);
    }
}

/// What `appos dev` watches in an app's directory: these folders and files
/// (package.json, package-lock.json and tsconfig.json for a TypeScript app,
/// build.zig and build.zig.zon for a Zig app, pom.xml for a Java app).
/// `appos dev --help` and the guide name each of them.
pub(crate) const WATCHED: &[&str] = &[
    "src",
    "templates",
    "wit",
    ".cargo",
    "app.toml",
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "rust-toolchain.toml",
    "icon.svg",
    "package.json",
    "package-lock.json",
    "tsconfig.json",
    "build.zig",
    "pom.xml",
    "build.zig.zon",
];

/// A Zig file a zt template compiles to, `view.zig` next to `view.zt`: the
/// build writes it, so it isn't a change of its own.
fn generated(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "zig") && path.with_extension("zt").is_file()
}

pub(crate) fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Stamp> {
    let root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let ignored: HashSet<_> = ["target", ".git", "appos-data"]
        .iter()
        .map(|p| {
            let p = root.join(p);
            p.canonicalize().unwrap_or(p)
        })
        .collect();
    let mut directories = HashSet::new();
    let mut files = BTreeMap::new();
    let mut stack: Vec<_> = WATCHED.iter().rev().map(|p| dir.join(p)).collect();
    while let Some(path) = stack.pop() {
        let Ok(link_meta) = std::fs::symlink_metadata(&path) else { continue };
        let is_link = link_meta.file_type().is_symlink();
        let link = if is_link { std::fs::read_link(&path).ok() } else { None };
        let meta = if is_link { std::fs::metadata(&path).unwrap_or(link_meta) } else { link_meta };
        let directory = if meta.is_dir() { path.canonicalize().ok() } else { None };
        if directory.as_ref().is_some_and(|p| ignored.contains(p)) {
            continue;
        }
        if (!meta.is_dir() || is_link) && !generated(&path) {
            files.insert(
                path.clone(),
                Stamp { modified: meta.modified().ok(), len: meta.len(), link, digest: if meta.is_file() { digest(&path) } else { None } },
            );
        }
        if meta.is_dir() {
            let Some(canonical) = directory else { continue };
            if !directories.insert(canonical) {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&path) {
                let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
                // Stable traversal also makes aliases to one directory stable.
                paths.sort();
                stack.extend(paths.into_iter().rev());
            }
        }
    }
    files
}

/// A detected content change may still look fresh to Cargo if its mtime
/// precedes the last build. Removed inputs may also escape dependency
/// tracking by generators. Rebuild workspace apps for those cases.
pub(crate) fn requires_fresh_build(before: &BTreeMap<PathBuf, Stamp>, after: &BTreeMap<PathBuf, Stamp>, built_at: SystemTime) -> bool {
    before.keys().any(|path| !after.contains_key(path))
        || after.iter().any(|(path, current)| {
            let changed = before.get(path).is_none_or(|old| old.digest != current.digest || old.link != current.link);
            changed && current.modified.is_none_or(|modified| modified <= built_at)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserved_timestamps_force_compilation_but_normal_edits_keep_incremental_builds() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let path = root.path().join("src/lib.rs");
        std::fs::write(&path, "one").unwrap();
        let set_time = |seconds| {
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)))
                .unwrap()
        };
        set_time(100);
        let before = snapshot(root.path());
        std::fs::write(&path, "two").unwrap();
        set_time(100);
        let after = snapshot(root.path());
        let built_at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(200);
        assert!(requires_fresh_build(&before, &after, built_at));
        set_time(300);
        assert!(!requires_fresh_build(&before, &snapshot(root.path()), built_at));
        std::fs::remove_file(path).unwrap();
        assert!(requires_fresh_build(&before, &snapshot(root.path()), built_at));
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlink_cycles_are_finite_and_build_outputs_are_ignored() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::create_dir(root.path().join("target")).unwrap();
        std::fs::write(root.path().join("src/lib.rs"), "source").unwrap();
        std::os::unix::fs::symlink("..", root.path().join("src/loop")).unwrap();
        let before = snapshot(root.path());
        assert!(before.len() < 10, "a source symlink caused recursive traversal");
        std::fs::write(root.path().join("target/output"), "build output").unwrap();
        assert_eq!(snapshot(root.path()), before, "a source alias made build output trigger another build");
        std::fs::write(root.path().join("src/lib.rs"), "edited source").unwrap();
        assert_ne!(snapshot(root.path()), before);
    }

    /// zt writes view.zig next to view.zt during the build: that is no
    /// change of its own, or every template edit would build twice.
    #[test]
    fn a_zig_file_a_template_compiles_to_is_not_a_source() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/view.zt"), "templ").unwrap();
        std::fs::write(root.path().join("src/web.zig"), "code").unwrap();
        let before = snapshot(root.path());
        std::fs::write(root.path().join("src/view.zig"), "generated").unwrap();
        assert_eq!(snapshot(root.path()), before);
        std::fs::write(root.path().join("src/view.zt"), "edited").unwrap();
        assert_ne!(snapshot(root.path()), before);
        let before = snapshot(root.path());
        std::fs::write(root.path().join("src/web.zig"), "edited").unwrap();
        assert_ne!(snapshot(root.path()), before);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_shared_sources_is_watched() {
        let root = tempfile::tempdir().unwrap();
        let shared = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(shared.path().join("lib.rs"), "shared").unwrap();
        std::os::unix::fs::symlink(shared.path(), root.path().join("src/shared")).unwrap();
        let before = snapshot(root.path());
        std::fs::write(shared.path().join("lib.rs"), "shared edit").unwrap();
        assert_ne!(snapshot(root.path()), before);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_build_output_does_not_cause_a_rebuild_loop() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::create_dir(root.path().join("target")).unwrap();
        std::os::unix::fs::symlink("../target", root.path().join("src/output")).unwrap();
        let before = snapshot(root.path());
        std::fs::write(root.path().join("target/output"), "build output").unwrap();
        assert_eq!(snapshot(root.path()), before);
    }

    fn set_modified(path: &Path, seconds: u64) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)))
            .unwrap();
    }

    #[test]
    fn source_watching_detects_deletion_below_the_latest_timestamp() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let earlier = root.path().join("src/earlier.rs");
        let later = root.path().join("src/later.rs");
        std::fs::write(&earlier, "earlier").unwrap();
        std::fs::write(&later, "later").unwrap();
        set_modified(&earlier, 100);
        set_modified(&later, 200);
        let before = snapshot(root.path());
        std::fs::remove_file(earlier).unwrap();
        assert_ne!(snapshot(root.path()), before, "deleting a source file did not trigger rebuilding");
    }

    #[test]
    fn source_watching_detects_content_changes_with_preserved_metadata() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        let source = root.path().join("src/lib.rs");
        std::fs::write(&source, "one").unwrap();
        set_modified(&source, 100);
        let before = snapshot(root.path());
        std::fs::write(&source, "two").unwrap();
        set_modified(&source, 100);
        assert_ne!(snapshot(root.path()), before, "a same-size source edit with preserved mtime was missed");
    }

    #[test]
    fn source_watching_includes_dependency_and_build_configuration() {
        let root = tempfile::tempdir().unwrap();
        for file in ["Cargo.lock", "build.rs", "rust-toolchain.toml"] {
            let before = snapshot(root.path());
            std::fs::write(root.path().join(file), "changed").unwrap();
            assert_ne!(snapshot(root.path()), before, "changes to {file} did not trigger rebuilding");
        }
    }
}
