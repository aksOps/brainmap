use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_iso() -> String {
    let now: DateTime<Utc> = Utc::now();
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn today() -> String {
    Utc::now().date_naive().to_string()
}

pub fn id(prefix: &str, seed: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    h.update(millis.to_string().as_bytes());
    format!("{prefix}_{millis}_{}", &hex::encode(h.finalize())[..8])
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s == "~" {
        return home_dir();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    path.to_path_buf()
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_vault() -> PathBuf {
    home_dir().join("BrainMap")
}

pub fn default_config() -> PathBuf {
    std::env::var_os("BRAINMAP_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config/brainmap/config.json"))
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    Ok(())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure_parent(path)?;
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|s| s.to_str()).unwrap_or("file")
    ));
    {
        let mut f = File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn append_jsonl(path: &Path, value: &serde_json::Value) -> Result<()> {
    ensure_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("append {}", path.display()))?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    Ok(())
}

pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    pub fn acquire(dir: &Path, name: &str) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                writeln!(file, "pid={}", std::process::id())?;
                Ok(Self { path })
            }
            Err(err) => bail!("lock already held at {}: {err}", path.display()),
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn safe_archive_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        bail!("archive path is absolute: {}", path.display());
    }
    for component in path.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("unsafe archive path: {}", path.display())
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
            let entry = entry?;
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap_or(&path);
            if rel
                .components()
                .any(|c| matches!(c, Component::Normal(s) if s == ".git" || s == "target" || s == "locks"))
            {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if root.exists() {
        walk(root, root, &mut out)?;
    }
    out.sort();
    Ok(out)
}

pub fn strip_optional_program_alias(mut args: Vec<OsString>) -> Vec<OsString> {
    if args.get(1).is_some_and(|a| a == "brainmap") {
        args.remove(1);
    }
    args
}
