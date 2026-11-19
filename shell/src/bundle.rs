//! app.zip: an app as one file. `appos deploy` sends it, the shell keeps it
//! per version, and anything that can't build the app (CI, an agent, a
//! review) can deploy one made elsewhere. At the top of the zip: app.toml,
//! app.wasm and, if the app has them, icon.svg and deps.json (deps.rs).
//!
//! A Wasm kit's zip is the same, with kit.toml and kit.wasm instead, its
//! WIT in wit/ and, if it has one, its fake as fake.wasm.
//!
//! An app from a folder, a deploy and a stored version are all a `Bundle`,
//! and `Bundle::checked` checks them the same before runtime::load.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};

use crate::manifest::Manifest;

/// What the shell keeps of each version, in its directory.
pub const FILE: &str = "app.zip";
const MANIFEST: &str = crate::manifest::APP_TOML;
const WASM: &str = "app.wasm";
const KIT_MANIFEST: &str = crate::manifest::KIT_TOML;
const KIT_WASM: &str = "kit.wasm";
const FAKE: &str = "fake.wasm";
const ICON: &str = "icon.svg";
const DEPS: &str = crate::deps::FILE;
/// What an app.zip holds, for messages.
const HOLDS: &str =
    "it holds app.toml, app.wasm and, if the app has them, icon.svg and deps.json; a kit's holds kit.toml, kit.wasm, wit/*.wit and, if it has one, fake.wasm";

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
    /// The text of app.toml, or of a kit's kit.toml.
    pub manifest: String,
    pub wasm: Vec<u8>,
    pub icon: Option<String>,
    /// The text of deps.json: the packages the app is built with.
    pub deps: Option<String>,
    /// A Wasm kit's other files; none for an app.
    pub kit: Option<KitFiles>,
}

/// A Wasm kit's files besides kit.toml and kit.wasm.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct KitFiles {
    /// Its WIT, by file name (wit/<name> in the zip), sorted.
    pub wit: Vec<(String, String)>,
    /// Its fake, for appos dev.
    pub fake: Option<Vec<u8>>,
}

/// A bundle whose app.toml reads and whose icon fits, with its label.
pub struct Checked {
    pub manifest: Manifest,
    /// The version and a 128-bit hash of every file.
    pub label: String,
    /// The package URLs in deps.json; None without one.
    pub packages: Option<Vec<String>>,
    pub bundle: Arc<Bundle>,
}

impl Bundle {
    /// The app in a folder, or of an app.toml: the manifest, the wasm it
    /// names and icon.svg next to it, if there is one. No deps.json: only
    /// `appos pack` writes one, into app.zip.
    pub fn from_dir(path: &Path) -> anyhow::Result<Bundle> {
        let (m, manifest, wasm_path) = Manifest::load(path)?;
        let wasm = read_file(&wasm_path).with_context(|| format!("{}", wasm_path.display()))?;
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(Path::new(".")) };
        let kit = match &m.kit {
            Some(offers) => {
                let mut wit = vec![];
                for file in &offers.wit {
                    let path = dir.join(file);
                    let text = String::from_utf8(read_file(&path).with_context(|| format!("{}", path.display()))?)
                        .map_err(|_| anyhow::anyhow!("{} is not UTF-8 text", path.display()))?;
                    wit.push((file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), text));
                }
                wit.sort();
                let fake = match &offers.fake {
                    Some(file) => Some(read_file(&dir.join(file)).with_context(|| format!("fake = {file:?}: {}", dir.join(file).display()))?),
                    None => None,
                };
                Some(KitFiles { wit, fake })
            }
            None => None,
        };
        let icon_path = dir.join(ICON);
        let icon = match read_file(&icon_path) {
            Ok(bytes) => Some(String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{} is not UTF-8 text", icon_path.display()))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("{}", icon_path.display())),
        };
        Ok(Bundle { manifest, wasm, icon, deps: None, kit })
    }

    /// The bytes of every file, to bound them.
    fn size(&self) -> Option<usize> {
        let optional = |f: &Option<String>| f.as_ref().map_or(0, String::len);
        let kit = self.kit.as_ref().map_or(Some(0), |k| {
            k.wit.iter().try_fold(k.fake.as_ref().map_or(0, Vec::len), |n, (name, text)| n.checked_add(name.len())?.checked_add(text.len()))
        })?;
        self.manifest.len().checked_add(self.wasm.len())?.checked_add(optional(&self.icon))?.checked_add(optional(&self.deps))?.checked_add(kit)
    }

    /// Its manifest, read as an app's or a kit's.
    pub fn manifest(&self) -> anyhow::Result<Manifest> {
        if self.kit.is_some() { Manifest::parse_kit(&self.manifest) } else { Manifest::parse(&self.manifest) }
    }

    /// The names of the files in its zip, for messages: "app.zip".
    pub fn file(&self) -> &'static str {
        if self.kit.is_some() { "kit.zip" } else { "app.zip" }
    }

    /// Checks what the shell checks of every app before it loads it: the
    /// manifest reads, the files fit, the icon is a small SVG, deps.json
    /// lists packages.
    pub fn checked(self) -> anyhow::Result<Checked> {
        let manifest = self.manifest()?;
        if let Some(kit) = &self.kit {
            anyhow::ensure!(!kit.wit.is_empty(), "the kit has no WIT: wit = [...] in kit.toml names it");
            for (name, _) in &kit.wit {
                anyhow::ensure!(wit_name(name), "WIT file {name:?}: <name>.wit, letters, digits and dashes");
            }
        }
        if self.size().is_none_or(|n| n > MAX) {
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
        let packages = self.deps.as_deref().map(crate::deps::parse).transpose()?;
        let label = self.label(&manifest.app.version);
        Ok(Checked { manifest, label, packages, bundle: Arc::new(self) })
    }

    /// The version and a hash of every file and its length: an icon-only
    /// change is a new version. deps.json counts only when there is one, so
    /// a bundle without keeps its label. 128 bits keep even a 200-byte version within
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
        if let Some(deps) = &self.deps {
            hash.update(b"deps\0");
            hash.update((deps.len() as u64).to_le_bytes());
            hash.update(deps.as_bytes());
        }
        if let Some(kit) = &self.kit {
            hash.update(b"kit\0");
            for (name, text) in &kit.wit {
                for bytes in [name.as_bytes(), text.as_bytes()] {
                    hash.update((bytes.len() as u64).to_le_bytes());
                    hash.update(bytes);
                }
            }
            if let Some(fake) = &kit.fake {
                hash.update(b"fake\0");
                hash.update((fake.len() as u64).to_le_bytes());
                hash.update(fake);
            }
        }
        let hex: String = hash.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect();
        format!("{version}-{hex}")
    }

    /// The zip. The same files give the same bytes: fixed order and times.
    pub fn pack(&self) -> anyhow::Result<Vec<u8>> {
        if self.size().is_none_or(|n| n > MAX) {
            bail!("app.zip unpacks to more than {} MB", MAX >> 20);
        }
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        let (manifest, wasm) = if self.kit.is_some() { (KIT_MANIFEST, KIT_WASM) } else { (MANIFEST, WASM) };
        let mut files: Vec<(String, &[u8])> = vec![(manifest.into(), self.manifest.as_bytes()), (wasm.into(), &self.wasm)];
        files.extend([(ICON, &self.icon), (DEPS, &self.deps)].into_iter().filter_map(|(name, f)| Some((name.to_string(), f.as_ref()?.as_bytes()))));
        if let Some(kit) = &self.kit {
            files.extend(kit.wit.iter().map(|(name, text)| (format!("wit/{name}"), text.as_bytes())));
            files.extend(kit.fake.as_deref().map(|fake| (FAKE.to_string(), fake)));
        }
        for (name, bytes) in files {
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
            bail!("app.zip has {} files; {HOLDS}", zip.len());
        }
        let (mut manifest, mut wasm, mut icon, mut deps, mut other) = (None, None, None, None, vec![]);
        let (mut kit_manifest, mut kit_wasm, mut fake, mut wit) = (None, None, None, BTreeMap::new());
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
                KIT_MANIFEST => &mut kit_manifest,
                KIT_WASM => &mut kit_wasm,
                FAKE => &mut fake,
                ICON => &mut icon,
                DEPS => &mut deps,
                _ => match name.strip_prefix("wit/").filter(|n| wit_name(n)) {
                    Some(file) => wit.entry(file.to_string()).or_insert(None),
                    None => {
                        other.push(name);
                        continue;
                    }
                },
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
        if let Some(nested) = other.iter().find(|n| n.ends_with("/app.toml") || n.ends_with("/kit.toml")) {
            bail!("app.zip has {nested}: its files go at the top, not in a folder; {HOLDS}");
        }
        let is_kit = kit_manifest.is_some();
        if is_kit {
            anyhow::ensure!(manifest.is_none() && wasm.is_none(), "the zip has app.toml and kit.toml: an app or a kit, not both");
        } else {
            other.extend(kit_wasm.is_some().then(|| KIT_WASM.to_string()));
            other.extend(fake.is_some().then(|| FAKE.to_string()));
            other.extend(wit.keys().map(|n| format!("wit/{n}")));
        }
        if !other.is_empty() {
            bail!("app.zip has files the shell doesn't use: {}; {HOLDS}", other.join(", "));
        }
        let text = |name: &str, bytes: Vec<u8>| String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("{name} in app.zip is not UTF-8 text"));
        if is_kit {
            let wit = wit.into_iter().map(|(name, bytes)| Ok((name.clone(), text(&name, bytes.unwrap_or_default())?))).collect::<anyhow::Result<_>>()?;
            return Ok(Bundle {
                manifest: text(KIT_MANIFEST, kit_manifest.unwrap_or_default())?,
                wasm: kit_wasm.context("kit.zip has no kit.wasm")?,
                icon: icon.map(|i| text(ICON, i)).transpose()?,
                deps: deps.map(|d| text(DEPS, d)).transpose()?,
                kit: Some(KitFiles { wit, fake }),
            });
        }
        Ok(Bundle {
            manifest: text(MANIFEST, manifest.context("app.zip has no app.toml")?)?,
            wasm: wasm.context("app.zip has no app.wasm")?,
            icon: icon.map(|i| text(ICON, i)).transpose()?,
            deps: deps.map(|d| text(DEPS, d)).transpose()?,
            kit: None,
        })
    }
}

/// A WIT file's name in a kit: <name>.wit, of letters, digits, dashes and
/// underscores.
fn wit_name(name: &str) -> bool {
    name.strip_suffix(".wit").is_some_and(|stem| !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// What the shell checks of app.toml (or a kit's kit.toml) before it
/// loads a deploy, beyond `Bundle::checked`: it names an owner and a
/// group, and says what data it keeps. Returns the id. `appos deploy`
/// checks the same before it builds.
pub fn check(manifest: &str) -> anyhow::Result<String> {
    let kit = toml::from_str::<toml::Table>(manifest).is_ok_and(|t| t.contains_key("kit"));
    let m = if kit { Manifest::parse_kit(manifest)? } else { Manifest::parse(manifest)? };
    m.owner()?;
    m.group()?;
    m.data()?;
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
        let deps = crate::deps::to_json(["pkg:cargo/serde@1.0.228".to_string()]);
        let app =
            Bundle { manifest: "[app]\nid = \"a\"".into(), wasm: vec![0, 97, 115, 109, 1, 0, 0, 0], icon: Some("<svg/>".into()), deps: Some(deps), kit: None };
        let zip = app.pack().unwrap();
        assert_eq!(Bundle::unpack(&zip).unwrap(), app);
        assert_eq!(app.pack().unwrap(), zip, "packing twice gives the same bytes");
        let plain = Bundle { icon: None, deps: None, ..app };
        let unpacked = Bundle::unpack(&plain.pack().unwrap()).unwrap();
        assert_eq!((unpacked.icon, unpacked.deps), (None, None));
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
        let app = Bundle { manifest: "manifest".into(), wasm: vec![0; MAX], icon: None, deps: None, kit: None };
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
    fn deps_json_is_checked_and_counts_in_the_label() {
        let app = |deps: Option<&str>| Bundle {
            manifest: "[app]\nid = 'a'\nname = 'A'\nversion = '1.0'\n".into(),
            wasm: vec![1],
            icon: None,
            deps: deps.map(str::to_string),
            kit: None,
        };
        let deps = crate::deps::to_json(["pkg:cargo/time@0.1.43".to_string()]);
        let checked = app(Some(&deps)).checked().unwrap();
        assert_eq!(checked.packages.unwrap(), ["pkg:cargo/time@0.1.43"]);
        let plain = app(None).checked().unwrap();
        assert_eq!(plain.packages, None);
        assert_ne!(checked.label, plain.label, "a changed deps.json is a new version");
        assert!(format!("{:#}", app(Some("{}")).checked().err().unwrap()).contains("deps.json"));
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
