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

pub fn normalize_archive_path(path: &Path) -> Result<String> {
    let portable = path.to_string_lossy().replace('\\', "/");
    let portable_path = Path::new(&portable);
    safe_archive_path(portable_path)?;
    let mut normalized = PathBuf::new();
    for component in portable_path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("unsafe archive path: {}", path.display())
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        bail!("empty archive path");
    }
    Ok(normalized.to_string_lossy().to_string())
}

pub fn portable_archive_collision_key(normalized_path: &str) -> Result<String> {
    let mut key = Vec::new();
    for component in normalized_path.split('/') {
        if component.is_empty()
            || component.ends_with(['.', ' '])
            || component
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        {
            bail!("archive path is not portable: {normalized_path}");
        }
        // Windows uses an uppercase table for case-insensitive filename comparison.
        // Rust's Unicode uppercase expansion also folds pairs such as Greek σ/ς.
        let folded = component.to_uppercase();
        let stem = folded.split('.').next().unwrap_or_default();
        let reserved = matches!(stem, "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
                });
        if reserved {
            bail!("archive path uses a reserved Windows name: {normalized_path}");
        }
        key.push(folded);
    }
    Ok(key.join("/"))
}

pub fn validate_safe_component(label: &str, value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 160
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
    if !valid {
        bail!("invalid {label}: use 1-160 ASCII letters, numbers, hyphens, or underscores");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_path_normalization_rejects_windows_style_traversal() {
        assert!(normalize_archive_path(Path::new("..\\escape")).is_err());
        assert_eq!(
            normalize_archive_path(Path::new("notes/./policy.md")).unwrap(),
            "notes/policy.md"
        );
    }

    #[test]
    fn portable_archive_keys_fold_case_and_reject_windows_names() {
        assert_eq!(
            portable_archive_collision_key("Notes/Policy.md").unwrap(),
            portable_archive_collision_key("notes/policy.MD").unwrap()
        );
        assert_eq!(
            portable_archive_collision_key("notes/σ.md").unwrap(),
            portable_archive_collision_key("notes/ς.md").unwrap()
        );
        assert!(portable_archive_collision_key("notes/CON.md").is_err());
        assert!(portable_archive_collision_key("notes/policy. ").is_err());
    }
}
