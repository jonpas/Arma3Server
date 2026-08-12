use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use regex::Regex;
use steamdepot::connection::CmConnection;
use tokio::sync::Semaphore;

use crate::cache;
use crate::keys;
use crate::steam::{self, SyncTasks};

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_9_3) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/35.0.1916.47 Safari/537.36";

/// Patch `meta.cpp` (publishedid) and, if `replace_app_id`, `mod.cpp`
/// (appId) the same way the old Python `_process_mod` did.
fn patch_mod_metadata(mod_dir: &Path, mod_id: u64, replace_app_id: bool) -> Result<()> {
    let meta_cpp = mod_dir.join("meta.cpp");
    if meta_cpp.exists() {
        let data = std::fs::read_to_string(&meta_cpp)?;
        let re_publishedid = Regex::new(r"publishedid\s*=\s*0\s*;").unwrap();
        let re_protocole = Regex::new(r"protocole").unwrap();
        let mut new_data = re_publishedid
            .replace(&data, format!("publishedid = {mod_id};").as_str())
            .into_owned();
        new_data = re_protocole.replace_all(&new_data, "protocol").into_owned();
        if new_data != data {
            tracing::info!("[{mod_id}] Updating {}", meta_cpp.display());
            std::fs::write(&meta_cpp, new_data)?;
        }
    }

    let mod_cpp = mod_dir.join("mod.cpp");
    if mod_cpp.exists() && replace_app_id {
        if let Ok(data) = std::fs::read_to_string(&mod_cpp) {
            let re_appid = Regex::new(r"appId\s*=\s*\d+\s*;").unwrap();
            let new_data = re_appid.replace(&data, "appId = 0;").into_owned();
            if new_data != data {
                tracing::info!("[{mod_id}] Replacing appId in {}", mod_cpp.display());
                std::fs::write(&mod_cpp, new_data)?;
            }
        } else {
            tracing::warn!("[{mod_id}] got bad mod.cpp");
        }
    }

    Ok(())
}

/// Result of resolving a mod preset: the `-mod=` params (every mod in the
/// preset, whether freshly downloaded or already up to date) and the
/// directories that need `.bikey` files copied out once downloads finish.
pub struct PresetResult {
    pub mods: Vec<String>,
    pub key_dirs: Vec<PathBuf>,
}

/// Parse a workshop mod preset (HTML export from a Steam Workshop
/// collection page) and spawn downloads for every candidate mod into the
/// shared `tasks` pool -- does not wait for them to finish. Every mod
/// always goes through full chunk-level verification (steam::
/// resolve_workshop_items' manifest cache + sync_depot's on-disk check),
/// same guarantee as server depots -- unlike the old `.manifest_gid`
/// shortcut this replaced, which skipped verification outright on a gid
/// match and just trusted the files were still correct. Only the network
/// round-trips (depot key, manifest bytes) get skipped on a cache hit, not
/// the actual correctness check. Each spawned download also handles its
/// own post-processing (meta.cpp/mod.cpp patching) once it completes,
/// since that has to happen after that specific item's content is
/// actually on disk.
pub async fn preset(
    conn: &mut CmConnection,
    mod_file: &str,
    verify_preset: bool,
    cdlc_force: bool,
    server_root: &Path,
    sem: Arc<Semaphore>,
    tasks: &Mutex<SyncTasks>,
    sync_state: Arc<cache::SyncState>,
) -> Result<PresetResult> {
    let html = if mod_file.starts_with("http") {
        let client = reqwest::Client::new();
        let body = client
            .get(mod_file)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
            .context("failed to fetch mod preset URL")?
            .text()
            .await?;
        std::fs::write("preset.html", &body)?;
        body
    } else {
        std::fs::read_to_string(mod_file)
            .with_context(|| format!("failed to read mod preset file {mod_file}"))?
    };

    let id_re = Regex::new(r#"filedetails/\?id=(\d+)""#).unwrap();
    let mod_ids: Vec<u64> = id_re
        .captures_iter(&html)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    tracing::info!("Found {} mods in preset", mod_ids.len());

    let replace_app_id = !cdlc_force;
    let workshop_root = server_root.join("workshop");

    // Skip existing mods entirely when not verifying (matches the old
    // VERIFY_PRESET=false fast path — no GetDetails call at all).
    let mut to_check = Vec::new();
    let mut mod_dirs: Vec<(u64, PathBuf)> = Vec::new();
    for &id in &mod_ids {
        let dir = workshop_root.join(id.to_string());
        mod_dirs.push((id, dir.clone()));
        if !verify_preset && dir.exists() {
            tracing::info!("[{id}] Skipping (already exists)");
            continue;
        }
        to_check.push(id);
    }

    // Every candidate goes through resolve_workshop_items, which handles
    // the depot-key/manifest caching itself (skipping network round-trips
    // on a hit) -- but always ends in a real sync_depot chunk verification,
    // same as server depots. Nothing here decides "trust it, skip
    // checking" based on cached metadata alone.
    let resolution = steam::resolve_workshop_items(conn, &workshop_root, &to_check, &sync_state)
        .await
        .context("failed to resolve workshop items for download")?;

    let http = reqwest::Client::new();
    for item in resolution.items {
        let item_dir = workshop_root.join(item.published_file_id.to_string());
        let pool = resolution.cdn_pool.clone();
        let http = http.clone();
        let sync_state = sync_state.clone();
        let id = item.published_file_id;

        let fut = async move {
            steam::download_one_depot(item.plan, item_dir.clone(), http, pool, sync_state).await?;
            patch_mod_metadata(&item_dir, id, replace_app_id)?;
            tracing::info!("[{id}] Finished");
            Ok(())
        };
        steam::spawn_bounded(tasks, sem.clone(), fut);
    }

    Ok(PresetResult {
        mods: mod_dirs
            .iter()
            .map(|(id, _)| format!("workshop/{id}"))
            .collect(),
        key_dirs: mod_dirs.into_iter().map(|(_, dir)| dir).collect(),
    })
}

/// Copy `.bikey` files for every mod directory in `key_dirs`. Call this
/// after all spawned downloads (see [`preset`]) have finished, since a mod
/// that just downloaded won't have its keys in place until then.
pub fn copy_keys(key_dirs: &[PathBuf]) -> Result<()> {
    for dir in key_dirs {
        keys::copy(dir)?;
    }
    Ok(())
}
