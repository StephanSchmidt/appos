//! The AppOS shell: a platform with only the built-in kits (kv, users).
//! `appos-shell --help` lists its options (args.rs).
//!
//! Each APP is a directory with an `app.toml`, or the `app.toml` itself.
//! `--risk` rates the grants from 0 to 10, one `grant = level` per line.
//! `--auth` sets up sign-in (auth.toml) and `--policy` says who may do what
//! (policy.csv); they go together. Without them the shell runs on this
//! machine only and prints a link with a token that signs in as an admin
//! who may do everything, and nobody else may do anything. `--no-auth` runs
//! it open: anyone may do anything, on the admin screen and the platform API
//! too.

#[tokio::main]
async fn main() {
    appos_shell::Platform::new().main().await
}
