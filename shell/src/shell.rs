//! The shell: what every request and change reaches. `State` (state.rs)
//! holds what runs; the rest is fixed from startup.

use std::path::PathBuf;
use std::sync::Arc;

use crate::access::Policies;
use crate::auth::Auth;
use crate::manifest::Risk;
use crate::platform::PlatformDb;
use crate::state::State;
use crate::tx::Db;
use crate::usage::Recorder;

pub struct Shell {
    pub origin: String,
    pub name: String,
    pub git: Option<String>,
    pub engine: wasmtime::Engine,
    /// The apps' kv.
    pub db: Db,
    /// Where the shell keeps kv, the platform database and deployed apps.
    pub data: PathBuf,
    /// Versions, events and usage.
    pub platform: PlatformDb,
    pub usage: Recorder,
    /// The grants' risk levels, as the company rates them.
    pub risk: Risk,
    /// Keep each route's last request that worked, for `appos review`.
    pub keep_examples: bool,
    /// What runs; every change goes through it.
    pub state: State,
    /// Sign-in, when `--auth` set it up.
    pub auth: Option<Auth>,
    /// Who may call what, when `--policy` set it up (access.rs).
    pub policy: Option<Policies>,
    /// Deploys unzip, compile and run init one at a time, outside the
    /// state's lock: a zip bomb or a slow compile can't pile up.
    pub loads: tokio::sync::Semaphore,
}

/// What a shell is made of.
pub struct Parts {
    pub origin: String,
    pub name: String,
    pub git: Option<String>,
    pub engine: wasmtime::Engine,
    pub db: Db,
    pub data: PathBuf,
    pub platform: PlatformDb,
    pub risk: Risk,
    pub keep_examples: bool,
    pub auth: Option<Auth>,
    pub policy: Option<Policies>,
}

impl Shell {
    /// Reads the state platform.db keeps and starts the usage writer; in a
    /// Tokio runtime.
    pub fn new(p: Parts) -> anyhow::Result<Arc<Shell>> {
        let state = State::open(p.platform.clone(), p.risk.clone())?;
        Ok(Arc::new(Shell {
            origin: p.origin,
            name: p.name,
            git: p.git,
            engine: p.engine,
            db: p.db,
            data: p.data,
            usage: crate::usage::start(p.platform.clone()),
            platform: p.platform,
            risk: p.risk,
            keep_examples: p.keep_examples,
            auth: p.auth,
            policy: p.policy,
            state,
            loads: tokio::sync::Semaphore::new(1),
        }))
    }
}
