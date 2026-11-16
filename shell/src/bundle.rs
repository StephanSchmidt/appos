//! app.zip: an app as one file. `appos deploy` sends it, the shell keeps it
//! per version, and anything that can't build the app (CI, an agent, a
//! review) can deploy one made elsewhere. At the top of the zip: app.toml,
//! app.wasm and, if the app has one, icon.svg.
//!
//! An app from a folder, a deploy and a stored version are all a `Bundle`,
//! and `Bundle::checked` checks them the same before runtime::load.

use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};

use crate::manifest::Manifest;

/// What the shell keeps of each version, in its directory.
pub const FILE: &str = "app.zip";
const MANIFEST: &str = "app.toml";
const WASM: &str = "app.wasm";
const ICON: &str = "icon.svg";

/// The largest app.zip, and the most its files may unpack to.
pub const MAX: usize = 64 * 1024 * 1024;
/// More files than this is not an app.zip.
const MAX_FILES: usize = 100;
/// The largest icon.svg.
pub const MAX_ICON: usize = 64 * 1024;

/// Bound reads from CI artifacts and the version store before allocating
/// for the whole file. Metadata alone cannot bound a file that is growing.
pub fn read_file(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{} is larger than {} MB", path.display(), MAX >> 20)));
    }
    Ok(bytes)
}

/// An app's files, as the shell loads them.
#[derive(Debug, PartialEq)]
pub struct Bundle {
    /// The text of app.toml.
    pub manifest: String,
    pub wasm: Vec<u8>,
    pub icon: Option<String>,
}

/// A bundle whose app.toml reads and whose icon fits, with its label.
pub struct Checked {
    pub manifest: Manifest,
    /// The version and a 128-bit hash of every file.
    pub label: String,
    pub bundle: Arc<Bundle>,
}

impl Bundle {
    /// The app in a folder, or of an app.toml: the manifest, the wasm it
    /// names and icon.svg next to it, if there is one.
    pub fn from_dir(path: &Path) -> anyhow::Result<Bundle> {
        let (_, manifest, wasm_path) = Manifest::load(path)?;
        let wasm = read_file(&wasm_path).with_context(|| format!("{}", wasm_path.display()))?;
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(Path::new(".")) };
        let icon_path = dir.join(ICON);
        let icon = match read_file(&icon_path) {
            Ok(bytes) => Some(String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not UTF-8 text", icon_path.display()))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("{}", icon_path.display())),
        };
        Ok(Bundle { manifest, wasm, icon })
    }

    /// Checks what the shell checks of every app before it loads it: the
    /// manifest reads, the files fit, the icon is a small SVG.
    pub fn checked(self) -> anyhow::Result<Checked> {
        let manifest = Manifest::parse(&self.manifest)?;
        let total = self.manifest.len() + self.wasm.len() + self.icon.as_ref().map_or(0, String::len);
        if total > MAX {
            bail!("the app's files are more than {} MB", MAX >> 20);
        }
        if let Some(icon) = &self.icon {
            if icon.len() > MAX_ICON {
                bail!("icon.svg is {} KB, more than {} KB", icon.len() / 1024, MAX_ICON / 1024);
            }
            if !icon.contains("<svg") {
                bail!("icon.svg is not an SVG image");
            }
        }
        let label = self.label(&manifest.app.version);
        Ok(Checked { manifest, label, bundle: Arc::new(self) })
    }

    /// The version and a hash of every file and its length: an icon-only
    /// change is a new version. 128 bits keep even a 200-byte version within
    /// a filename's limit; the store also refuses to overwrite different
    /// content under one label.
    pub fn label(&self, version: &str) -> String {
        use sha2::Digest as _;
        let mut hash = sha2::Sha256::new().chain_update(b"appos-bundle-v1\0");
        for bytes in [self.manifest.as_bytes(), &self.wasm] {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        hash.update([u8::from(self.icon.is_some())]);
        if let Some(icon) = &self.icon {
            hash.update((icon.len() as u64).to_le_bytes());
            hash.update(icon.as_bytes());
        }
        let hex: String = hash.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
        format!("{version}-{hex}")
    }

    /// The zip. The same files give the same bytes: fixed order and times.
    pub fn pack(&self) -> anyhow::Result<Vec<u8>> {
        let total = self.manifest.len().checked_add(self.wasm.len()).and_then(|n| n.checked_add(self.icon.as_ref().map_or(0, String::len)));
        if total.is_none_or(|n| n > MAX) {
            bail!("app.zip unpacks to more than {} MB", MAX >> 20);
        }
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        let icon = self.icon.as_ref().map(|i| (ICON, i.as_bytes()));
        for (name, bytes) in [(MANIFEST, self.manifest.as_bytes()), (WASM, &self.wasm[..])].into_iter().chain(icon) {
            zip.start_file(name, options)?;
            zip.write_all(bytes)?;
        }
        let bytes = zip.finish()?.into_inner();
        if bytes.len() > MAX {
            bail!("app.zip is larger than {} MB", MAX >> 20);
        }
        Ok(bytes)
    }

    /// Reads an app.zip as `pack` writes it, or as someone zipped the files
    /// by hand: folders and macOS's extras are skipped, any other file is
    /// refused, so nothing the shell doesn't look at rides along.
    pub fn unpack(bytes: &[u8]) -> anyhow::Result<Bundle> {
        if bytes.len() > MAX {
            bail!("app.zip is larger than {} MB", MAX >> 20);
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| anyhow::anyhow!("app.zip is not a zip file"))?;
        if zip.len() > MAX_FILES {
            bail!("app.zip has {} files; it holds app.toml, app.wasm and icon.svg", zip.len());
        }
        let (mut manifest, mut wasm, mut icon, mut other) = (None, None, None, vec![]);
        let mut total = 0;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).context("app.zip is damaged")?;
            let name = file.name().context("app.zip has a file name that can't be read")?.into_owned();
            if file.is_dir() || name.starts_with("__MACOSX/") || name.rsplit('/').next() == Some(".DS_Store") {
                continue;
            }
            let slot = match name.as_str() {
                MANIFEST => &mut manifest,
                WASM => &mut wasm,
                ICON => &mut icon,
                _ => {
                    other.push(name);
                    continue;
                }
            };
            // Read at most what is left: the size in the zip can lie.
            let mut out = Vec::new();
            file.by_ref().take((MAX - total) as u64 + 1).read_to_end(&mut out).with_context(|| format!("could not read {name} in app.zip"))?;
            total += out.len();
            if total > MAX {
                bail!("app.zip unpacks to more than {} MB", MAX >> 20);
            }
            *slot = Some(out);
        }
        if let Some(nested) = other.iter().find(|n| n.ends_with("/app.toml")) {
            bail!("app.zip has {nested}: app.toml, app.wasm and icon.svg go at the top, not in a folder");
        }
        if !other.is_empty() {
            bail!("app.zip has files the shell doesn't use: {}; it holds app.toml, app.wasm and icon.svg", other.join(", "));
        }
        let text = |name: &str, bytes: Vec<u8>| String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{name} in app.zip is not UTF-8 text"));
        Ok(Bundle {
            manifest: text(MANIFEST, manifest.context("app.zip has no app.toml")?)?,
            wasm: wasm.context("app.zip has no app.wasm")?,
            icon: icon.map(|i| text(ICON, i)).transpose()?,
        })
    }
}

/// What the shell checks of app.toml before it loads a deploy, beyond
/// `Bundle::checked`: it names an owner and a group. Returns the app's id.
/// `appos deploy` checks the same before it builds.
pub fn check(manifest: &str) -> anyhow::Result<String> {
    let m = Manifest::parse(manifest)?;
    m.owner()?;
    m.group()?;
    Ok(m.app.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            if name.ends_with('/') {
                zip.add_directory(*name, zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                zip.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
                zip.write_all(bytes).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_packed_app_unpacks_the_same_and_packs_the_same_bytes() {
        let app = Bundle { manifest: "[app]\nid = \"a\"".into(), wasm: vec![0, 97, 115, 109, 1, 0, 0, 0], icon: Some("<svg/>".into()) };
        let zip = app.pack().unwrap();
        assert_eq!(Bundle::unpack(&zip).unwrap(), app);
        assert_eq!(app.pack().unwrap(), zip, "packing twice gives the same bytes");
        let plain = Bundle { icon: None, ..app };
        assert_eq!(Bundle::unpack(&plain.pack().unwrap()).unwrap().icon, None);
    }

    #[test]
    fn a_zip_made_by_hand_is_read_and_anything_extra_refused() {
        let by_hand = zip_of(&[("__MACOSX/", b""), ("__MACOSX/._app.toml", b"x"), (".DS_Store", b"x"), ("app.toml", b"t"), ("app.wasm", b"w")]);
        assert_eq!(Bundle::unpack(&by_hand).unwrap().manifest, "t");
        let err = |files: &[(&str, &[u8])]| format!("{:#}", Bundle::unpack(&zip_of(files)).unwrap_err());
        assert!(err(&[("hello/app.toml", b"t"), ("hello/app.wasm", b"w")]).contains("go at the top, not in a folder"));
        assert!(err(&[("app.toml", b"t"), ("app.wasm", b"w"), ("run.sh", b"x")]).contains("files the shell doesn't use: run.sh"));
        assert!(err(&[("app.wasm", b"w")]).contains("no app.toml"));
        assert!(err(&[("app.toml", b"t")]).contains("no app.wasm"));
        assert!(err(&[("app.toml", &[0xff]), ("app.wasm", b"w")]).contains("not UTF-8"));
        assert!(format!("{:#}", Bundle::unpack(b"{\"manifest\": \"\"}").unwrap_err()).contains("not a zip file"));
    }

    #[test]
    fn a_zip_bomb_stops_at_the_limit() {
        let big = vec![0u8; MAX + 1];
        let err = Bundle::unpack(&zip_of(&[("app.toml", b"t"), ("app.wasm", &big)])).unwrap_err();
        assert!(format!("{err:#}").contains("unpacks to more than 64 MB"), "{err:#}");
    }

    #[test]
    fn packing_obeys_the_platforms_unpacked_size_limit() {
        let app = Bundle { manifest: "manifest".into(), wasm: vec![0; MAX], icon: None };
        assert!(app.pack().is_err(), "the CLI packed an app that no platform can unpack");
    }

    #[test]
    fn oversized_archives_are_rejected_even_with_small_contents() {
        let zip = zip_of(&[("app.toml", b"manifest"), ("app.wasm", b"wasm")]);
        // A valid self-extracting-style ZIP may have arbitrary leading data.
        let mut large = vec![0; MAX + 1 - zip.len()];
        large.extend(zip);
        assert!(Bundle::unpack(&large).is_err(), "accepted an app.zip larger than the upload limit");
    }

    #[test]
    fn artifact_reads_stop_at_the_upload_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.zip");
        let zip = zip_of(&[("app.toml", b"manifest"), ("app.wasm", b"wasm")]);
        std::fs::write(&path, &zip).unwrap();
        assert_eq!(read_file(&path).unwrap(), zip);
        std::fs::File::create(&path).unwrap().set_len(MAX as u64 * 4).unwrap();
        assert_eq!(read_file(&path).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_deploy_needs_an_owner_and_a_group() {
        let app = "[app]\nid = \"calc\"\nname = \"Calc\"\nversion = \"0.1.0\"\n";
        assert!(format!("{:#}", check(app).unwrap_err()).contains("needs an owner"));
        assert!(format!("{:#}", check(&format!("{app}owner = \"  \"\n")).unwrap_err()).contains("needs an owner"));
        let owned = format!("{app}owner = \"Jane Doe <jane@example.com>\"\n");
        assert!(format!("{:#}", check(&owned).unwrap_err()).contains("needs a group"));
        assert!(format!("{:#}", check(&format!("{owned}group = \"\"\n")).unwrap_err()).contains("needs a group"));
        assert_eq!(check(&format!("{owned}group = \"Marketing\"\n")).unwrap(), "calc");
    }
}
