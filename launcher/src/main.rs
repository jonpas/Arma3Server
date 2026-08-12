mod cache;
mod config;
mod keys;
mod launch;
mod local_mods;
mod steam;
mod workshop;

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use config::Config;
use steam::{CmPool, SyncTasks};
use tokio::sync::Semaphore;
use tracing::info;

const SERVER_ROOT: &str = "/arma3/server";
const CONNECTION_POOL_SIZE: usize = 4;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();

    raise_fd_limit();

    let process_start = std::time::Instant::now();
    info!("Starting Arma 3 Server...");

    let cfg = Config::from_env()?;
    let mut mods: Vec<String> = Vec::new();

    keys::ensure_keys_dir()?;

    info!("Logging in to Steam ({CONNECTION_POOL_SIZE} connections)...");
    let pool = CmPool::start(CONNECTION_POOL_SIZE, &cfg.steam_auth, Path::new(SERVER_ROOT)).await?;
    info!("Logged in to Steam ({CONNECTION_POOL_SIZE} connections)");

    let authed_at = std::time::Instant::now();
    info!("Logged in to Steam in {:.1}s", (authed_at - process_start).as_secs_f64());

    // Server/CDLC depots and workshop mods share one global download
    // concurrency budget and task pool for their chunk downloads (spawned
    // as soon as each plan/item resolves, not waited on inline).
    let sem = Arc::new(Semaphore::new(steam::SYNC_CONCURRENCY));
    let tasks: Mutex<SyncTasks> = Mutex::new(SyncTasks::new());
    let mut key_dirs: Vec<std::path::PathBuf> = Vec::new();

    // Cross-process record of what's already been fully chunk-verified at
    // its current manifest_id, plus advisory locking -- more than one
    // server instance can share this content directory, so resolution
    // needs to both skip dispatching a depot/mod that's already verified
    // *and* not race another instance verifying/downloading the same one
    // concurrently. Backed by SQLite (see cache::SyncState) specifically
    // for real cross-process safety, not just in-process.
    let sync_state = cache::SyncState::open(Path::new(SERVER_ROOT))?;

    // Server depot resolution and workshop resolution used to serialize on
    // one shared connection -- now each gets its own connection from the
    // pool and they run concurrently via tokio::join!, so a large server
    // depot resolution doesn't hold up workshop resolution starting (or
    // vice versa).
    let server_fut = async {
        let mut conn = pool.acquire().await;
        let cdlc_depot_ids = cfg.cdlc_depot_ids();
        steam::resolve_and_spawn_server(
            &mut conn,
            Path::new(SERVER_ROOT),
            cfg.wants_profiling(),
            &cdlc_depot_ids,
            sem.clone(),
            &tasks,
            sync_state.clone(),
        )
        .await
    };

    let workshop_fut = async {
        let Some(preset_path) = &cfg.mods_preset else {
            return Ok::<_, anyhow::Error>(None);
        };
        let mut conn = pool.acquire().await;
        let verify_preset = std::env::var("VERIFY_PRESET")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        let cdlc_force = std::env::var("CDLC_FORCE")
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        info!("VERIFY_PRESET is set to {verify_preset}");

        let result = workshop::preset(
            &mut conn,
            preset_path,
            verify_preset,
            cdlc_force,
            Path::new(SERVER_ROOT),
            sem.clone(),
            &tasks,
            sync_state.clone(),
        )
        .await?;
        Ok(Some(result))
    };

    let (server_result, workshop_result) = tokio::join!(server_fut, workshop_fut);
    server_result?;
    if let Some(result) = workshop_result? {
        mods.extend(result.mods);
        key_dirs = result.key_dirs;
    }

    let resolved_at = std::time::Instant::now();
    info!("Depot/mod resolution took {:.1}s", (resolved_at - authed_at).as_secs_f64());

    // Now wait for every spawned depot/mod sync (server + workshop,
    // interleaved) to finish before touching anything that depends on the
    // downloaded content (key copying, local mods, the actual launch).
    let mut tasks = tasks.into_inner().unwrap();
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    workshop::copy_keys(&key_dirs)?;

    pool.shutdown().await;

    if cfg.mods_local {
        let local_dir = Path::new(SERVER_ROOT).join("mods");
        if local_dir.exists() {
            mods.extend(local_mods::mods(&local_dir)?);
        }
    }

    launch::run(&cfg, mods, process_start, authed_at, resolved_at).await
}

/// Raise the process's open-file limit toward its hard ceiling. With up to
/// SYNC_CONCURRENCY depots/mods verifying concurrently -- each holding open
/// every one of its own verify-candidate files for the whole pass, plus
/// whatever HTTP/DNS sockets are in flight for chunk downloads -- the
/// default container limit (often 1024) is nowhere near enough (a single
/// depot alone can have 1000+ files). Best-effort: if this fails (e.g. the
/// container's hard limit itself is capped low), we log and continue
/// rather than fail startup over it.
fn raise_fd_limit() {
    match rlimit::increase_nofile_limit(65536) {
        Ok(limit) => tracing::debug!("raised open-file limit to {limit}"),
        Err(e) => tracing::warn!("failed to raise open-file limit: {e}"),
    }
}
