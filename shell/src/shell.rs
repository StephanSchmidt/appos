//! The shell: what every request and change reaches. `State` (state.rs)
//! holds what runs; the rest is fixed from startup.

use std::path::PathBuf;
use std::sync::Arc;

use crate::access::Policies;
use crate::auth::Auth;
use crate::kit::{Kits, LiveKits};
use crate::manifest::Risk;
use crate::outbox::Outbox;
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
    /// What apps call besides the page: kv, the platform's kits and the
    /// Wasm kits' live versions.
    pub kits: Arc<LiveKits>,
    /// Delivers the commands to the kits (outbox.rs).
    pub outbox: Outbox,
    /// Keep each route's last request that worked, for `appos review`.
    pub keep_examples: bool,
    /// What runs; every change goes through it.
    pub state: State,
    /// Sign-in, when `--auth` set it up.
    pub auth: Option<Auth>,
    /// Who may call what, when `--policy` set it up (access.rs).
    pub policy: Option<Arc<Policies>>,
    /// Deploys unzip, compile and run init one at a time, outside the
    /// state's lock: a zip bomb or a slow compile can't pile up.
    pub loads: tokio::sync::Semaphore,
    /// What reviews an app deploy before it goes live (`--review`).
    pub review: Option<String>,
    /// Reviews run one at a time.
    pub reviews: tokio::sync::Semaphore,
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
    /// The platform's own kits: the Wasm kits come as they load.
    pub kits: Kits,
    pub keep_examples: bool,
    pub review: Option<String>,
    pub auth: Option<Auth>,
    pub policy: Option<Arc<Policies>>,
}

impl Shell {
    /// Reads the state platform.db keeps and starts the usage writer and
    /// the kits' deliveries; in a Tokio runtime.
    pub fn new(p: Parts) -> anyhow::Result<Arc<Shell>> {
        let kits = Arc::new(LiveKits::new(p.kits));
        let state = State::open(p.platform.clone(), p.risk.clone(), kits.clone())?;
        let outbox = crate::outbox::start(p.db.clone(), p.platform.clone(), state.live(p.policy.clone()));
        // A kit that went live or on: its interfaces get a delivery task,
        // and waiting messages go at once.
        let (mut changes, wake) = (state.changes(), outbox.clone());
        tokio::spawn(async move {
            while changes.changed().await.is_ok() {
                wake.wake();
            }
        });
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
            outbox,
            kits,
            keep_examples: p.keep_examples,
            auth: p.auth,
            policy: p.policy,
            state,
            loads: tokio::sync::Semaphore::new(1),
            review: p.review,
            reviews: tokio::sync::Semaphore::new(1),
        }))
    }

    /// What is live, for kits: the apps that serve and the policy.
    pub fn live(&self) -> crate::state::Live {
        self.state.live(self.policy.clone())
    }
}
