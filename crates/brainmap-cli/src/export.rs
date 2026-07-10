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
struct ManifestFile {
    path: String,
    sha256: String,
}

pub fn export_cmd(args: ExportArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
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
    util::ensure_parent(out)?;
    fs::write(out, archive_bytes(root, mode, false)?)?;
    Ok(())
}

fn export_encrypted_archive(
    root: &Path,
    out: &Path,
    mode: ExportMode,
    recipient: &str,
) -> Result<()> {
    util::ensure_parent(out)?;
    let archive = archive_bytes(root, mode, true)?;
    fs::write(out, encrypt_bytes(&archive, recipient)?)?;
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
        let collision_key = util::portable_archive_collision_key(&rel)?;
        if !portable_keys.insert(collision_key) {
            bail!("portable archive path collision: {rel}");
        }
        let mut bytes = fs::read(&path)?;
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
        schema_version: "decision-engine-v2".into(),
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

fn should_skip(rel: &str, mode: &ExportMode) -> bool {
    if rel.starts_with("99-meta/backups/") || rel == ".brainmap/last-snapshot" {
        return true;
    }
    if rel.contains(".brainmap/locks")
        || rel.contains(".brainmap/web-cache")
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
    let mut manifest_paths = HashSet::new();
    let mut manifest_collision_keys = HashSet::new();
    for file in &manifest.files {
        let normalized_path = util::normalize_archive_path(Path::new(&file.path))?;
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
        let got = util::sha256_hex(bytes);
        if got != file.sha256 {
            bail!("checksum mismatch for {}", file.path);
        }
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
    restore_archive(&args.file, &args.to, args.identity.as_deref())
}

fn restore_archive(file: &Path, to: &Path, identity: Option<&Path>) -> Result<()> {
    let verified = verify_archive(file, identity)?;
    let target = util::expand_tilde(to);
    if target.exists() && fs::read_dir(&target)?.next().is_some() {
        let backup = target.with_extension(format!("backup-{}", chrono::Utc::now().timestamp()));
        fs::rename(&target, &backup)?;
        println!("backed up existing target to {}", backup.display());
    }
    fs::create_dir_all(&target)?;
    for (path, bytes) in verified.entries {
        if path == "manifest.json" {
            continue;
        }
        let rel = PathBuf::from(&path);
        reject_hidden_traversal(&rel)?;
        let out = target.join(rel);
        util::write_atomic(&out, &bytes)?;
    }
    index::rebuild(&target)?;
    vault::link_check(&target)?;
    let _ = gate::evaluate(
        &target,
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
    println!("restored {}", target.display());
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

        let err = archive_bytes(&root, ExportMode::Portable, false).unwrap_err();

        assert!(err.to_string().contains("portable archive path collision"));
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

        assert_eq!(manifest.schema_version, "decision-engine-v2");
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
