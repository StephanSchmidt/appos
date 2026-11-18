//! Builds the CRM's fake (crm-fake/) for wasm32-wasip2: `Crm::fake` hands
//! it to `appos sync`, for appos dev.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    for path in ["crm-fake/src", "crm-fake/Cargo.toml", "wit"] {
        println!("cargo::rerun-if-changed={path}");
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--target",
            "wasm32-wasip2",
            "--manifest-path",
            "crm-fake/Cargo.toml",
            "--target-dir",
        ])
        .arg(out.join("crm-fake"))
        // The outer build's flags are for the platform, not for wasm.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("CARGO_BUILD_TARGET")
        .status()
        .expect("cargo runs");
    assert!(
        status.success(),
        "the CRM's fake did not build; it needs the wasm32-wasip2 target: rustup target add wasm32-wasip2"
    );
    let wasm = out.join("crm-fake/wasm32-wasip2/release/crm_fake.wasm");
    std::fs::copy(&wasm, out.join("crm-fake.wasm")).expect("the fake's wasm");
}
