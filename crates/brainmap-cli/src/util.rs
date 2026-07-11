use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use fs4::FileExt;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static ID_SEQUENCE: AtomicU64 = AtomicU64::new(0);
#[cfg(windows)]
static WINDOWS_ATOMIC_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn now_iso() -> String {
    let now: DateTime<Utc> = Utc::now();
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn today() -> String {
    Utc::now().date_naive().to_string()
}

pub fn id(prefix: &str, seed: &str) -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let millis = elapsed.as_millis();
    let sequence = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    h.update(elapsed.as_nanos().to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(sequence.to_le_bytes());
    format!("{prefix}_{millis}_{}", &hex::encode(h.finalize())[..12])
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

pub fn default_project_scope() -> String {
    if let Ok(scope) = std::env::var("BRAINMAP_PROJECT_SCOPE")
        && scope.starts_with("project:")
        && scope.len() <= 160
        && scope.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '/' | '.')
        })
    {
        return scope;
    }
    let directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let canonical = directory.canonicalize().unwrap_or(directory);
    let label = canonical
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("local")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let hash = sha256_hex(canonical.to_string_lossy().as_bytes());
    format!("project:{}-{}", label.trim_matches('-'), &hash[..12])
}

pub fn resolve_learning_scope(scope: &str) -> String {
    if scope == "project:auto" {
        default_project_scope()
    } else {
        scope.to_string()
    }
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
    #[cfg(windows)]
    let _windows_atomic_write_guard = WINDOWS_ATOMIC_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("Windows atomic-write lock poisoned");
    ensure_parent(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("brainmap-file");
    let tmp = parent.join(format!(".{file_name}.{}.tmp", id("write", file_name)));
    let result = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        replace_file_atomic(&tmp, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        remove_file_with_retry(&tmp);
    }
    result
}

fn remove_file_with_retry(path: &Path) {
    for attempt in 0..50 {
        match fs::remove_file(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) if retryable_windows_io_error(&error) && attempt < 49 => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return,
        }
    }
}

fn retryable_windows_io_error(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

pub fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .with_context(|| format!("open directory {} for sync", path.display()))?
            .sync_all()
            .with_context(|| format!("sync directory {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn sync_file(path: &Path) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("open {} for sync", path.display()))?
        .sync_all()
        .with_context(|| format!("sync {}", path.display()))
}

pub fn rename_and_sync(staged: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::rename(staged, target)
        .with_context(|| format!("rename {} to {}", staged.display(), target.display()))?;
    sync_directory(parent)
}

pub fn remove_file_and_sync(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => sync_directory(path.parent().unwrap_or_else(|| Path::new("."))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

pub fn remove_dir_all_and_sync(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => sync_directory(path.parent().unwrap_or_else(|| Path::new("."))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("remove {}", path.display())),
    }
}

#[cfg(not(windows))]
pub fn replace_file_atomic(staged: &Path, target: &Path) -> Result<()> {
    fs::rename(staged, target)
        .with_context(|| format!("replace {} with {}", target.display(), staged.display()))?;
    Ok(())
}

#[cfg(windows)]
pub fn replace_file_atomic(staged: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACEFILE_WRITE_THROUGH,
        ReplaceFileW,
    };

    let staged = staged
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    for attempt in 0..50 {
        let replaced = unsafe {
            ReplaceFileW(
                target.as_ptr(),
                staged.as_ptr(),
                null(),
                REPLACEFILE_WRITE_THROUGH,
                null(),
                null(),
            )
        };
        if replaced != 0 {
            return Ok(());
        }
        let moved = unsafe {
            MoveFileExW(
                staged.as_ptr(),
                target.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        let retryable = matches!(error.raw_os_error(), Some(5 | 32 | 33));
        if !retryable || attempt == 49 {
            return Err(error).context("atomically replace Windows file");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Ok(())
}

pub struct LockedJsonl {
    file: File,
    path: PathBuf,
    #[cfg(windows)]
    windows_mutex: WindowsJsonlMutex,
}

impl LockedJsonl {
    pub fn append(&mut self, value: &serde_json::Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        self.file
            .seek(SeekFrom::End(0))
            .with_context(|| format!("seek {}", self.path.display()))?;
        self.file
            .write_all(&bytes)
            .with_context(|| format!("append {}", self.path.display()))?;
        self.file
            .sync_data()
            .with_context(|| format!("sync {}", self.path.display()))
    }

    pub fn read_all(&mut self) -> Result<Vec<u8>> {
        self.file
            .seek(SeekFrom::Start(0))
            .with_context(|| format!("seek {}", self.path.display()))?;
        let mut bytes = Vec::new();
        self.file
            .read_to_end(&mut bytes)
            .with_context(|| format!("read {}", self.path.display()))?;
        Ok(bytes)
    }
}

impl Drop for LockedJsonl {
    fn drop(&mut self) {
        #[cfg(not(windows))]
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(windows)]
struct WindowsJsonlMutex {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsJsonlMutex {
    fn acquire(path: &Path) -> Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        use std::ptr::null;
        use windows_sys::Win32::Foundation::{
            CloseHandle, GetLastError, WAIT_ABANDONED, WAIT_FAILED, WAIT_OBJECT_0,
        };
        use windows_sys::Win32::System::Threading::{CreateMutexW, INFINITE, WaitForSingleObject};

        let normalized = path
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        let name = format!("Local\\BrainMapJsonl-{}", sha256_hex(normalized.as_bytes()));
        let name = name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error()).context("create Windows JSONL mutex");
        }
        let wait = unsafe { WaitForSingleObject(handle, INFINITE) };
        if !matches!(wait, WAIT_OBJECT_0 | WAIT_ABANDONED) {
            let error = if wait == WAIT_FAILED {
                std::io::Error::from_raw_os_error(unsafe { GetLastError() } as i32)
            } else {
                std::io::Error::other(format!("unexpected mutex wait result {wait}"))
            };
            unsafe { CloseHandle(handle) };
            return Err(error).context("wait for Windows JSONL mutex");
        }
        Ok(Self { handle })
    }
}

#[cfg(windows)]
impl Drop for WindowsJsonlMutex {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

pub fn lock_jsonl(path: &Path) -> Result<LockedJsonl> {
    ensure_parent(path)?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("append {}", path.display()))?;
    #[cfg(windows)]
    let windows_mutex = WindowsJsonlMutex::acquire(path)?;
    #[cfg(not(windows))]
    for attempt in 0..50 {
        match FileExt::lock(&file) {
            Ok(()) => break,
            Err(error) if retryable_windows_io_error(&error) && attempt < 49 => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("lock {}", path.display()));
            }
        }
    }
    Ok(LockedJsonl {
        file,
        path: path.to_path_buf(),
        #[cfg(windows)]
        windows_mutex,
    })
}

pub fn append_jsonl(path: &Path, value: &serde_json::Value) -> Result<()> {
    lock_jsonl(path)?.append(value)
}

#[derive(Debug)]
pub struct FileLock {
    file: File,
}

impl FileLock {
    pub fn acquire(dir: &Path, name: &str) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(name);
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        let mut owner = String::new();
        let _ = file.read_to_string(&mut owner);
        let _ = file.seek(SeekFrom::Start(0));
        if let Err(error) = FileExt::try_lock(&file) {
            let owner = if owner.trim().is_empty() {
                "owner metadata unavailable".to_string()
            } else {
                owner.trim().to_string()
            };
            bail!("lock already held at {} ({owner}): {error}", path.display());
        }
        file.set_len(0)?;
        writeln!(file, "pid={}", std::process::id())?;
        file.sync_data()?;
        Ok(Self { file })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Debug)]
pub struct VaultMaintenanceGuard {
    _lock: FileLock,
    identity: PathBuf,
}

impl VaultMaintenanceGuard {
    pub fn require_target(&self, root: &Path) -> Result<()> {
        let (_, identity) = vault_lock_identity(root)?;
        if identity != self.identity {
            bail!(
                "maintenance guard protects {}, not {}",
                self.identity.display(),
                identity.display()
            );
        }
        Ok(())
    }
}

pub fn acquire_vault_maintenance(root: &Path) -> Result<VaultMaintenanceGuard> {
    let (canonical_parent, identity) = vault_lock_identity(root)?;
    let key = sha256_hex(identity.to_string_lossy().as_bytes());
    let lock = FileLock::acquire(
        &canonical_parent.join(".brainmap-vault-locks"),
        &format!("vault-{}.lock", &key[..32]),
    )?;
    Ok(VaultMaintenanceGuard {
        _lock: lock,
        identity,
    })
}

fn vault_lock_identity(root: &Path) -> Result<(PathBuf, PathBuf)> {
    let expanded = expand_tilde(root);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()?.join(expanded)
    };
    if fs::symlink_metadata(&absolute).is_ok() {
        let identity = absolute
            .canonicalize()
            .with_context(|| format!("resolve vault path {}", absolute.display()))?;
        let canonical_parent = identity
            .parent()
            .context("vault path must have a parent directory")?
            .to_path_buf();
        return Ok((canonical_parent, identity));
    }
    let parent = absolute
        .parent()
        .context("vault path must have a parent directory")?;
    fs::create_dir_all(parent)?;
    let canonical_parent = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    let target_name = absolute
        .file_name()
        .context("vault path must name a directory")?;
    let identity = canonical_parent.join(target_name);
    Ok((canonical_parent, identity))
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
    Ok(normalized.to_string_lossy().replace('\\', "/"))
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

    #[test]
    fn concurrent_identical_requests_receive_unique_ids() {
        let ids = std::thread::scope(|scope| {
            let handles = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        (0..100)
                            .map(|_| id("dec", "identical request"))
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn automatic_learning_scope_is_project_narrow_and_stable() {
        let first = resolve_learning_scope("project:auto");
        let second = resolve_learning_scope("project:auto");
        assert!(first.starts_with("project:"));
        assert_ne!(first, "project:auto");
        assert_eq!(first, second);
        assert_eq!(resolve_learning_scope("global"), "global");
    }

    #[test]
    fn concurrent_atomic_writes_never_share_temporary_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("policy.md");
        let candidates = (0..16)
            .map(|index| format!("complete policy body {index}\n"))
            .collect::<Vec<_>>();
        std::thread::scope(|scope| {
            let handles = candidates
                .iter()
                .map(|body| scope.spawn(|| write_atomic(&path, body.as_bytes())))
                .collect::<Vec<_>>();
            for handle in handles {
                handle.join().unwrap().unwrap();
            }
        });

        let final_body = fs::read_to_string(&path).unwrap();
        assert!(candidates.contains(&final_body));
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_jsonl_appends_remain_complete() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ledger.jsonl");
        std::thread::scope(|scope| {
            let handles = (0..16)
                .map(|worker| {
                    let path = &path;
                    scope.spawn(move || {
                        for event in 0..100 {
                            append_jsonl(
                                path,
                                &serde_json::json!({"worker": worker, "event": event}),
                            )
                            .unwrap();
                        }
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                handle.join().unwrap();
            }
        });
        let text = fs::read_to_string(path).unwrap();
        let events = text
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<serde_json::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(events.len(), 1_600);
    }

    #[test]
    fn file_lock_recovers_after_the_owner_drops() {
        let tmp = tempfile::tempdir().unwrap();
        let first = FileLock::acquire(tmp.path(), "operation.lock").unwrap();
        assert!(FileLock::acquire(tmp.path(), "operation.lock").is_err());
        drop(first);
        FileLock::acquire(tmp.path(), "operation.lock").unwrap();
    }

    #[test]
    fn file_lock_recovers_after_a_process_is_killed() {
        let tmp = tempfile::tempdir().unwrap();
        let ready = tmp.path().join("ready");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "util::tests::lock_owner_child", "--nocapture"])
            .env("BRAINMAP_LOCK_OWNER_DIR", tmp.path())
            .env("BRAINMAP_LOCK_OWNER_READY", &ready)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready.exists(), "child did not acquire the process lock");
        let active_error = FileLock::acquire(tmp.path(), "process.lock").unwrap_err();
        let active_error = active_error.to_string();
        #[cfg(not(windows))]
        assert!(active_error.contains("pid="));
        #[cfg(windows)]
        assert!(active_error.contains("lock already held"));

        child.kill().unwrap();
        child.wait().unwrap();
        FileLock::acquire(tmp.path(), "process.lock").unwrap();
    }

    #[test]
    fn vault_maintenance_lock_survives_directory_swap_across_processes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        let backup = tmp.path().join("BrainMap.backup");
        let ready = tmp.path().join("vault-ready");
        fs::create_dir(&root).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "util::tests::vault_maintenance_lock_owner_child",
                "--nocapture",
            ])
            .env("BRAINMAP_MAINTENANCE_ROOT", &root)
            .env("BRAINMAP_MAINTENANCE_READY", &ready)
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if ready.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ready.exists(), "child did not acquire the maintenance lock");

        fs::rename(&root, &backup).unwrap();
        fs::create_dir(&root).unwrap();
        let active_error = acquire_vault_maintenance(&root).unwrap_err();
        assert!(active_error.to_string().contains("lock already held"));

        child.kill().unwrap();
        child.wait().unwrap();
        acquire_vault_maintenance(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn vault_maintenance_lock_contends_through_a_symlink_alias() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let real_root = tmp.path().join("BrainMap");
        let alias_root = tmp.path().join("BrainMapAlias");
        fs::create_dir(&real_root).unwrap();
        symlink(&real_root, &alias_root).unwrap();

        let real_guard = acquire_vault_maintenance(&real_root).unwrap();
        let alias_error = acquire_vault_maintenance(&alias_root).unwrap_err();
        assert!(alias_error.to_string().contains("lock already held"));
        real_guard.require_target(&alias_root).unwrap();

        drop(real_guard);
        acquire_vault_maintenance(&alias_root).unwrap();
    }

    #[test]
    fn lock_owner_child() {
        let (Ok(dir), Ok(ready)) = (
            std::env::var("BRAINMAP_LOCK_OWNER_DIR"),
            std::env::var("BRAINMAP_LOCK_OWNER_READY"),
        ) else {
            return;
        };
        let _lock = FileLock::acquire(Path::new(&dir), "process.lock").unwrap();
        fs::write(ready, b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }

    #[test]
    fn vault_maintenance_lock_owner_child() {
        let (Ok(root), Ok(ready)) = (
            std::env::var("BRAINMAP_MAINTENANCE_ROOT"),
            std::env::var("BRAINMAP_MAINTENANCE_READY"),
        ) else {
            return;
        };
        let _lock = acquire_vault_maintenance(Path::new(&root)).unwrap();
        fs::write(ready, b"ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
