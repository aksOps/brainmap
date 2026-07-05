use crate::{export, util, vault};
use anyhow::{Result, bail};
use std::fs;
use std::path::PathBuf;

pub fn create(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let id = format!("snap-{}", chrono::Utc::now().timestamp());
    let dir = root.join("99-meta/backups");
    fs::create_dir_all(&dir)?;
    let out = dir.join(format!("{id}.brainmap.tar.zst"));
    export::export_cmd(crate::cli::ExportArgs {
        mode: crate::cli::ExportMode::Portable,
        vault: Some(root.clone()),
        out: out.clone(),
        encrypt: false,
        recipient: None,
    })?;
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
    let root = vault::resolve_vault(vault);
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
    let result = export::restore_cmd(crate::cli::RestoreArgs {
        file: restore_file.clone(),
        to: root,
        identity: None,
    });
    let _ = fs::remove_file(restore_file);
    result
}

pub fn rollback_last(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let id = fs::read_to_string(root.join(".brainmap/last-snapshot"))?;
    restore(Some(root), id.trim())
}
