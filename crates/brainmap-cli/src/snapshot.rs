use crate::{export, util, vault};
use anyhow::{Result, bail};
use std::fs;
use std::path::PathBuf;

pub fn create(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let _lock = util::acquire_vault_maintenance(&root)?;
    let id = util::id("snap", "snapshot");
    let dir = root.join("99-meta/backups");
    fs::create_dir_all(&dir)?;
    let out = dir.join(format!("{id}.brainmap.tar.zst"));
    export::export_portable_snapshot(&root, &out)?;
    util::write_atomic(&root.join(".brainmap/last-snapshot"), id.as_bytes())?;
    println!("snapshot {id}");
    Ok(())
}

pub fn list(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let dir = root.join("99-meta/backups");
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        println!("{}", entry?.path().display());
    }
    Ok(())
}

pub fn restore(vault: Option<PathBuf>, id: &str) -> Result<()> {
    util::validate_safe_component("snapshot id", id)?;
    let root = vault::resolve_vault(vault);
    let guard = util::acquire_vault_maintenance(&root)?;
    restore_locked(&root, id, &guard)
}

fn restore_locked(
    root: &std::path::Path,
    id: &str,
    guard: &util::VaultMaintenanceGuard,
) -> Result<()> {
    let file = root
        .join("99-meta/backups")
        .join(format!("{id}.brainmap.tar.zst"));
    if !file.exists() {
        bail!("snapshot not found: {id}")
    }
    let restore_file = root
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("{id}.restore.tmp.brainmap.tar.zst"));
    fs::copy(&file, &restore_file)?;
    let result = export::restore_archive_with_guard(&restore_file, root, None, guard);
    let _ = fs::remove_file(restore_file);
    result
}

pub fn rollback_last(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let guard = util::acquire_vault_maintenance(&root)?;
    let id = fs::read_to_string(root.join(".brainmap/last-snapshot"))?;
    util::validate_safe_component("snapshot id", id.trim())?;
    restore_locked(&root, id.trim(), &guard)
}
