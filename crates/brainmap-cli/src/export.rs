use crate::cli::{ExportArgs, ExportMode, ImportArgs, RestoreArgs, VerifyExportArgs};
use crate::{gate, index, privacy, util, vault};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};
use tar::{Archive, Builder, Header};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    #[serde(rename = "formatVersion")]
    format_version: u32,
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "brainmapVersion")]
    brainmap_version: String,
    #[serde(rename = "exportMode")]
    export_mode: String,
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    #[serde(rename = "includesIndexes")]
    includes_indexes: bool,
    #[serde(rename = "includesEmbeddings")]
    includes_embeddings: bool,
    #[serde(rename = "includesPrivateNotes")]
    includes_private_notes: bool,
    encrypted: bool,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    path: String,
    sha256: String,
}

pub fn export_cmd(args: ExportArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let _lock = util::acquire_vault_maintenance(&root)?;
    if args.encrypt || matches!(args.mode, ExportMode::Encrypted) {
        let recipient = args
            .recipient
            .as_deref()
            .context("encrypted export requires --recipient age1...")?;
        export_encrypted_archive(&root, &args.out, args.mode, recipient)?;
    } else {
        export_archive(&root, &args.out, args.mode)?;
    }
    println!("exported {}", args.out.display());
    Ok(())
}

fn export_archive(root: &Path, out: &Path, mode: ExportMode) -> Result<()> {
    util::write_atomic(out, &archive_bytes(root, mode, false)?)?;
    Ok(())
}

pub(crate) fn export_portable_snapshot(root: &Path, out: &Path) -> Result<()> {
    export_archive(root, out, ExportMode::Portable)
}

fn export_encrypted_archive(
    root: &Path,
    out: &Path,
    mode: ExportMode,
    recipient: &str,
) -> Result<()> {
    let archive = archive_bytes(root, mode, true)?;
    util::write_atomic(out, &encrypt_bytes(&archive, recipient)?)?;
    Ok(())
}

fn archive_bytes(root: &Path, mode: ExportMode, encrypted: bool) -> Result<Vec<u8>> {
    let mut entries = Vec::<(String, Vec<u8>)>::new();
    let mut portable_keys = HashSet::new();
    for path in util::collect_files(root)? {
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if should_skip(&rel, &mode) {
            continue;
        }
        if privacy::contains_secret(&rel) {
            if matches!(mode, ExportMode::ShareSafe) {
                continue;
            }
            bail!("refusing to export secret-like archive path: {rel}");
        }
        let source_bytes = fs::read(&path)?;
        if export_file_is_secret(&source_bytes) {
            if matches!(mode, ExportMode::ShareSafe) {
                continue;
            }
            bail!("refusing to export secret-like or secret-classified file: {rel}");
        }
        let collision_key = util::portable_archive_collision_key(&rel)?;
        if !portable_keys.insert(collision_key) {
            bail!("portable archive path collision: {rel}");
        }
        let mut bytes = source_bytes;
        if matches!(mode, ExportMode::ShareSafe) {
            bytes = privacy::redact(&String::from_utf8_lossy(&bytes)).into_bytes();
        }
        entries.push((rel, bytes));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let files = entries
        .iter()
        .map(|(path, bytes)| ManifestFile {
            path: path.clone(),
            sha256: util::sha256_hex(bytes),
        })
        .collect::<Vec<_>>();
    let manifest = Manifest {
        format: "brainmap-export".into(),
        format_version: 1,
        created_at: util::now_iso(),
        brainmap_version: env!("CARGO_PKG_VERSION").into(),
        export_mode: format!("{mode:?}").to_lowercase(),
        schema_version: index::COMPILED_SCHEMA_VERSION.into(),
        includes_indexes: matches!(mode, ExportMode::Full),
        includes_embeddings: matches!(mode, ExportMode::Full),
        includes_private_notes: !matches!(mode, ExportMode::ShareSafe),
        encrypted,
        files,
    };
    let encoder = zstd::Encoder::new(Vec::new(), 3)?;
    let mut tar = Builder::new(encoder);
    append_bytes(
        &mut tar,
        "manifest.json",
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    for (path, bytes) in entries {
        append_bytes(&mut tar, &path, &bytes)?;
    }
    let encoder = tar.into_inner()?;
    encoder.finish().map_err(Into::into)
}

fn export_file_is_secret(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    privacy::contains_secret(&text)
        || text.lines().any(|line| {
            line.split_once(':').is_some_and(|(key, value)| {
                key.trim().eq_ignore_ascii_case("sensitivity")
                    && value.trim().eq_ignore_ascii_case("secret")
            })
        })
}

fn should_skip(rel: &str, mode: &ExportMode) -> bool {
    if rel.starts_with("99-meta/backups/") || rel == ".brainmap/last-snapshot" {
        return true;
    }
    if rel.contains(".brainmap/locks") {
        return true;
    }
    if rel.contains(".brainmap/web-cache")
        || rel.contains(".brainmap/models")
        || rel.ends_with("brainmap.sqlite")
        || rel.ends_with("brainmap.sqlite-wal")
        || rel.ends_with("brainmap.sqlite-shm")
    {
        return !matches!(mode, ExportMode::Full);
    }
    if matches!(mode, ExportMode::ShareSafe)
        && (rel.contains("decision-ledger") || rel.contains("pending-update-packets"))
    {
        return true;
    }
    false
}

fn append_bytes<W: std::io::Write>(tar: &mut Builder<W>, path: &str, bytes: &[u8]) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, path, Cursor::new(bytes))?;
    Ok(())
}

pub fn verify_export_cmd(args: VerifyExportArgs) -> Result<()> {
    let verified = verify_archive(&args.file, args.identity.as_deref())?;
    println!(
        "verify ok: {} files, mode {}",
        verified.manifest.files.len(),
        verified.manifest.export_mode
    );
    Ok(())
}

pub(crate) fn verify_export_archive(file: &Path, identity: Option<&Path>) -> Result<()> {
    verify_archive(file, identity).map(|_| ())
}

#[derive(Debug)]
struct VerifiedArchive {
    manifest: Manifest,
    entries: Vec<(String, Vec<u8>)>,
}

fn verify_archive(file: &Path, identity: Option<&Path>) -> Result<VerifiedArchive> {
    let entries = read_archive(file, identity)?;
    let mut archive_paths = HashSet::new();
    let mut archive_collision_keys = HashSet::new();
    let mut entry_bytes = HashMap::new();
    for (path, bytes) in &entries {
        if path != "manifest.json" && privacy::contains_secret(path) {
            bail!("refusing secret-like archive path: {path}");
        }
        if !archive_paths.insert(path.as_str()) {
            bail!("duplicate archive entry: {path}");
        }
        let collision_key = util::portable_archive_collision_key(path)?;
        if !archive_collision_keys.insert(collision_key) {
            bail!("portable archive path collision: {path}");
        }
        entry_bytes.insert(path.as_str(), bytes.as_slice());
    }
    let manifest_bytes = entry_bytes
        .get("manifest.json")
        .copied()
        .context("missing manifest.json")?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)?;
    if manifest.format != "brainmap-export" {
        bail!("invalid export format");
    }
    if manifest.format_version != 1 {
        bail!(
            "unsupported export format version: {}",
            manifest.format_version
        );
    }
    if !matches!(
        manifest.schema_version.as_str(),
        "decision-engine-v2" | "decision-engine-v3" | "decision-engine-v4"
    ) {
        bail!(
            "unsupported decision engine schema: {}",
            manifest.schema_version
        );
    }
    let mut manifest_paths = HashSet::new();
    let mut manifest_collision_keys = HashSet::new();
    for file in &manifest.files {
        let normalized_path = util::normalize_archive_path(Path::new(&file.path))?;
        if privacy::contains_secret(&normalized_path) {
            bail!("refusing secret-like archive path: {}", file.path);
        }
        if normalized_path == "manifest.json" {
            bail!("manifest.json cannot list itself");
        }
        if !manifest_paths.insert(normalized_path.clone()) {
            bail!("duplicate manifest path: {}", file.path);
        }
        let collision_key = util::portable_archive_collision_key(&normalized_path)?;
        if !manifest_collision_keys.insert(collision_key) {
            bail!("portable manifest path collision: {}", file.path);
        }
        let bytes = entry_bytes
            .get(normalized_path.as_str())
            .copied()
            .with_context(|| format!("missing {}", file.path))?;
        if export_file_is_secret(bytes) {
            bail!("refusing secret-like archive content: {}", file.path);
        }
        let got = util::sha256_hex(bytes);
        if got != file.sha256 {
            bail!("checksum mismatch for {}", file.path);
        }
    }
    if export_file_is_secret(manifest_bytes) {
        bail!("refusing secret-like archive manifest metadata");
    }
    for path in archive_paths {
        if path != "manifest.json" && !manifest_paths.contains(path) {
            bail!("unmanifested archive entry: {path}");
        }
    }
    Ok(VerifiedArchive { manifest, entries })
}

fn read_archive(file: &Path, identity: Option<&Path>) -> Result<Vec<(String, Vec<u8>)>> {
    let bytes = archive_plaintext(file, identity)?;
    read_archive_bytes(&bytes)
}

fn read_archive_bytes(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let decoded = decode_strict_zstd_frame(bytes)?;
    let mut archive = Archive::new(Cursor::new(decoded));
    let mut entries = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();
        let path_s = util::normalize_archive_path(&path)?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        entries.push((path_s, bytes));
    }
    Ok(entries)
}

fn decode_strict_zstd_frame(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = zstd::Decoder::new(Cursor::new(bytes))?.single_frame();
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded)?;
    let mut reader = decoder.finish();
    if !reader.fill_buf()?.is_empty() {
        bail!("trailing data after zstd frame");
    }
    Ok(decoded)
}

pub fn import_cmd(args: ImportArgs) -> Result<()> {
    if args.dry_run {
        verify_archive(&args.file, args.identity.as_deref())?;
        println!(
            "import dry-run ok: {} -> {}",
            args.file.display(),
            args.to.display()
        );
        return Ok(());
    }
    restore_archive(&args.file, &args.to, args.identity.as_deref())?;
    Ok(())
}

pub fn restore_cmd(args: RestoreArgs) -> Result<()> {
    restore_archive_with_fault(
        &args.file,
        &args.to,
        args.identity.as_deref(),
        args.fault_phase.map(RestorePhase::from),
    )
}

fn restore_archive(file: &Path, to: &Path, identity: Option<&Path>) -> Result<()> {
    restore_archive_with_fault(file, to, identity, None)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestorePhase {
    Verified,
    StagingCreated,
    FilesWritten,
    IndexRebuilt,
    LinksChecked,
    GateChecked,
    ExistingBackedUp,
    StagingActivated,
}

impl From<crate::cli::RestoreFaultPhase> for RestorePhase {
    fn from(value: crate::cli::RestoreFaultPhase) -> Self {
        use crate::cli::RestoreFaultPhase;
        match value {
            RestoreFaultPhase::Verified => Self::Verified,
            RestoreFaultPhase::StagingCreated => Self::StagingCreated,
            RestoreFaultPhase::FilesWritten => Self::FilesWritten,
            RestoreFaultPhase::IndexRebuilt => Self::IndexRebuilt,
            RestoreFaultPhase::LinksChecked => Self::LinksChecked,
            RestoreFaultPhase::GateChecked => Self::GateChecked,
            RestoreFaultPhase::ExistingBackedUp => Self::ExistingBackedUp,
            RestoreFaultPhase::StagingActivated => Self::StagingActivated,
        }
    }
}

fn restore_archive_with_fault(
    file: &Path,
    to: &Path,
    identity: Option<&Path>,
    fault: Option<RestorePhase>,
) -> Result<()> {
    let verified = verify_archive(file, identity)?;
    fail_restore_at(RestorePhase::Verified, fault)?;
    let target = restore_target_path(to)?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let _lock = util::acquire_vault_maintenance(&target)?;
    restore_verified_archive_with_fault(verified, &target, fault)
}

pub(crate) fn restore_archive_with_guard(
    file: &Path,
    to: &Path,
    identity: Option<&Path>,
    guard: &util::VaultMaintenanceGuard,
) -> Result<()> {
    let target = restore_target_path(to)?;
    guard.require_target(&target)?;
    let verified = verify_archive(file, identity)?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    restore_verified_archive_with_fault(verified, &target, None)
}

fn restore_target_path(to: &Path) -> Result<PathBuf> {
    let expanded = util::expand_tilde(to);
    if fs::symlink_metadata(&expanded).is_ok() {
        return expanded
            .canonicalize()
            .with_context(|| format!("resolve restore target {}", expanded.display()));
    }
    Ok(expanded)
}

fn restore_verified_archive_with_fault(
    verified: VerifiedArchive,
    target: &Path,
    fault: Option<RestorePhase>,
) -> Result<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let target_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("BrainMap");
    let operation_id = util::id("restore", target_name);
    let staging = parent.join(format!(".{target_name}.{operation_id}.staging"));
    let backup = parent.join(format!(".{target_name}.{operation_id}.backup"));

    fs::create_dir(&staging)
        .with_context(|| format!("create restore staging vault {}", staging.display()))?;
    if let Err(error) = fail_restore_at(RestorePhase::StagingCreated, fault) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let staged = (|| -> Result<()> {
        for (path, bytes) in verified.entries {
            if path == "manifest.json" {
                continue;
            }
            let rel = PathBuf::from(&path);
            reject_hidden_traversal(&rel)?;
            let out = staging.join(rel);
            util::write_atomic(&out, &bytes)?;
        }
        fail_restore_at(RestorePhase::FilesWritten, fault)?;
        index::rebuild(&staging)?;
        fail_restore_at(RestorePhase::IndexRebuilt, fault)?;
        vault::link_check(&staging)?;
        fail_restore_at(RestorePhase::LinksChecked, fault)?;
        let _ = gate::evaluate(
            &staging,
            gate::GateInput {
                intent: "would-ask-user".into(),
                situation: "restore smoke test".into(),
                options: vec!["do nothing".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )?;
        fail_restore_at(RestorePhase::GateChecked, fault)
    })();
    if let Err(error) = staged {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let had_existing_target = target.exists();
    if had_existing_target {
        fs::rename(target, &backup).with_context(|| {
            format!(
                "move existing vault {} to backup {}",
                target.display(),
                backup.display()
            )
        })?;
    }
    if let Err(error) = fail_restore_at(RestorePhase::ExistingBackedUp, fault) {
        rollback_restore_swap(target, &staging, &backup, had_existing_target)?;
        return Err(error);
    }

    if let Err(error) = fs::rename(&staging, target) {
        rollback_restore_swap(target, &staging, &backup, had_existing_target)?;
        return Err(error).with_context(|| {
            format!(
                "activate staged vault {} at {}",
                staging.display(),
                target.display()
            )
        });
    }
    sync_directory(parent)?;
    fail_restore_at(RestorePhase::StagingActivated, fault)?;

    if had_existing_target {
        println!("backed up existing target to {}", backup.display());
    }
    println!("restored {}", target.display());
    Ok(())
}

fn fail_restore_at(phase: RestorePhase, fault: Option<RestorePhase>) -> Result<()> {
    if fault == Some(phase) {
        bail!("injected restore failure at {phase:?}");
    }
    Ok(())
}

fn rollback_restore_swap(
    target: &Path,
    staging: &Path,
    backup: &Path,
    had_existing_target: bool,
) -> Result<()> {
    if target.exists() {
        fs::remove_dir_all(target)
            .with_context(|| format!("remove failed restored vault {}", target.display()))?;
    }
    if had_existing_target && backup.exists() {
        fs::rename(backup, target).with_context(|| {
            format!(
                "restore previous vault backup {} to {}",
                backup.display(),
                target.display()
            )
        })?;
    }
    if staging.exists() {
        fs::remove_dir_all(staging)
            .with_context(|| format!("remove restore staging vault {}", staging.display()))?;
    }
    sync_directory(target.parent().unwrap_or_else(|| Path::new(".")))
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn archive_plaintext(file: &Path, identity: Option<&Path>) -> Result<Vec<u8>> {
    let bytes = fs::read(file)?;
    if file.extension().and_then(|s| s.to_str()) == Some("age") || identity.is_some() {
        let identity = identity.context("encrypted archive requires --identity path")?;
        decrypt_bytes(&bytes, identity)
    } else {
        Ok(bytes)
    }
}

fn encrypt_bytes(bytes: &[u8], recipient: &str) -> Result<Vec<u8>> {
    let recipient: age::x25519::Recipient = recipient
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid age recipient: {err}"))?;
    let recipients: [&dyn age::Recipient; 1] = [&recipient];
    let encryptor = age::Encryptor::with_recipients(recipients.into_iter())?;
    let mut writer = encryptor.wrap_output(Vec::new())?;
    writer.write_all(bytes)?;
    writer.finish().map_err(Into::into)
}

fn decrypt_bytes(bytes: &[u8], identity_path: &Path) -> Result<Vec<u8>> {
    let identity_text = fs::read_to_string(identity_path)
        .with_context(|| format!("read identity {}", identity_path.display()))?;
    let identity_line = identity_text
        .lines()
        .find(|line| line.trim_start().starts_with("AGE-SECRET-KEY-"))
        .context("identity file does not contain AGE-SECRET-KEY")?;
    let identity: age::x25519::Identity = identity_line
        .trim()
        .parse()
        .map_err(|err| anyhow::anyhow!("invalid age identity: {err}"))?;
    let identities: [&dyn age::Identity; 1] = [&identity];
    let decryptor = age::Decryptor::new(Cursor::new(bytes))?;
    let mut reader = decryptor.decrypt(identities.into_iter())?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

fn reject_hidden_traversal(path: &Path) -> Result<()> {
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            bail!("unsafe archive path: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use age::secrecy::ExposeSecret;

    #[test]
    fn export_verify_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();
        verify_archive(&out, None).unwrap();
        restore_archive(&out, &tmp.path().join("Restored"), None).unwrap();
        assert!(
            tmp.path()
                .join("Restored/.brainmap/brainmap.sqlite")
                .exists()
        );
    }

    #[test]
    fn index_snapshot_and_restore_share_the_external_maintenance_lock() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();
        let archive = tmp.path().join("source.brainmap.tar.zst");
        export_archive(&root, &archive, ExportMode::Portable).unwrap();
        let _lock = util::acquire_vault_maintenance(&root).unwrap();

        let index_error = index::rebuild_cmd(Some(root.clone())).unwrap_err();
        assert!(index_error.to_string().contains("lock already held"));
        let snapshot_error = crate::snapshot::create(Some(root.clone())).unwrap_err();
        assert!(snapshot_error.to_string().contains("lock already held"));
        let restore_error = restore_cmd(RestoreArgs {
            file: archive,
            to: root,
            identity: None,
            fault_phase: None,
        })
        .unwrap_err();
        assert!(restore_error.to_string().contains("lock already held"));
    }

    #[test]
    fn guarded_restore_rejects_a_guard_for_another_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Source");
        let target = tmp.path().join("Target");
        let other = tmp.path().join("Other");
        vault::init_vault(Some(source.clone()), false, true).unwrap();
        let archive = tmp.path().join("source.brainmap.tar.zst");
        export_archive(&source, &archive, ExportMode::Portable).unwrap();
        let guard = util::acquire_vault_maintenance(&other).unwrap();

        let error = restore_archive_with_guard(&archive, &target, None, &guard).unwrap_err();
        assert!(error.to_string().contains("maintenance guard protects"));
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn restore_through_a_symlink_preserves_the_alias_and_replaces_the_real_vault() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Source");
        let real_target = tmp.path().join("Target");
        let alias_target = tmp.path().join("TargetAlias");
        vault::init_vault(Some(source.clone()), false, true).unwrap();
        fs::write(source.join("restored-marker.md"), "restored").unwrap();
        index::rebuild(&source).unwrap();
        let archive = tmp.path().join("source.brainmap.tar.zst");
        export_archive(&source, &archive, ExportMode::Portable).unwrap();

        vault::init_vault(Some(real_target.clone()), false, true).unwrap();
        fs::write(real_target.join("original-marker.md"), "original").unwrap();
        index::rebuild(&real_target).unwrap();
        symlink(&real_target, &alias_target).unwrap();

        restore_archive(&archive, &alias_target, None).unwrap();

        assert!(
            fs::symlink_metadata(&alias_target)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(real_target.join("restored-marker.md").exists());
        assert!(!real_target.join("original-marker.md").exists());
        assert!(index::status(&real_target).unwrap().valid);
    }

    #[test]
    fn encrypted_export_verify_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();
        let identity = age::x25519::Identity::generate();
        let recipient = identity.to_public().to_string();
        let identity_path = tmp.path().join("identity.txt");
        fs::write(&identity_path, identity.to_string().expose_secret()).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst.age");
        export_encrypted_archive(&root, &out, ExportMode::Portable, &recipient).unwrap();
        verify_archive(&out, Some(&identity_path)).unwrap();
        restore_archive(
            &out,
            &tmp.path().join("EncryptedRestored"),
            Some(&identity_path),
        )
        .unwrap();
        assert!(
            tmp.path()
                .join("EncryptedRestored/.brainmap/brainmap.sqlite")
                .exists()
        );
    }

    #[test]
    fn verify_rejects_trailing_archive_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();
        let mut bytes = fs::read(&out).unwrap();
        bytes.push(b'x');
        fs::write(&out, bytes).unwrap();

        let err = verify_archive(&out, None).unwrap_err();
        assert!(err.to_string().contains("trailing data"));
    }

    #[test]
    fn verify_rejects_unmanifested_archive_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();

        let mut entries = read_archive(&out, None).unwrap();
        entries.push(("unmanifested.md".into(), b"unverified".to_vec()));
        fs::write(&out, encode_entries(&entries)).unwrap();

        let err = verify_archive(&out, None).unwrap_err();
        assert!(err.to_string().contains("unmanifested"));
    }

    #[test]
    fn verify_rejects_duplicate_archive_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();

        let mut entries = read_archive(&out, None).unwrap();
        let duplicate = entries
            .iter()
            .find(|(path, _)| path != "manifest.json")
            .cloned()
            .unwrap();
        entries.push(duplicate);
        fs::write(&out, encode_entries(&entries)).unwrap();

        let err = verify_archive(&out, None).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn verify_rejects_lexical_alias_archive_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();

        let mut entries = read_archive(&out, None).unwrap();
        let mut alias = entries
            .iter()
            .find(|(path, _)| path != "manifest.json")
            .cloned()
            .unwrap();
        alias.0 = format!("./{}", alias.0);
        entries.push(alias);
        fs::write(&out, encode_entries(&entries)).unwrap();

        let err = verify_archive(&out, None).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn verify_rejects_case_folded_portable_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();

        let mut entries = read_archive(&out, None).unwrap();
        let mut alias = entries
            .iter()
            .find(|(path, _)| path != "manifest.json")
            .cloned()
            .unwrap();
        alias.0 = alias.0.to_uppercase();
        entries.push(alias);
        fs::write(&out, encode_entries(&entries)).unwrap();

        let err = verify_archive(&out, None).unwrap_err();
        assert!(err.to_string().contains("portable archive path collision"));
    }

    #[test]
    fn export_rejects_case_colliding_source_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        fs::write(root.join("CaseCollision.md"), b"upper").unwrap();
        fs::write(root.join("casecollision.md"), b"lower").unwrap();

        // The default macOS and Windows filesystems collapse these names into
        // one file, so there is no source collision for the exporter to see.
        if fs::read(root.join("CaseCollision.md")).unwrap() != b"upper" {
            return;
        }

        let err = archive_bytes(&root, ExportMode::Portable, false).unwrap_err();

        assert!(err.to_string().contains("portable archive path collision"));
    }

    #[test]
    fn portable_and_full_exports_reject_secret_files_while_share_safe_skips_them() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        fs::write(
            root.join("secret-note.md"),
            "---\nid: secret\ntype: reference\nsensitivity: secret\n---\napi_key=abcdef1234567890\n",
        )
        .unwrap();

        for mode in [ExportMode::Portable, ExportMode::Full] {
            let error = archive_bytes(&root, mode, false).unwrap_err();
            assert!(error.to_string().contains("refusing to export secret"));
        }
        let share_safe = archive_bytes(&root, ExportMode::ShareSafe, false).unwrap();
        let entries = read_archive_bytes(&share_safe).unwrap();
        assert!(entries.iter().all(|(path, _)| path != "secret-note.md"));
    }

    #[test]
    fn exports_reject_secret_like_non_utf8_content() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let mut content = vec![0xff, b'\n'];
        content.extend_from_slice(b"api_key=abcdef1234567890\n");
        fs::write(root.join("binary-note.bin"), content).unwrap();

        for mode in [ExportMode::Portable, ExportMode::Full] {
            let error = archive_bytes(&root, mode, false).unwrap_err();
            assert!(error.to_string().contains("refusing to export secret"));
        }
        let share_safe = archive_bytes(&root, ExportMode::ShareSafe, false).unwrap();
        let entries = read_archive_bytes(&share_safe).unwrap();
        assert!(entries.iter().all(|(path, _)| path != "binary-note.bin"));
    }

    #[test]
    fn exports_reject_secret_like_archive_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let secret_path = "api_key=abcdef1234567890.md";
        fs::write(root.join(secret_path), b"benign body").unwrap();

        for mode in [ExportMode::Portable, ExportMode::Full] {
            let error = archive_bytes(&root, mode, false).unwrap_err();
            assert!(error.to_string().contains("secret-like archive path"));
        }
        let share_safe = archive_bytes(&root, ExportMode::ShareSafe, false).unwrap();
        let entries = read_archive_bytes(&share_safe).unwrap();
        assert!(entries.iter().all(|(path, _)| path != secret_path));
    }

    #[test]
    fn verify_rejects_unknown_decision_engine_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("demo.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();
        let mut entries = read_archive(&out, None).unwrap();
        let manifest = entries
            .iter_mut()
            .find(|(path, _)| path == "manifest.json")
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&manifest.1).unwrap();
        value["schemaVersion"] = serde_json::json!("decision-engine-v999");
        manifest.1 = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&out, encode_entries(&entries)).unwrap();

        let error = verify_archive(&out, None).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("unsupported decision engine schema")
        );
    }

    #[test]
    fn verify_rejects_secret_bearing_archive_content_before_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("malicious.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();
        let mut entries = read_archive(&out, None).unwrap();
        let secret_path = "notes/injected-secret.md";
        let secret = b"---\nid: injected-secret\ntype: reference\nsensitivity: secret\n---\napi_key=abcdef1234567890\n".to_vec();
        let manifest = entries
            .iter_mut()
            .find(|(path, _)| path == "manifest.json")
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&manifest.1).unwrap();
        value["files"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "path": secret_path,
                "sha256": util::sha256_hex(&secret)
            }));
        manifest.1 = serde_json::to_vec_pretty(&value).unwrap();
        entries.push((secret_path.into(), secret));
        fs::write(&out, encode_entries(&entries)).unwrap();

        let error = verify_archive(&out, None).unwrap_err();
        assert!(error.to_string().contains("secret-like archive content"));
    }

    #[test]
    fn verify_rejects_secret_like_manifest_and_archive_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("malicious-path.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();
        let mut entries = read_archive(&out, None).unwrap();
        let original_path = entries
            .iter()
            .find(|(path, _)| path != "manifest.json")
            .map(|(path, _)| path.clone())
            .unwrap();
        let secret_path = "notes/api_key=abcdef1234567890.md";
        let manifest = entries
            .iter_mut()
            .find(|(path, _)| path == "manifest.json")
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&manifest.1).unwrap();
        let manifest_file = value["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|file| file["path"] == original_path)
            .unwrap();
        manifest_file["path"] = serde_json::json!(secret_path);
        manifest.1 = serde_json::to_vec_pretty(&value).unwrap();
        entries
            .iter_mut()
            .find(|(path, _)| path == &original_path)
            .unwrap()
            .0 = secret_path.into();
        fs::write(&out, encode_entries(&entries)).unwrap();

        let error = verify_archive(&out, None).unwrap_err();
        assert!(error.to_string().contains("secret-like archive path"));
    }

    #[test]
    fn verify_rejects_secret_like_manifest_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let out = tmp.path().join("malicious-manifest.brainmap.tar.zst");
        export_archive(&root, &out, ExportMode::Portable).unwrap();
        let mut entries = read_archive(&out, None).unwrap();
        let manifest = entries
            .iter_mut()
            .find(|(path, _)| path == "manifest.json")
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&manifest.1).unwrap();
        value["brainmapVersion"] = serde_json::json!("api_key=abcdef1234567890");
        manifest.1 = serde_json::to_vec_pretty(&value).unwrap();
        fs::write(&out, encode_entries(&entries)).unwrap();

        let error = verify_archive(&out, None).unwrap_err();
        assert!(error.to_string().contains("secret-like archive manifest"));
    }

    #[test]
    fn portable_exports_exclude_snapshot_archives() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let backup = root.join("99-meta/backups/old.brainmap.tar.zst");
        fs::write(&backup, b"old snapshot").unwrap();

        let bytes = archive_bytes(&root, ExportMode::Portable, false).unwrap();
        let entries = read_archive_bytes(&bytes).unwrap();

        assert!(
            entries
                .iter()
                .all(|(path, _)| !path.starts_with("99-meta/backups/"))
        );
    }

    #[test]
    fn exports_declare_current_compiled_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();

        let bytes = archive_bytes(&root, ExportMode::Full, false).unwrap();
        let entries = read_archive_bytes(&bytes).unwrap();
        let manifest: Manifest = serde_json::from_slice(
            &entries
                .iter()
                .find(|(path, _)| path == "manifest.json")
                .unwrap()
                .1,
        )
        .unwrap();

        assert_eq!(manifest.schema_version, index::COMPILED_SCHEMA_VERSION);
    }

    #[test]
    fn restore_validates_in_staging_before_replacing_the_target() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Source");
        vault::init_vault(Some(source.clone()), false, true).unwrap();
        fs::write(source.join("restored-marker.md"), "restored").unwrap();
        index::rebuild(&source).unwrap();
        let archive = tmp.path().join("source.brainmap.tar.zst");
        export_archive(&source, &archive, ExportMode::Portable).unwrap();

        let target = tmp.path().join("Target");
        vault::init_vault(Some(target.clone()), false, true).unwrap();
        fs::write(target.join("original-marker.md"), "original").unwrap();

        let error =
            restore_archive_with_fault(&archive, &target, None, Some(RestorePhase::GateChecked))
                .unwrap_err();

        assert!(error.to_string().contains("GateChecked"));
        assert_eq!(
            fs::read_to_string(target.join("original-marker.md")).unwrap(),
            "original"
        );
        assert!(!target.join("restored-marker.md").exists());
    }

    #[test]
    fn every_injected_restore_failure_leaves_a_complete_old_or_new_vault() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Source");
        vault::init_vault(Some(source.clone()), false, true).unwrap();
        fs::write(source.join("restored-marker.md"), "restored").unwrap();
        index::rebuild(&source).unwrap();
        let archive = tmp.path().join("source.brainmap.tar.zst");
        export_archive(&source, &archive, ExportMode::Portable).unwrap();

        for phase in [
            RestorePhase::Verified,
            RestorePhase::StagingCreated,
            RestorePhase::FilesWritten,
            RestorePhase::IndexRebuilt,
            RestorePhase::LinksChecked,
            RestorePhase::GateChecked,
            RestorePhase::ExistingBackedUp,
            RestorePhase::StagingActivated,
        ] {
            let target = tmp.path().join(format!("Target-{phase:?}"));
            vault::init_vault(Some(target.clone()), false, true).unwrap();
            fs::write(target.join("original-marker.md"), "original").unwrap();
            index::rebuild(&target).unwrap();

            let error =
                restore_archive_with_fault(&archive, &target, None, Some(phase)).unwrap_err();
            assert!(error.to_string().contains(&format!("{phase:?}")));

            let old_complete = target.join("original-marker.md").exists()
                && !target.join("restored-marker.md").exists();
            let new_complete = !target.join("original-marker.md").exists()
                && target.join("restored-marker.md").exists()
                && index::status(&target).unwrap().valid;
            assert!(
                old_complete || new_complete,
                "incomplete state after {phase:?}"
            );
        }
    }

    fn encode_entries(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let encoder = zstd::Encoder::new(Vec::new(), 3).unwrap();
        let mut tar = Builder::new(encoder);
        for (path, bytes) in entries {
            append_bytes(&mut tar, path, bytes).unwrap();
        }
        let encoder = tar.into_inner().unwrap();
        encoder.finish().unwrap()
    }
}
