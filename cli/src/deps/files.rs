//! What a lister reads: the disk, or in tests a map of paths to contents.
//! Rust's std has no io/fs with an fstest.MapFS, as Go has; this is the
//! little of one the listers need.

use std::io;
use std::path::{Component, Path, PathBuf};

pub trait Files {
    fn read(&self, path: &Path) -> io::Result<String>;
    fn exists(&self, path: &Path) -> bool;
    fn is_file(&self, path: &Path) -> bool;
    /// The directories in `dir`, sorted, symlinks not followed.
    fn dirs(&self, dir: &Path) -> Vec<PathBuf>;
}

pub struct Disk;

impl Files for Disk {
    fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn dirs(&self, dir: &Path) -> Vec<PathBuf> {
        let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
        let mut dirs: Vec<PathBuf> = entries.filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
        dirs.sort();
        dirs
    }
}

/// `a/b/../c` as `a/c`, without the disk, as cargo normalizes paths.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part),
        }
    }
    out
}

/// Files in memory, by path, like Go's fstest.MapFS. A directory is there
/// when a file is in it.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct MapFs(pub std::collections::BTreeMap<PathBuf, String>);

#[cfg(test)]
impl Files for MapFs {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.0.get(path).cloned().ok_or_else(|| io::ErrorKind::NotFound.into())
    }

    fn exists(&self, path: &Path) -> bool {
        self.0.keys().any(|k| k.starts_with(path))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.0.contains_key(path)
    }

    fn dirs(&self, dir: &Path) -> Vec<PathBuf> {
        let dirs: std::collections::BTreeSet<PathBuf> = self
            .0
            .keys()
            .filter_map(|k| {
                let mut rest = k.strip_prefix(dir).ok()?.components();
                let first = rest.next()?;
                rest.next()?;
                Some(dir.join(first))
            })
            .collect();
        dirs.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_of_files_has_the_directories_its_paths_go_through() {
        let files = MapFs(["/a/c/x.toml", "/a/b/d/y", "/a/z"].into_iter().map(|p| (PathBuf::from(p), String::new())).collect());
        assert_eq!(files.dirs(Path::new("/a")), [PathBuf::from("/a/b"), PathBuf::from("/a/c")], "sorted, without the file /a/z");
        assert_eq!(files.dirs(Path::new("/a/b")), [PathBuf::from("/a/b/d")]);
        assert!(files.exists(Path::new("/a/b/d")) && files.exists(Path::new("/a/z")) && !files.exists(Path::new("/a/b/x")));
        assert!(files.is_file(Path::new("/a/z")) && !files.is_file(Path::new("/a/b/d")) && !files.is_file(Path::new("/a/q")));
        assert_eq!(files.read(Path::new("/a/z")).unwrap(), "");
        assert_eq!(files.read(Path::new("/a/b")).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn paths_are_normalized_without_the_disk() {
        assert_eq!(normalize(Path::new("/w/app/../helper/./src")), Path::new("/w/helper/src"));
        assert_eq!(normalize(Path::new("/w/app/../../../x")), Path::new("/x"));
    }
}
