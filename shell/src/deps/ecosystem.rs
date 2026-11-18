//! The package ecosystems `appos audit` matches against OSV.

/// An ecosystem: deps.json names its packages by a package URL type, OSV
/// its advisories by another name.
#[derive(Debug, PartialEq, Eq)]
pub struct Ecosystem {
    /// The package URL type, as in pkg:cargo/serde@1.0.228.
    pub purl: &'static str,
    /// OSV's name for it, in its advisories and its zip's URL.
    pub osv: &'static str,
}

/// The ecosystems matched. Their versions are compared as semver, which
/// crates.io and npm use; one with another scheme, like Maven or PyPI,
/// needs its own comparison in osv.rs.
pub const ECOSYSTEMS: &[Ecosystem] = &[Ecosystem { purl: "cargo", osv: "crates.io" }, Ecosystem { purl: "npm", osv: "npm" }];

impl Ecosystem {
    /// The ecosystem of a package URL type.
    pub fn of_purl(kind: &str) -> Option<&'static Ecosystem> {
        ECOSYSTEMS.iter().find(|e| e.purl == kind)
    }

    /// The ecosystem an advisory names.
    pub fn of_osv(name: &str) -> Option<&'static Ecosystem> {
        ECOSYSTEMS.iter().find(|e| e.osv == name)
    }

    /// Where OSV publishes all its advisories, as one zip.
    pub fn advisories(&self) -> String {
        self.url("all.zip")
    }

    /// Where OSV lists when each advisory last changed, newest first:
    /// `2026-11-17T17:30:17.901276722Z,MAL-2026-20076` a line.
    pub fn changes(&self) -> String {
        self.url("modified_id.csv")
    }

    /// Where OSV publishes one advisory, by its id.
    pub fn advisory(&self, id: &str) -> String {
        self.url(&format!("{id}.json"))
    }

    fn url(&self, file: &str) -> String {
        format!("https://osv-vulnerabilities.storage.googleapis.com/{}/{file}", self.osv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ecosystem_is_found_by_its_package_url_type_and_by_osvs_name() {
        let cargo = Ecosystem::of_purl("cargo").unwrap();
        assert_eq!(Ecosystem::of_osv("crates.io"), Some(cargo));
        assert_eq!(cargo.advisories(), "https://osv-vulnerabilities.storage.googleapis.com/crates.io/all.zip");
        let npm = Ecosystem::of_purl("npm").unwrap();
        assert_eq!(npm.changes(), "https://osv-vulnerabilities.storage.googleapis.com/npm/modified_id.csv");
        assert_eq!(npm.advisory("MAL-2026-20076"), "https://osv-vulnerabilities.storage.googleapis.com/npm/MAL-2026-20076.json");
        assert_eq!(Ecosystem::of_purl("crates.io"), None);
        assert_eq!(Ecosystem::of_osv("cargo"), None);
        assert_eq!(Ecosystem::of_osv("npm"), Ecosystem::of_purl("npm"));
        assert_eq!(Ecosystem::of_purl("pypi"), None);
    }
}
