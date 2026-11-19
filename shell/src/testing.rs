//! A small executable component for sandbox/deployment regression tests.
//! Kept as source, so tests need no separately built wasm target or app.

use std::path::PathBuf;
use std::sync::Arc;

use crate::manifest::Risk;
use crate::shell::{Parts, Shell};
use crate::{bundle::Bundle, platform::PlatformDb, runtime, tx};

pub fn shell(data: PathBuf) -> Arc<Shell> {
    shell_with(data, Risk::default())
}

/// A shell whose company rates the grants `risk`.
pub fn shell_with(data: PathBuf, risk: Risk) -> Arc<Shell> {
    Shell::new(parts(data, risk)).unwrap()
}

/// What a test shell is made of.
pub fn parts(data: PathBuf, risk: Risk) -> Parts {
    Parts {
        origin: "http://localhost".into(),
        name: "test".into(),
        git: None,
        engine: runtime::engine().unwrap(),
        db: tx::in_memory(),
        data,
        platform: PlatformDb::in_memory(),
        risk,
        kits: Default::default(),
        keep_examples: false,
        review: None,
        auth: None,
        policy: None,
    }
}

/// A shell with sign-in as `auth_toml` sets it up, listening on localhost.
pub fn shell_with_auth(auth_toml: &str) -> Arc<Shell> {
    let auth = crate::auth::Auth::parse_with(auth_toml, "http://localhost", [7; 32], "127.0.0.1:8080", &|_| Some("s3cret/".into())).unwrap();
    Shell::new(Parts { auth: Some(auth), ..parts(Default::default(), Risk::default()) }).unwrap()
}

/// A shell with sign-in (when `auth_toml` isn't empty) and policy.csv as
/// given, with the kv app (`kv_bundle`) deployed as "test".
pub async fn shell_with_policy(auth_toml: &str, policy_csv: &str) -> (tempfile::TempDir, Arc<Shell>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy.csv");
    std::fs::write(&path, policy_csv).unwrap();
    let policy = crate::access::Policies::load(&path, crate::kit::Kits::builtin().names()).await.unwrap();
    let auth = (!auth_toml.is_empty()).then(|| crate::auth::Auth::parse(auth_toml, "http://localhost", [7; 32], "127.0.0.1:8080").unwrap());
    let shell = Shell::new(Parts { auth, policy: Some(Arc::new(policy)), ..parts(dir.path().join("data"), Risk::default()) }).unwrap();
    crate::deploy::deploy(&shell, &kv_bundle().pack().unwrap(), crate::platform::Actor::Cli).await.unwrap();
    (dir, shell)
}

/// A shell started without --auth and --policy: the token sign-in with
/// `secret`, and its built-in policy, with the kv app deployed as "test".
pub async fn shell_with_token(secret: &str) -> (tempfile::TempDir, Arc<Shell>) {
    let dir = tempfile::tempdir().unwrap();
    let policy = crate::access::Policies::built_in(&crate::auth::token::policy(), crate::kit::Kits::builtin().names()).await.unwrap();
    let auth = crate::auth::Auth::token_with(secret.into(), "http://localhost", [7; 32]);
    let shell = Shell::new(Parts { auth: Some(auth), policy: Some(Arc::new(policy)), ..parts(dir.path().join("data"), Risk::default()) }).unwrap();
    crate::deploy::deploy(&shell, &kv_bundle().pack().unwrap(), crate::platform::Actor::Cli).await.unwrap();
    (dir, shell)
}

/// An app that reads kv on GET / and writes it on POST / (testing-kv.wat).
pub fn kv_bundle() -> Bundle {
    Bundle {
        manifest: "[app]\nid = 'test'\nname = 'Test'\nversion = '1'\nowner = 'Tess Tester'\ngroup = 'Tests'\n[capabilities]\ngrant = ['kv-read', 'kv-write']\n\
                   [[data]]\nwhat = 'Notes'\nwhere = 'kv'\nwhy = 'Tests'\n"
            .into(),
        wasm: include_bytes!("testing-kv.wat").to_vec(),
        icon: None,
        deps: None,
        kit: None,
    }
}

pub fn bundle(version: &str, icon: Option<&str>) -> Bundle {
    Bundle {
        manifest: format!("[app]\nid = 'test'\nname = 'Test'\nversion = '{version}'\nowner = 'Tests'\ngroup = 'Tests'\n"),
        wasm: include_bytes!("testing.wat").to_vec(),
        icon: icon.map(str::to_string),
        deps: None,
        kit: None,
    }
}
