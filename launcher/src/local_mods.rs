use std::path::Path;

use anyhow::Result;

use crate::keys;

/// Scan `dir` for mod folders (or symlinks to folders) and return their
/// paths relative to `/arma3/server/` where applicable, copying any
/// `.bikey` files found into the server's keys directory along the way.
pub fn mods(dir: &Path) -> Result<Vec<String>> {
    tracing::info!("Loading local mods from {}", dir.display());
    let mut mods = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let mod_dir = entry.path();

        let is_dir_or_dir_symlink = mod_dir.is_dir();
        if !is_dir_or_dir_symlink {
            continue;
        }

        let mod_dir_str = mod_dir.to_string_lossy().to_string();
        if let Some(rel) = mod_dir_str.strip_prefix("/arma3/server/") {
            mods.push(rel.to_string());
        } else {
            mods.push(mod_dir_str);
        }

        keys::copy(&mod_dir)?;
    }

    Ok(mods)
}
