use crate::{gate, index, learning, qualification, skill, util, vault};
use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Args)]
pub struct InstallHarnessArgs {
    #[arg(long)]
    pub target: String,
    #[arg(long, conflicts_with = "project")]
    pub global: bool,
    #[arg(long)]
    pub project: Option<PathBuf>,
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub uninstall: bool,
}

#[derive(Args)]
pub struct InstallCandidateArgs {
    #[arg(long)]
    pub qualification_bundle: PathBuf,
    #[arg(long, value_name = "DIR")]
    pub to: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
struct CandidateInstallSource {
    brainmap: PathBuf,
    brainmapd: PathBuf,
    candidate_commit: String,
    brainmap_sha256: String,
    brainmapd_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CandidateInstallChange {
    Create,
    Update,
    Unchanged,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateInstallPlan {
    schema_version: &'static str,
    dry_run: bool,
    qualification_verified: bool,
    candidate_commit: String,
    brainmap_sha256: String,
    brainmapd_sha256: String,
    destination: PathBuf,
    path_active: bool,
    brainmap_change: CandidateInstallChange,
    brainmapd_change: CandidateInstallChange,
    backup_required: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateInstallResult {
    schema_version: &'static str,
    installed: bool,
    idempotent: bool,
    candidate_commit: String,
    brainmap_path: PathBuf,
    brainmapd_path: PathBuf,
    brainmap_sha256: String,
    brainmapd_sha256: String,
    backups: Vec<PathBuf>,
    path_verified: bool,
    qualification_verified: bool,
}

pub fn install_candidate(args: InstallCandidateArgs) -> Result<()> {
    let bundle = fs::canonicalize(&args.qualification_bundle).with_context(|| {
        format!(
            "resolve qualification bundle {}",
            args.qualification_bundle.display()
        )
    })?;
    let verified = qualification::verify_bundle(&bundle)?;
    let running = qualification::verify_running_qualification(&verified)?;
    let brainmap = fs::canonicalize(std::env::current_exe()?)?;
    let brainmapd = brainmap.with_file_name(if cfg!(windows) {
        "brainmapd.exe"
    } else {
        "brainmapd"
    });
    let source = CandidateInstallSource {
        brainmap,
        brainmapd,
        candidate_commit: verified.candidate.commit.clone(),
        brainmap_sha256: running.brainmap_sha256,
        brainmapd_sha256: running.brainmapd_sha256,
    };
    let destination = canonical_future_directory(
        &args
            .to
            .unwrap_or_else(|| util::home_dir().join(".local/bin")),
    )?;
    let path_entries = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    let plan = plan_candidate_install(&source, &destination, args.dry_run, &path_entries)?;
    if args.dry_run {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }

    let expected_build_info = crate::build_info::build_info_json()?;
    let result = install_candidate_files(
        &source,
        &destination,
        &path_entries,
        |installed_brainmap, installed_brainmapd| {
            verify_installed_candidate(
                installed_brainmap,
                installed_brainmapd,
                &bundle,
                &expected_build_info,
            )
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn plan_candidate_install(
    source: &CandidateInstallSource,
    destination: &Path,
    dry_run: bool,
    path_entries: &[PathBuf],
) -> Result<CandidateInstallPlan> {
    validate_candidate_source(source)?;
    ensure!(
        destination_on_path(destination, path_entries),
        "candidate install destination is not active on PATH: {}",
        destination.display()
    );
    ensure!(
        destination_will_resolve(destination, path_entries, "brainmap")?
            && destination_will_resolve(destination, path_entries, "brainmapd")?,
        "candidate install destination is shadowed by an earlier executable on PATH"
    );
    let brainmap = destination.join(if cfg!(windows) {
        "brainmap.exe"
    } else {
        "brainmap"
    });
    let brainmapd = destination.join(if cfg!(windows) {
        "brainmapd.exe"
    } else {
        "brainmapd"
    });
    let brainmap_change = candidate_file_change(&brainmap, &source.brainmap_sha256)?;
    let brainmapd_change = candidate_file_change(&brainmapd, &source.brainmapd_sha256)?;
    Ok(CandidateInstallPlan {
        schema_version: "brainmap-candidate-install-plan-v1",
        dry_run,
        qualification_verified: true,
        candidate_commit: source.candidate_commit.clone(),
        brainmap_sha256: source.brainmap_sha256.clone(),
        brainmapd_sha256: source.brainmapd_sha256.clone(),
        destination: destination.to_path_buf(),
        path_active: true,
        brainmap_change,
        brainmapd_change,
        backup_required: matches!(brainmap_change, CandidateInstallChange::Update)
            || matches!(brainmapd_change, CandidateInstallChange::Update),
    })
}

fn install_candidate_files(
    source: &CandidateInstallSource,
    destination: &Path,
    path_entries: &[PathBuf],
    verify_installed: impl FnOnce(&Path, &Path) -> Result<()>,
) -> Result<CandidateInstallResult> {
    let _preflight = plan_candidate_install(source, destination, false, path_entries)?;
    fs::create_dir_all(destination).with_context(|| {
        format!(
            "create candidate install directory {}",
            destination.display()
        )
    })?;
    let metadata = fs::symlink_metadata(destination)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "candidate install destination is not a symlink-free directory"
    );
    let lock = destination.join(".brainmap-candidate-install.lock");
    fs::create_dir(&lock).with_context(|| {
        format!(
            "candidate install lock already exists or cannot be created: {}",
            lock.display()
        )
    })?;
    util::sync_directory(destination)?;

    let brainmap = destination.join(if cfg!(windows) {
        "brainmap.exe"
    } else {
        "brainmap"
    });
    let brainmapd = destination.join(if cfg!(windows) {
        "brainmapd.exe"
    } else {
        "brainmapd"
    });
    let result = (|| -> Result<CandidateInstallResult> {
        let plan = plan_candidate_install(source, destination, false, path_entries)?;
        let original_brainmap = candidate_file_snapshot(&brainmap)?;
        let original_brainmapd = candidate_file_snapshot(&brainmapd)?;
        validate_candidate_snapshot(
            &original_brainmap,
            plan.brainmap_change,
            &source.brainmap_sha256,
        )?;
        validate_candidate_snapshot(
            &original_brainmapd,
            plan.brainmapd_change,
            &source.brainmapd_sha256,
        )?;
        let mut backups = Vec::new();
        if matches!(plan.brainmap_change, CandidateInstallChange::Update) {
            backups.push(backup_candidate(
                &brainmap,
                original_brainmap
                    .as_ref()
                    .context("updated brainmap has no original snapshot")?,
            )?);
        }
        if matches!(plan.brainmapd_change, CandidateInstallChange::Update) {
            backups.push(backup_candidate(
                &brainmapd,
                original_brainmapd
                    .as_ref()
                    .context("updated brainmapd has no original snapshot")?,
            )?);
        }

        let install_result = (|| -> Result<()> {
            ensure!(
                candidate_snapshot_bytes(&candidate_file_snapshot(&brainmap)?)
                    == candidate_snapshot_bytes(&original_brainmap)
                    && candidate_snapshot_bytes(&candidate_file_snapshot(&brainmapd)?)
                        == candidate_snapshot_bytes(&original_brainmapd),
                "candidate install target changed after backup"
            );
            if !matches!(plan.brainmapd_change, CandidateInstallChange::Unchanged) {
                write_executable(&source.brainmapd, &brainmapd)?;
            }
            if !matches!(plan.brainmap_change, CandidateInstallChange::Unchanged) {
                write_executable(&source.brainmap, &brainmap)?;
            }
            ensure!(
                file_sha256(&brainmap)? == source.brainmap_sha256
                    && file_sha256(&brainmapd)? == source.brainmapd_sha256,
                "installed candidate hashes do not match the qualified pair"
            );
            ensure!(
                resolve_path_command("brainmap", path_entries)?.as_deref()
                    == Some(brainmap.as_path())
                    && resolve_path_command("brainmapd", path_entries)?.as_deref()
                        == Some(brainmapd.as_path()),
                "installed candidate pair is shadowed on PATH"
            );
            verify_installed(&brainmap, &brainmapd)
        })();
        if let Err(error) = install_result {
            restore_candidate_file(&brainmap, original_brainmap.as_ref())?;
            restore_candidate_file(&brainmapd, original_brainmapd.as_ref())?;
            return Err(error).context("candidate installation failed and was rolled back");
        }

        Ok(CandidateInstallResult {
            schema_version: "brainmap-candidate-install-result-v1",
            installed: true,
            idempotent: matches!(plan.brainmap_change, CandidateInstallChange::Unchanged)
                && matches!(plan.brainmapd_change, CandidateInstallChange::Unchanged),
            candidate_commit: source.candidate_commit.clone(),
            brainmap_path: brainmap,
            brainmapd_path: brainmapd,
            brainmap_sha256: source.brainmap_sha256.clone(),
            brainmapd_sha256: source.brainmapd_sha256.clone(),
            backups,
            path_verified: true,
            qualification_verified: true,
        })
    })();
    let unlock = fs::remove_dir(&lock)
        .with_context(|| format!("remove candidate install lock {}", lock.display()))
        .and_then(|()| util::sync_directory(destination));
    match (result, unlock) {
        (Ok(result), Ok(())) => Ok(result),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(unlock_error)) => Err(error).context(format!(
            "candidate install cleanup also failed: {unlock_error:#}"
        )),
    }
}

#[derive(Clone)]
struct CandidateFileSnapshot {
    bytes: Vec<u8>,
    permissions: fs::Permissions,
}

fn canonical_future_directory(path: &Path) -> Result<PathBuf> {
    ensure!(
        path.is_absolute(),
        "candidate install destination must be absolute"
    );
    ensure!(
        !path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir)),
        "candidate install destination must be canonical"
    );
    let mut cursor = path;
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "candidate install destination traverses a symlink or non-directory"
                );
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor
                    .file_name()
                    .context("candidate install destination has no existing ancestor")?;
                missing.push(name.to_os_string());
                cursor = cursor
                    .parent()
                    .context("candidate install destination has no existing ancestor")?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect install path {}", cursor.display()));
            }
        }
    }
    let mut canonical = fs::canonicalize(cursor)?;
    for component in missing.iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

fn validate_candidate_source(source: &CandidateInstallSource) -> Result<()> {
    for (label, path, expected_sha) in [
        ("brainmap", &source.brainmap, &source.brainmap_sha256),
        ("brainmapd", &source.brainmapd, &source.brainmapd_sha256),
    ] {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("inspect candidate {label} {}", path.display()))?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "candidate {label} is not a symlink-free regular file"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o111 != 0,
                "candidate {label} is not executable"
            );
        }
        ensure!(
            file_sha256(path)? == *expected_sha,
            "candidate {label} hash changed before installation"
        );
    }
    Ok(())
}

fn candidate_file_change(path: &Path, expected_sha: &str) -> Result<CandidateInstallChange> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CandidateInstallChange::Create);
        }
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "candidate install target is not a symlink-free regular file: {}",
        path.display()
    );
    if file_sha256(path)? == expected_sha {
        Ok(CandidateInstallChange::Unchanged)
    } else {
        Ok(CandidateInstallChange::Update)
    }
}

fn candidate_file_snapshot(path: &Path) -> Result<Option<CandidateFileSnapshot>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "candidate install target is not a symlink-free regular file: {}",
                path.display()
            );
            Ok(Some(CandidateFileSnapshot {
                bytes: fs::read(path)?,
                permissions: metadata.permissions(),
            }))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("inspect {}", path.display())),
    }
}

fn validate_candidate_snapshot(
    snapshot: &Option<CandidateFileSnapshot>,
    change: CandidateInstallChange,
    expected_sha: &str,
) -> Result<()> {
    let actual_sha = snapshot
        .as_ref()
        .map(|snapshot| util::sha256_hex(&snapshot.bytes));
    let valid = match change {
        CandidateInstallChange::Create => snapshot.is_none(),
        CandidateInstallChange::Update => {
            actual_sha.as_deref().is_some_and(|sha| sha != expected_sha)
        }
        CandidateInstallChange::Unchanged => actual_sha.as_deref() == Some(expected_sha),
    };
    ensure!(valid, "candidate install target changed during preflight");
    Ok(())
}

fn candidate_snapshot_bytes(snapshot: &Option<CandidateFileSnapshot>) -> Option<&[u8]> {
    snapshot.as_ref().map(|snapshot| snapshot.bytes.as_slice())
}

fn write_executable(source: &Path, destination: &Path) -> Result<()> {
    let source_metadata = fs::metadata(source)?;
    util::write_atomic(destination, &fs::read(source)?)?;
    fs::set_permissions(destination, source_metadata.permissions())?;
    util::sync_file(destination)
}

fn backup_candidate(path: &Path, snapshot: &CandidateFileSnapshot) -> Result<PathBuf> {
    let backup = path.with_extension(format!("bak-{}", chrono::Utc::now().timestamp_micros()));
    ensure!(!backup.exists(), "candidate backup path already exists");
    util::write_atomic(&backup, &snapshot.bytes)?;
    fs::set_permissions(&backup, snapshot.permissions.clone())?;
    util::sync_file(&backup)?;
    util::sync_directory(backup.parent().unwrap_or_else(|| Path::new(".")))?;
    Ok(backup)
}

fn restore_candidate_file(path: &Path, snapshot: Option<&CandidateFileSnapshot>) -> Result<()> {
    if let Some(snapshot) = snapshot {
        util::write_atomic(path, &snapshot.bytes)?;
        fs::set_permissions(path, snapshot.permissions.clone())?;
        util::sync_file(path)
    } else {
        util::remove_file_and_sync(path)
    }
}

fn file_sha256(path: &Path) -> Result<String> {
    Ok(util::sha256_hex(
        &fs::read(path).with_context(|| format!("read {}", path.display()))?,
    ))
}

fn destination_on_path(destination: &Path, path_entries: &[PathBuf]) -> bool {
    path_entries
        .iter()
        .any(|entry| canonical_path_entry(entry).is_ok_and(|canonical| canonical == destination))
}

fn canonical_path_entry(path: &Path) -> Result<PathBuf> {
    let path = if path.as_os_str().is_empty() {
        std::env::current_dir().context("resolve empty PATH entry")?
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("resolve relative PATH entry")?
            .join(path)
    };
    match fs::canonicalize(&path) {
        Ok(canonical) => {
            ensure!(
                fs::metadata(&canonical)?.is_dir(),
                "PATH entry is not a directory"
            );
            Ok(canonical)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            canonical_future_directory(&path)
        }
        Err(error) => Err(error).with_context(|| format!("resolve PATH entry {}", path.display())),
    }
}

fn destination_will_resolve(
    destination: &Path,
    path_entries: &[PathBuf],
    command: &str,
) -> Result<bool> {
    let executable = if cfg!(windows) {
        format!("{command}.exe")
    } else {
        command.to_string()
    };
    for entry in path_entries {
        let directory = match canonical_path_entry(entry) {
            Ok(directory) => directory,
            Err(_) => continue,
        };
        if directory == destination {
            return Ok(true);
        }
        let candidate = directory.join(&executable);
        if command_path_is_executable(&candidate)? {
            return Ok(false);
        }
    }
    Ok(false)
}

fn resolve_path_command(command: &str, path_entries: &[PathBuf]) -> Result<Option<PathBuf>> {
    let executable = if cfg!(windows) {
        format!("{command}.exe")
    } else {
        command.to_string()
    };
    for entry in path_entries {
        let directory = match canonical_path_entry(entry) {
            Ok(directory) => directory,
            Err(_) => continue,
        };
        let candidate = directory.join(&executable);
        if command_path_is_executable(&candidate)? {
            return fs::canonicalize(&candidate)
                .map(Some)
                .with_context(|| format!("resolve PATH executable {}", candidate.display()));
        }
    }
    Ok(None)
}

fn command_path_is_executable(path: &Path) -> Result<bool> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    };
    if !metadata.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn verify_installed_candidate(
    brainmap: &Path,
    brainmapd: &Path,
    bundle: &Path,
    expected_build_info: &str,
) -> Result<()> {
    for (label, executable) in [("brainmap", brainmap), ("brainmapd", brainmapd)] {
        let output = Command::new(executable)
            .arg("build-info")
            .output()
            .with_context(|| format!("run installed {label} build-info"))?;
        ensure!(
            output.status.success(),
            "installed {label} build-info failed"
        );
        let actual: Value = serde_json::from_slice(&output.stdout)
            .with_context(|| format!("parse installed {label} build-info"))?;
        let expected: Value = serde_json::from_str(expected_build_info)?;
        ensure!(
            actual == expected,
            "installed {label} build provenance changed"
        );
    }
    let verification = Command::new(brainmap)
        .args(["qualification", "verify", "--bundle"])
        .arg(bundle)
        .output()
        .context("run installed candidate qualification verification")?;
    ensure!(
        verification.status.success(),
        "installed candidate rejected qualification bundle: {}",
        String::from_utf8_lossy(&verification.stderr).trim()
    );
    Ok(())
}

pub fn install_harness(args: InstallHarnessArgs) -> Result<()> {
    let plan = plan(&args)?;
    let changes = plan
        .iter()
        .map(|item| item.preflight(args.uninstall))
        .collect::<Result<Vec<_>>>()?;
    if args.dry_run {
        println!("install harness dry-run target={}", args.target);
        for (item, change) in plan.iter().zip(&changes) {
            let backup = matches!(change, PlanChange::Update | PlanChange::Remove)
                .then_some("; backup required")
                .unwrap_or_default();
            println!(
                "would {} {} ({}{backup})",
                change.as_str(),
                item.path.display(),
                item.enforcement
            );
        }
        return Ok(());
    }
    for item in plan {
        if args.uninstall {
            item.uninstall()?;
        } else {
            item.install()?;
        }
    }
    Ok(())
}

pub fn integration_doctor(args: crate::cli::IntegrationDoctorArgs) -> Result<()> {
    let supported = matches!(
        args.target.as_str(),
        "codex" | "claude-code" | "opencode" | "copilot" | "generic-stdio"
    );
    let install_args = InstallHarnessArgs {
        target: args.target.clone(),
        global: args.global,
        project: args.project.clone(),
        vault: args.vault.clone(),
        dry_run: true,
        uninstall: false,
    };
    let planned = plan(&install_args)?;
    let root = configured_vault_path(args.vault.as_deref())?;
    let running_executable = running_brainmap_executable()?;
    let host_probe_required = args.target == "codex";
    let project_trust_required = host_probe_required && !args.global;
    let project_trust_result = if project_trust_required {
        codex_project_trusted(
            args.project
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new(".")),
        )
        .map(Some)
    } else {
        Ok(None)
    };
    let (project_trusted, project_trust_configuration_valid, project_trust_error) =
        match project_trust_result {
            Ok(Some(trusted)) => (trusted, true, None),
            Ok(None) => (true, true, None),
            Err(error) => (false, false, Some(error.to_string())),
        };
    let installed = supported && planned.iter().all(|item| item.path.exists());
    let configuration_valid = planned.iter().all(|item| {
        if !item.path.exists() {
            return true;
        }
        match &item.action {
            PlanAction::JsonHooks(bindings) => fs::read(&item.path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|value| hook_bindings_installed(&value, bindings)),
            PlanAction::JsonInstruction(expected) => fs::read(&item.path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| {
                    value
                        .get("instructions")
                        .and_then(Value::as_str)
                        .map(|instructions| instructions.matches(expected).count() == 1)
                })
                .unwrap_or(false),
            PlanAction::ManagedToml(_) => fs::read_to_string(&item.path)
                .map(|text| codex_mcp_config_status(&text, &root, &running_executable).valid)
                .unwrap_or(false),
            PlanAction::OwnedText(expected) => {
                fs::read_to_string(&item.path).is_ok_and(|actual| actual == *expected)
            }
            PlanAction::ManagedText(expected) => fs::read_to_string(&item.path)
                .is_ok_and(|actual| actual.matches(expected).count() == 1),
        }
    });
    let contract = planned
        .iter()
        .filter_map(|item| fs::read_to_string(&item.path).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let learning_probe = probe_learning_lifecycle().unwrap_or_default();
    let recording_supported =
        contract.contains("record-decision") && learning_probe.recording_works;
    let feedback_supported = contract.contains("learn-feedback") && learning_probe.feedback_works;
    let activation_requires_approval = contract.contains("apply --pending --yes")
        && learning_probe.preview_works
        && learning_probe.approved_apply_changes_decision;
    let executable = running_executable.exists();
    let mcp_vault_configured = args.target != "codex"
        || planned
            .iter()
            .find(|item| matches!(&item.action, PlanAction::ManagedToml(_)))
            .and_then(|item| fs::read_to_string(&item.path).ok())
            .is_some_and(|text| {
                codex_mcp_config_status(&text, &root, &running_executable).vault_matches
            });
    let vault_exists = root.exists();
    let index_status = index::status(&root).ok();
    let index_valid = index_status.as_ref().is_some_and(|status| status.valid);
    let gate_reachable = index_valid
        && gate::evaluate(
            &root,
            gate::GateInput {
                intent: "integration-doctor".into(),
                situation: "Choose v1 storage".into(),
                options: vec!["Markdown+JSONL".into(), "External Vector DB".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .is_ok();
    let enforcement = planned
        .iter()
        .map(|item| item.enforcement)
        .collect::<Vec<_>>();
    let healthy = supported
        && installed
        && configuration_valid
        && executable
        && vault_exists
        && index_valid
        && gate_reachable
        && recording_supported
        && feedback_supported
        && activation_requires_approval
        && project_trust_configuration_valid
        && project_trusted;
    let healthy = healthy && mcp_vault_configured;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "target": args.target,
            "supported": supported,
            "installed": installed,
            "configurationValid": configuration_valid,
            "executableAvailable": executable,
            "vaultExists": vault_exists,
            "indexValid": index_valid,
            "gateReachable": gate_reachable,
            "recordingSupported": recording_supported,
            "feedbackSupported": feedback_supported,
            "activationRequiresApproval": activation_requires_approval,
            "mcpVaultConfigured": mcp_vault_configured,
            "projectTrustRequired": project_trust_required,
            "projectTrusted": project_trusted,
            "projectTrustConfigurationValid": project_trust_configuration_valid,
            "projectTrustError": project_trust_error,
            "healthScope": "local-adapter-files-and-contract",
            "hostHookTrustVerified": false,
            "hostProbeRequired": host_probe_required,
            "enforcement": enforcement,
            "healthy": healthy,
        }))?
    );
    if !healthy {
        let mut issues = Vec::new();
        if !supported {
            issues.push("unsupported target");
        }
        if !installed {
            issues.push("adapter files missing");
        }
        if !configuration_valid {
            issues.push("invalid host configuration");
        }
        if !executable {
            issues.push("brainmap executable unavailable");
        }
        if !vault_exists {
            issues.push("vault missing");
        } else if !index_valid {
            issues.push("compiled index missing or invalid");
        }
        if !gate_reachable {
            issues.push("decision gate unhealthy");
        }
        if !recording_supported {
            issues.push("recording contract missing");
        }
        if !feedback_supported {
            issues.push("feedback contract missing");
        }
        if !activation_requires_approval {
            issues.push("explicit activation approval missing");
        }
        if !mcp_vault_configured {
            issues.push("Codex MCP vault path does not match the requested vault");
        }
        if !project_trust_configuration_valid {
            issues.push("Codex trust configuration is unreadable or invalid");
        } else if !project_trusted {
            issues.push(
                "Codex project is not trusted; trust it in CODEX_HOME/config.toml or install the adapter --global",
            );
        }
        bail!("integration doctor unhealthy: {}", issues.join(", "));
    }
    Ok(())
}

#[derive(Default)]
struct LearningLifecycleProbe {
    recording_works: bool,
    feedback_works: bool,
    preview_works: bool,
    approved_apply_changes_decision: bool,
}

fn probe_learning_lifecycle() -> Result<LearningLifecycleProbe> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("BrainMap");
    vault::init_vault_quiet(Some(root.clone()), true)?;
    index::rebuild(&root)?;
    let request = |dry_run| gate::GateInput {
        intent: "integration-doctor".into(),
        situation: "Choose package manager for integration doctor".into(),
        options: vec!["npm".into(), "pnpm".into()],
        proposed_action: String::new(),
        risk: "low".into(),
        reversible: Some(true),
        decision_type: "tooling".into(),
        scope: "project:integration-doctor".into(),
        agent_confidence: None,
        dry_run,
    };
    let initial = gate::evaluate(&root, request(false))?;
    learning::record_decision_quiet(crate::cli::RecordDecisionArgs {
        decision_id: Some(initial.decision_id.clone()),
        chosen: Some("pnpm".into()),
        was_asked: Some(true),
        vault: Some(root.clone()),
    })?;
    let packet_id = learning::learn_feedback_quiet(crate::cli::LearnFeedbackArgs {
        decision_id: initial.decision_id,
        correction: None,
        chosen: Some("pnpm".into()),
        rejected: Some("npm".into()),
        incident: None,
        vault: Some(root.clone()),
    })?
    .context("integration learning probe did not create a packet")?;
    let preview = learning::pending_updates_value(&root, Some(&packet_id))?;
    learning::apply_update_by_id(&root, &packet_id)?;
    let changed = gate::evaluate(&root, request(true))?;
    Ok(LearningLifecycleProbe {
        recording_works: true,
        feedback_works: true,
        preview_works: preview.as_array().is_some_and(|packets| packets.len() == 1),
        approved_apply_changes_decision: changed.outcome == "ask_user"
            && changed.selected_option.is_none()
            && changed.predicted_outcome == "proceed"
            && changed.predicted_selected_option.as_deref() == Some("pnpm"),
    })
}

struct PlanItem {
    path: PathBuf,
    enforcement: &'static str,
    action: PlanAction,
}

enum PlanAction {
    OwnedText(String),
    ManagedText(String),
    ManagedToml(String),
    JsonHooks(Vec<HookBinding>),
    JsonInstruction(String),
}

#[derive(Clone)]
struct HookBinding {
    event: &'static str,
    matcher: Option<&'static str>,
    command: String,
    managed_suffix: String,
    managed_commands: Vec<String>,
    timeout_secs: u64,
}

#[derive(Clone, Copy)]
enum PlanChange {
    Create,
    Update,
    Unchanged,
    Remove,
}

impl PlanChange {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Unchanged => "leave unchanged",
            Self::Remove => "remove",
        }
    }
}

impl PlanItem {
    fn preflight(&self, uninstall: bool) -> Result<PlanChange> {
        let exists = self.path.exists();
        if !exists && uninstall {
            return Ok(PlanChange::Unchanged);
        }
        let existing = if exists {
            fs::read_to_string(&self.path)
                .with_context(|| format!("read {}", self.path.display()))?
        } else {
            String::new()
        };
        let desired = match &self.action {
            PlanAction::OwnedText(contents) => {
                if exists && existing != *contents {
                    bail!(
                        "refusing to {} unmanaged or modified file {}",
                        if uninstall { "remove" } else { "overwrite" },
                        self.path.display()
                    );
                }
                if uninstall {
                    return Ok(PlanChange::Remove);
                }
                contents.clone()
            }
            PlanAction::ManagedText(block) => merge_managed_text(&existing, block, uninstall),
            PlanAction::ManagedToml(block) => merge_managed_toml(&existing, block, uninstall)?,
            PlanAction::JsonHooks(bindings) => {
                json_hooks_contents(&self.path, bindings, uninstall)?
            }
            PlanAction::JsonInstruction(instruction) => {
                json_instruction_contents(&self.path, instruction, uninstall)?
            }
        };
        if !exists {
            Ok(PlanChange::Create)
        } else if desired == existing {
            Ok(PlanChange::Unchanged)
        } else if uninstall && desired.is_empty() {
            Ok(PlanChange::Remove)
        } else {
            Ok(PlanChange::Update)
        }
    }

    #[cfg(test)]
    fn contents(&self) -> Result<String> {
        match &self.action {
            PlanAction::OwnedText(contents)
            | PlanAction::ManagedText(contents)
            | PlanAction::ManagedToml(contents) => Ok(contents.clone()),
            PlanAction::JsonHooks(bindings) => json_hooks_contents(&self.path, bindings, false),
            PlanAction::JsonInstruction(instruction) => {
                json_instruction_contents(&self.path, instruction, false)
            }
        }
    }

    fn install(&self) -> Result<()> {
        if let PlanAction::OwnedText(contents) = &self.action {
            if self.path.exists() {
                let existing = fs::read_to_string(&self.path)
                    .with_context(|| format!("read {}", self.path.display()))?;
                if existing != *contents {
                    bail!(
                        "refusing to overwrite unmanaged file {}; move it or remove it explicitly",
                        self.path.display()
                    );
                }
                println!("unchanged {} ({})", self.path.display(), self.enforcement);
                return Ok(());
            }
            util::write_atomic(&self.path, contents.as_bytes())?;
            println!("wrote {} ({})", self.path.display(), self.enforcement);
            return Ok(());
        }
        let contents = match &self.action {
            PlanAction::OwnedText(_) => unreachable!(),
            PlanAction::ManagedText(block) => {
                let existing = if self.path.exists() {
                    fs::read_to_string(&self.path)
                        .with_context(|| format!("read {}", self.path.display()))?
                } else {
                    String::new()
                };
                merge_managed_text(&existing, block, false)
            }
            PlanAction::ManagedToml(block) => {
                let existing = if self.path.exists() {
                    fs::read_to_string(&self.path)
                        .with_context(|| format!("read {}", self.path.display()))?
                } else {
                    String::new()
                };
                merge_managed_toml(&existing, block, false)?
            }
            PlanAction::JsonHooks(bindings) => json_hooks_contents(&self.path, bindings, false)?,
            PlanAction::JsonInstruction(instruction) => {
                json_instruction_contents(&self.path, instruction, false)?
            }
        };
        if self.path.exists()
            && fs::read_to_string(&self.path)
                .with_context(|| format!("read {}", self.path.display()))?
                == contents
        {
            println!("unchanged {} ({})", self.path.display(), self.enforcement);
            return Ok(());
        }
        if self.path.exists() {
            backup(&self.path)?;
        }
        util::write_atomic(&self.path, contents.as_bytes())?;
        println!("wrote {} ({})", self.path.display(), self.enforcement);
        Ok(())
    }

    fn uninstall(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        match &self.action {
            PlanAction::OwnedText(contents) => {
                let existing = fs::read_to_string(&self.path)
                    .with_context(|| format!("read {}", self.path.display()))?;
                if existing != *contents {
                    bail!(
                        "refusing to remove unmanaged or modified file {}",
                        self.path.display()
                    );
                }
                backup(&self.path)?;
                fs::remove_file(&self.path)?;
                println!("removed {}", self.path.display());
            }
            PlanAction::ManagedText(block) => {
                backup(&self.path)?;
                let existing = fs::read_to_string(&self.path)?;
                let contents = merge_managed_text(&existing, block, true);
                if contents.is_empty() {
                    fs::remove_file(&self.path)?;
                    println!("removed {}", self.path.display());
                } else {
                    util::write_atomic(&self.path, contents.as_bytes())?;
                    println!("updated {} ({})", self.path.display(), self.enforcement);
                }
            }
            PlanAction::ManagedToml(block) => {
                backup(&self.path)?;
                let existing = fs::read_to_string(&self.path)?;
                let contents = merge_managed_toml(&existing, block, true)?;
                if contents.is_empty() {
                    fs::remove_file(&self.path)?;
                    println!("removed {}", self.path.display());
                } else {
                    util::write_atomic(&self.path, contents.as_bytes())?;
                    println!("updated {} ({})", self.path.display(), self.enforcement);
                }
            }
            PlanAction::JsonHooks(bindings) => {
                backup(&self.path)?;
                let contents = json_hooks_contents(&self.path, bindings, true)?;
                util::write_atomic(&self.path, contents.as_bytes())?;
                println!("updated {} ({})", self.path.display(), self.enforcement);
            }
            PlanAction::JsonInstruction(instruction) => {
                backup(&self.path)?;
                let contents = json_instruction_contents(&self.path, instruction, true)?;
                util::write_atomic(&self.path, contents.as_bytes())?;
                println!("updated {} ({})", self.path.display(), self.enforcement);
            }
        }
        Ok(())
    }
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| util::home_dir().join(".codex"))
}

fn codex_project_trusted(project: &std::path::Path) -> Result<bool> {
    let config_path = codex_home().join("config.toml");
    let text = match fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("read {}", config_path.display()));
        }
    };
    let document = text
        .parse::<toml::Table>()
        .with_context(|| format!("parse {}", config_path.display()))?;
    let canonical = fs::canonicalize(project)
        .with_context(|| format!("canonicalize Codex project {}", project.display()))?;
    Ok(document
        .get("projects")
        .and_then(toml::Value::as_table)
        .and_then(|projects| projects.get(canonical.to_string_lossy().as_ref()))
        .and_then(toml::Value::as_table)
        .and_then(|settings| settings.get("trust_level"))
        .and_then(toml::Value::as_str)
        == Some("trusted"))
}

fn configured_vault_path(vault: Option<&std::path::Path>) -> Result<PathBuf> {
    let expanded = vault
        .map(util::expand_tilde)
        .unwrap_or_else(util::default_vault);
    std::path::absolute(&expanded)
        .with_context(|| format!("resolve absolute vault path {}", expanded.display()))
}

fn running_brainmap_executable() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("resolve running brainmap executable")?;
    fs::canonicalize(&executable)
        .with_context(|| format!("canonicalize running executable {}", executable.display()))
}

fn plan(args: &InstallHarnessArgs) -> Result<Vec<PlanItem>> {
    let base = if args.global {
        util::home_dir()
    } else {
        args.project.clone().unwrap_or_else(|| PathBuf::from("."))
    };
    let items = match args.target.as_str() {
        "claude-code" => vec![
            PlanItem {
                path: base.join(".claude/skills/build-decision-engine/SKILL.md"),
                enforcement: "instruction-only",
                action: PlanAction::OwnedText(skill::build_decision_engine_shim("claude-code")),
            },
            PlanItem {
                path: base.join(".claude/settings.json"),
                enforcement: "enforced",
                action: PlanAction::JsonHooks(hook_bindings("claude-code")),
            },
        ],
        "codex" => {
            let executable = running_brainmap_executable()?;
            let vault = configured_vault_path(args.vault.as_deref())?;
            let codex_base = if args.global {
                codex_home()
            } else {
                base.join(".codex")
            };
            let agents_path = if args.global {
                codex_base.join("AGENTS.md")
            } else {
                base.join("AGENTS.md")
            };
            let config_path = codex_base.join("config.toml");
            let previous_command = managed_codex_mcp_command(&config_path);
            vec![
            PlanItem {
                path: codex_base.join("skills/build-decision-engine/SKILL.md"),
                enforcement: "instruction-only",
                action: PlanAction::OwnedText(skill::build_decision_engine_shim("codex")),
            },
            PlanItem {
                path: agents_path,
                enforcement: "instruction-only",
                action: PlanAction::ManagedText(managed_block("codex")),
            },
            PlanItem {
                path: config_path,
                enforcement: "best-effort",
                action: PlanAction::ManagedToml(codex_mcp_block(&vault, &executable)),
            },
            PlanItem {
                path: codex_base.join("hooks.json"),
                enforcement: "enforced",
                action: PlanAction::JsonHooks(codex_hook_bindings(
                    "codex",
                    &executable,
                    previous_command.as_deref(),
                )),
            },
        ]
        }
        "opencode" => vec![PlanItem {
            path: base.join("opencode.json"),
            enforcement: "best-effort",
            action: PlanAction::JsonInstruction(managed_block("opencode")),
        }],
        "copilot" => vec![PlanItem {
            path: base.join(".github/copilot-instructions.md"),
            enforcement: "instruction-only",
            action: PlanAction::ManagedText(managed_block("copilot")),
        }],
        "generic-stdio" => vec![PlanItem {
            path: base.join("brainmap-harness.md"),
            enforcement: "enforced",
            action: PlanAction::OwnedText("Generic stdio harness can enforce with `brainmap harness stdio --fail-on-block`. Send one JSON gate request per line; read one gate JSON response per line.\n".into()),
        }],
        _ => vec![PlanItem {
            path: base.join("brainmap-harness-unsupported.txt"),
            enforcement: "instruction-only",
            action: PlanAction::OwnedText(format!(
                "Unsupported target {}; no install performed\n",
                args.target
            )),
        }],
    };
    Ok(items)
}

fn hook_bindings(host: &str) -> Vec<HookBinding> {
    hook_bindings_with_commands(host, "brainmap", &[])
}

fn codex_hook_bindings(
    host: &str,
    executable: &std::path::Path,
    previous_command: Option<&str>,
) -> Vec<HookBinding> {
    let mut additional = vec!["brainmap".to_string()];
    if let Some(previous) = previous_command {
        let previous = if std::path::Path::new(previous).is_absolute() {
            shell_quote(previous)
        } else {
            previous.to_string()
        };
        additional.push(previous);
    }
    hook_bindings_with_commands(
        host,
        &shell_quote(executable.to_string_lossy().as_ref()),
        &additional,
    )
}

fn hook_bindings_with_commands(
    host: &str,
    command: &str,
    additional_managed_commands: &[String],
) -> Vec<HookBinding> {
    [
        ("UserPromptSubmit", None),
        ("PreToolUse", Some("Bash|Edit|Write|MultiEdit|NotebookEdit")),
    ]
    .into_iter()
    .map(|(event, matcher)| {
        let current = brainmap_hook_command(command, host, event);
        let mut managed_commands = vec![current.clone()];
        managed_commands.extend(
            additional_managed_commands
                .iter()
                .map(|owned| brainmap_hook_command(owned, host, event)),
        );
        managed_commands.sort();
        managed_commands.dedup();
        HookBinding {
            event,
            matcher,
            command: current,
            managed_suffix: brainmap_hook_suffix(host, event),
            managed_commands,
            timeout_secs: 10,
        }
    })
    .collect()
}

fn managed_codex_mcp_command(path: &std::path::Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    if text.matches(TOML_MANAGED_START).count() != 1 || text.matches(TOML_MANAGED_END).count() != 1
    {
        return None;
    }
    text.parse::<toml::Table>()
        .ok()?
        .get("mcp_servers")?
        .as_table()?
        .get("brainmap")?
        .as_table()?
        .get("command")?
        .as_str()
        .map(str::to_string)
}

fn brainmap_hook_command(command: &str, host: &str, event: &str) -> String {
    format!("{command} harness hook --host {host} --event {event}")
}

fn brainmap_hook_suffix(host: &str, event: &str) -> String {
    format!(" harness hook --host {host} --event {event}")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn backup(path: &PathBuf) -> Result<PathBuf> {
    let backup = path.with_extension(format!("bak-{}", chrono::Utc::now().timestamp_micros()));
    fs::copy(path, &backup)
        .with_context(|| format!("backup {} to {}", path.display(), backup.display()))?;
    println!("backup {}", backup.display());
    Ok(backup)
}

fn json_instruction_contents(path: &PathBuf, instruction: &str, uninstall: bool) -> Result<String> {
    let mut root = if path.exists() {
        serde_json::from_slice::<Value>(
            &fs::read(path).with_context(|| format!("read {}", path.display()))?,
        )
        .with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({})
    };
    let object = root
        .as_object_mut()
        .context("OpenCode configuration must be a JSON object")?;
    let existing = object
        .get("instructions")
        .map(|value| {
            value
                .as_str()
                .context("OpenCode instructions must be a string")
        })
        .transpose()?
        .unwrap_or_default();
    let merged = merge_managed_text(existing, instruction, uninstall);
    if merged.is_empty() {
        object.remove("instructions");
    } else {
        object.insert("instructions".into(), Value::String(merged));
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
}

const MANAGED_START: &str = "<!-- BEGIN BRAINMAP MANAGED BLOCK -->";
const MANAGED_END: &str = "<!-- END BRAINMAP MANAGED BLOCK -->";
const TOML_MANAGED_START: &str = "# BEGIN BRAINMAP MANAGED BLOCK";
const TOML_MANAGED_END: &str = "# END BRAINMAP MANAGED BLOCK";

fn merge_managed_text(existing: &str, block: &str, uninstall: bool) -> String {
    let cleaned = remove_managed_text(existing);
    if uninstall {
        return cleaned;
    }
    if cleaned.is_empty() {
        block.to_string()
    } else {
        format!("{cleaned}\n\n{block}")
    }
}

fn merge_managed_toml(existing: &str, block: &str, uninstall: bool) -> Result<String> {
    let start_markers = existing.matches(TOML_MANAGED_START).count();
    let end_markers = existing.matches(TOML_MANAGED_END).count();
    if start_markers != end_markers || start_markers > 1 {
        bail!("invalid or duplicate Brainmap managed TOML markers");
    }
    let cleaned = remove_marked_block(existing, TOML_MANAGED_START, TOML_MANAGED_END);
    if uninstall {
        return Ok(cleaned);
    }
    let existing_table = cleaned
        .parse::<toml::Table>()
        .context("invalid existing Codex TOML configuration")?;
    if existing_table
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .is_some_and(|servers| servers.contains_key("brainmap"))
    {
        bail!(
            "refusing to replace unmanaged Brainmap MCP table; remove [mcp_servers.brainmap] or manage it explicitly"
        );
    }
    let mut out = cleaned.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block.trim());
    out.push('\n');
    out.parse::<toml::Table>()
        .context("generated invalid Codex TOML configuration")?;
    Ok(out)
}

#[derive(Clone, Copy, Debug, Default)]
struct CodexMcpConfigStatus {
    valid: bool,
    vault_matches: bool,
}

fn codex_mcp_config_status(
    text: &str,
    expected_vault: &std::path::Path,
    expected_executable: &std::path::Path,
) -> CodexMcpConfigStatus {
    let Ok(document) = text.parse::<toml::Table>() else {
        return CodexMcpConfigStatus::default();
    };
    let Some(server) = document
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .and_then(|servers| servers.get("brainmap"))
        .and_then(toml::Value::as_table)
    else {
        return CodexMcpConfigStatus::default();
    };
    let Some(args) = server
        .get("args")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
        })
    else {
        return CodexMcpConfigStatus::default();
    };
    let expected_tools = [
        "brainmap_decision_gate",
        "brainmap_context",
        "brainmap_record_decision",
        "brainmap_learn_feedback",
        "brainmap_list_pending",
        "brainmap_preview_update",
        "brainmap_apply_update",
        "brainmap_autopilot_status",
    ];
    let enabled_tools = server
        .get("enabled_tools")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tools = server.get("tools").and_then(toml::Value::as_table);
    let approval_is_prompt = |tool: &str| {
        tools
            .and_then(|tools| tools.get(tool))
            .and_then(toml::Value::as_table)
            .and_then(|settings| settings.get("approval_mode"))
            .and_then(toml::Value::as_str)
            == Some("prompt")
    };
    let args_shape_valid = args.len() == 4 && args[..3] == ["mcp", "serve", "--vault"];
    let valid = server.get("command").and_then(toml::Value::as_str)
        == Some(expected_executable.to_string_lossy().as_ref())
        && args_shape_valid
        && server.get("required").and_then(toml::Value::as_bool) == Some(true)
        && server
            .get("default_tools_approval_mode")
            .and_then(toml::Value::as_str)
            == Some("auto")
        && enabled_tools == expected_tools
        && approval_is_prompt("brainmap_learn_feedback")
        && approval_is_prompt("brainmap_apply_update");
    CodexMcpConfigStatus {
        valid,
        vault_matches: valid
            && args.get(3).copied() == Some(expected_vault.to_string_lossy().as_ref()),
    }
}

fn remove_marked_block(existing: &str, start_marker: &str, end_marker: &str) -> String {
    let Some(start) = existing.find(start_marker) else {
        return existing.trim().to_string() + if existing.trim().is_empty() { "" } else { "\n" };
    };
    let Some(relative_end) = existing[start..].find(end_marker) else {
        return existing.to_string();
    };
    let end = start + relative_end + end_marker.len();
    let mut out = format!("{}{}", &existing[..start], &existing[end..]);
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_string() + if out.trim().is_empty() { "" } else { "\n" }
}

fn codex_mcp_block(vault: &std::path::Path, executable: &std::path::Path) -> String {
    let vault = serde_json::to_string(&vault.to_string_lossy()).unwrap();
    let executable = serde_json::to_string(&executable.to_string_lossy()).unwrap();
    format!(
        r#"{TOML_MANAGED_START}
[mcp_servers.brainmap]
command = {executable}
args = ["mcp", "serve", "--vault", {vault}]
required = true
default_tools_approval_mode = "auto"
enabled_tools = ["brainmap_decision_gate", "brainmap_context", "brainmap_record_decision", "brainmap_learn_feedback", "brainmap_list_pending", "brainmap_preview_update", "brainmap_apply_update", "brainmap_autopilot_status"]

[mcp_servers.brainmap.tools.brainmap_learn_feedback]
approval_mode = "prompt"

[mcp_servers.brainmap.tools.brainmap_apply_update]
approval_mode = "prompt"
{TOML_MANAGED_END}"#
    )
}

fn remove_managed_text(existing: &str) -> String {
    let Some(start) = existing.find(MANAGED_START) else {
        return existing.to_string();
    };
    let Some(relative_end) = existing[start..].find(MANAGED_END) else {
        return existing.to_string();
    };
    let end = start + relative_end + MANAGED_END.len();
    let mut before = existing[..start].to_string();
    if before.ends_with("\n\n") {
        before.truncate(before.len() - 2);
    }
    let mut after = &existing[end..];
    if let Some(rest) = after.strip_prefix('\n') {
        after = rest;
    }
    format!("{before}{after}")
}

fn json_hooks_contents(
    path: &PathBuf,
    bindings: &[HookBinding],
    uninstall: bool,
) -> Result<String> {
    let root = if path.exists() {
        serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
            .with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({})
    };
    let merged = merge_hook_bindings(root, bindings, uninstall)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&merged)?))
}

fn merge_hook_bindings(
    mut root: Value,
    bindings: &[HookBinding],
    uninstall: bool,
) -> Result<Value> {
    let root_obj = root
        .as_object_mut()
        .context("hook configuration must be a JSON object")?;
    let hooks = root_obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .context("hook configuration 'hooks' must be an object")?;

    for binding in bindings {
        let entries = hooks_obj.entry(binding.event).or_insert_with(|| json!([]));
        let entries = entries
            .as_array_mut()
            .with_context(|| format!("hook event '{}' must be an array", binding.event))?;
        if uninstall {
            entries.retain_mut(|entry| remove_managed_hook_commands(entry, binding));
        } else {
            entries.retain_mut(|entry| remove_managed_hook_commands(entry, binding));
            entries.push(binding_json(binding));
        }
    }
    Ok(root)
}

fn remove_managed_hook_commands(entry: &mut Value, binding: &HookBinding) -> bool {
    let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
        return true;
    };
    hooks.retain(|hook| {
        !hook
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| {
                binding
                    .managed_commands
                    .iter()
                    .any(|managed| managed == command)
            })
    });
    !hooks.is_empty()
}

fn binding_json(binding: &HookBinding) -> Value {
    let mut entry = json!({
        "hooks": [
            {
                "type": "command",
                "command": binding.command,
                "timeout": binding.timeout_secs
            }
        ]
    });
    if let Some(matcher) = binding.matcher {
        entry["matcher"] = json!(matcher);
    }
    entry
}

#[cfg(test)]
fn entry_has_command(entry: &Value, command: &str) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks
                .iter()
                .any(|hook| hook.get("command").and_then(Value::as_str) == Some(command))
        })
}

fn hook_bindings_installed(root: &Value, bindings: &[HookBinding]) -> bool {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    bindings.iter().all(|binding| {
        let Some(entries) = hooks.get(binding.event).and_then(Value::as_array) else {
            return false;
        };
        let matching = entries
            .iter()
            .flat_map(|entry| {
                entry
                    .get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|command| command.get("command").and_then(Value::as_str))
            .filter(|command| command.ends_with(&binding.managed_suffix))
            .collect::<Vec<_>>();
        matching.len() == 1
            && matching[0] == binding.command
            && entries.iter().any(|entry| {
                entry.get("matcher").and_then(Value::as_str) == binding.matcher
                    && entry
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|commands| {
                            commands.iter().any(|command| {
                                command.get("type").and_then(Value::as_str) == Some("command")
                                    && command.get("command").and_then(Value::as_str)
                                        == Some(binding.command.as_str())
                                    && command.get("timeout").and_then(Value::as_u64)
                                        == Some(binding.timeout_secs)
                            })
                        })
            })
    })
}

fn managed_block(host: &str) -> String {
    format!(
        r#"<!-- BEGIN BRAINMAP MANAGED BLOCK -->
# Brainmap Harness Instructions

Host: {host}
Enforcement: host hooks call `brainmap harness hook`; this file is the fallback.

Load current local instructions before decision-engine work:

```bash
brainmap skill build-decision-engine --host {host}
```

If that command fails, run `brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json` before decision questions. Ask naturally with concrete options and a free-text path. Never store secrets or raw project archives.
<!-- END BRAINMAP MANAGED BLOCK -->
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_executable(path: &Path, contents: &[u8]) {
        fs::write(path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn candidate_test_source(root: &Path) -> CandidateInstallSource {
        let brainmap = root.join(if cfg!(windows) {
            "brainmap.exe"
        } else {
            "brainmap"
        });
        let brainmapd = root.join(if cfg!(windows) {
            "brainmapd.exe"
        } else {
            "brainmapd"
        });
        write_test_executable(&brainmap, b"qualified brainmap");
        write_test_executable(&brainmapd, b"qualified brainmapd");
        CandidateInstallSource {
            brainmap_sha256: file_sha256(&brainmap).unwrap(),
            brainmapd_sha256: file_sha256(&brainmapd).unwrap(),
            brainmap,
            brainmapd,
            candidate_commit: "0123456789abcdef0123456789abcdef01234567".into(),
        }
    }

    #[test]
    fn candidate_install_dry_run_plans_without_mutation() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        fs::create_dir(&source_root).unwrap();
        let source = candidate_test_source(&source_root);
        let destination = tmp.path().join("bin");

        let plan = plan_candidate_install(
            &source,
            &destination,
            true,
            std::slice::from_ref(&destination),
        )
        .unwrap();

        assert!(plan.dry_run);
        assert_eq!(plan.brainmap_change, CandidateInstallChange::Create);
        assert_eq!(plan.brainmapd_change, CandidateInstallChange::Create);
        assert!(!plan.backup_required);
        assert!(!destination.exists());
    }

    #[test]
    fn candidate_install_normalizes_relative_and_empty_path_entries() {
        let current = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();

        assert_eq!(canonical_path_entry(Path::new(".")).unwrap(), current);
        assert_eq!(canonical_path_entry(Path::new("")).unwrap(), current);
    }

    #[test]
    fn candidate_install_backs_up_updates_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        let destination = tmp.path().join("bin");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination).unwrap();
        let source = candidate_test_source(&source_root);
        let brainmap = destination.join(if cfg!(windows) {
            "brainmap.exe"
        } else {
            "brainmap"
        });
        let brainmapd = destination.join(if cfg!(windows) {
            "brainmapd.exe"
        } else {
            "brainmapd"
        });
        write_test_executable(&brainmap, b"old brainmap");
        write_test_executable(&brainmapd, b"old brainmapd");

        let first = install_candidate_files(
            &source,
            &destination,
            std::slice::from_ref(&destination),
            |installed_brainmap, installed_brainmapd| {
                ensure!(fs::read(installed_brainmap)? == b"qualified brainmap");
                ensure!(fs::read(installed_brainmapd)? == b"qualified brainmapd");
                Ok(())
            },
        )
        .unwrap();
        assert!(!first.idempotent);
        assert_eq!(first.backups.len(), 2);

        let backup_count = fs::read_dir(&destination)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".bak-"))
            .count();
        let second = install_candidate_files(
            &source,
            &destination,
            std::slice::from_ref(&destination),
            |_, _| Ok(()),
        )
        .unwrap();
        assert!(second.idempotent);
        assert!(second.backups.is_empty());
        assert_eq!(
            fs::read_dir(&destination)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".bak-"))
                .count(),
            backup_count
        );
    }

    #[test]
    fn candidate_install_rolls_back_when_post_install_verification_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        let destination = tmp.path().join("bin");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination).unwrap();
        let source = candidate_test_source(&source_root);
        let brainmap = destination.join(if cfg!(windows) {
            "brainmap.exe"
        } else {
            "brainmap"
        });
        let brainmapd = destination.join(if cfg!(windows) {
            "brainmapd.exe"
        } else {
            "brainmapd"
        });
        write_test_executable(&brainmap, b"old brainmap");
        write_test_executable(&brainmapd, b"old brainmapd");

        let error = install_candidate_files(
            &source,
            &destination,
            std::slice::from_ref(&destination),
            |_, _| bail!("qualification rejected installed pair"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("rolled back"));
        assert_eq!(fs::read(&brainmap).unwrap(), b"old brainmap");
        assert_eq!(fs::read(&brainmapd).unwrap(), b"old brainmapd");
        assert!(
            !destination
                .join(".brainmap-candidate-install.lock")
                .exists()
        );
    }

    #[test]
    fn candidate_install_rejects_a_shadowed_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        let shadow = tmp.path().join("shadow");
        let destination = tmp.path().join("bin");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&shadow).unwrap();
        let source = candidate_test_source(&source_root);
        write_test_executable(
            &shadow.join(if cfg!(windows) {
                "brainmap.exe"
            } else {
                "brainmap"
            }),
            b"shadow brainmap",
        );
        write_test_executable(
            &shadow.join(if cfg!(windows) {
                "brainmapd.exe"
            } else {
                "brainmapd"
            }),
            b"shadow brainmapd",
        );

        let error =
            plan_candidate_install(&source, &destination, true, &[shadow, destination.clone()])
                .unwrap_err();

        assert!(error.to_string().contains("shadowed"));
    }

    #[cfg(unix)]
    #[test]
    fn candidate_install_rejects_symlink_targets() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        let destination = tmp.path().join("bin");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&destination).unwrap();
        let source = candidate_test_source(&source_root);
        let target = tmp.path().join("user-owned");
        write_test_executable(&target, b"user owned");
        symlink(&target, destination.join("brainmap")).unwrap();

        let error = plan_candidate_install(
            &source,
            &destination,
            true,
            std::slice::from_ref(&destination),
        )
        .unwrap_err();

        assert!(error.to_string().contains("symlink-free regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn candidate_install_detects_a_symlinked_path_shadow() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let source_root = tmp.path().join("source");
        let shadow = tmp.path().join("shadow");
        let destination = tmp.path().join("bin");
        fs::create_dir(&source_root).unwrap();
        fs::create_dir(&shadow).unwrap();
        let source = candidate_test_source(&source_root);
        let shadow_target = tmp.path().join("shadow-target");
        write_test_executable(&shadow_target, b"shadow");
        symlink(&shadow_target, shadow.join("brainmap")).unwrap();
        let shadow_path_entry = tmp.path().join("shadow-path-entry");
        symlink(&shadow, &shadow_path_entry).unwrap();

        let error = plan_candidate_install(
            &source,
            &destination,
            true,
            &[shadow_path_entry, destination.clone()],
        )
        .unwrap_err();

        assert!(error.to_string().contains("shadowed"));
    }

    #[test]
    fn merge_hooks_is_idempotent_and_preserves_existing_hooks() {
        let bindings = hook_bindings("codex");
        let root = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {"type": "command", "command": "rtk hook claude"}
                        ]
                    }
                ]
            }
        });
        let merged = merge_hook_bindings(root, &bindings, false).unwrap();
        let pre_tool = merged["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre_tool
                .iter()
                .any(|entry| entry_has_command(entry, "rtk hook claude"))
        );
        assert!(pre_tool.iter().any(|entry| entry_has_command(
            entry,
            "brainmap harness hook --host codex --event PreToolUse"
        )));

        let merged_again = merge_hook_bindings(merged.clone(), &bindings, false).unwrap();
        assert_eq!(
            pre_tool.len(),
            merged_again["hooks"]["PreToolUse"]
                .as_array()
                .unwrap()
                .len()
        );

        let removed = merge_hook_bindings(merged_again, &bindings, true).unwrap();
        let pre_tool = removed["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre_tool
                .iter()
                .any(|entry| entry_has_command(entry, "rtk hook claude"))
        );
        assert!(!pre_tool.iter().any(|entry| entry_has_command(
            entry,
            "brainmap harness hook --host codex --event PreToolUse"
        )));
    }

    #[test]
    fn uninstall_removes_only_brainmap_command_from_mixed_hook_entry() {
        let bindings = hook_bindings("codex");
        let root = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [
                        {"type": "command", "command": "brainmap harness hook --host codex --event PreToolUse"},
                        {"type": "command", "command": "user-owned-hook"}
                    ]
                }]
            }
        });

        let removed = merge_hook_bindings(root, &bindings, true).unwrap();
        let entries = removed["hooks"]["PreToolUse"].as_array().unwrap();

        assert_eq!(entries.len(), 1);
        assert!(entry_has_command(&entries[0], "user-owned-hook"));
        assert!(!entry_has_command(
            &entries[0],
            "brainmap harness hook --host codex --event PreToolUse"
        ));
    }

    #[test]
    fn codex_plan_installs_skill() {
        let args = InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(PathBuf::from("/tmp/brainmap-project")),
            vault: None,
            dry_run: true,
            uninstall: false,
        };
        let plan = plan(&args).unwrap();

        assert!(plan.iter().any(|item| {
            item.path
                .ends_with(".codex/skills/build-decision-engine/SKILL.md")
                && item.enforcement == "instruction-only"
        }));
        let config = plan
            .iter()
            .find(|item| item.path.ends_with(".codex/config.toml"))
            .unwrap()
            .contents()
            .unwrap();
        assert!(config.contains("[mcp_servers.brainmap]"));
        assert!(config.contains("brainmap_apply_update"));
        assert!(config.contains("default_tools_approval_mode = \"auto\""));
        assert!(config.contains("approval_mode = \"prompt\""));
    }

    #[test]
    fn codex_mcp_config_merge_is_idempotent_and_preserves_user_toml() {
        let existing = "model = \"gpt-5\"\n";
        let block = codex_mcp_block(
            std::path::Path::new("/tmp/BrainMap"),
            std::path::Path::new("/opt/brainmap/bin/brainmap"),
        );
        let merged = merge_managed_toml(existing, &block, false).unwrap();
        let merged_again = merge_managed_toml(&merged, &block, false).unwrap();

        assert_eq!(merged, merged_again);
        assert!(merged.contains("model = \"gpt-5\""));
        assert!(merged.contains("/tmp/BrainMap"));
        assert_eq!(
            merge_managed_toml(&merged, &block, true).unwrap(),
            "model = \"gpt-5\"\n"
        );
    }

    #[test]
    fn codex_install_refuses_an_unmanaged_brainmap_mcp_table() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let config = project.join(".codex/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let original = r#"[mcp_servers.brainmap]
command = "user-owned-brainmap"
"#;
        fs::write(&config, original).unwrap();

        let error = install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project),
            vault: Some(PathBuf::from("/tmp/BrainMap")),
            dry_run: false,
            uninstall: false,
        })
        .unwrap_err();

        assert!(error.to_string().contains("unmanaged Brainmap MCP table"));
        assert_eq!(fs::read_to_string(config).unwrap(), original);
    }

    #[test]
    fn installed_skill_is_static_cli_shim() {
        let args = InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(PathBuf::from("/tmp/brainmap-project")),
            vault: None,
            dry_run: true,
            uninstall: false,
        };
        let plan = plan(&args).unwrap();
        let skill = plan
            .iter()
            .find(|item| {
                item.path
                    .ends_with(".codex/skills/build-decision-engine/SKILL.md")
            })
            .unwrap()
            .contents()
            .unwrap();

        assert!(skill.contains("brainmap skill build-decision-engine --host codex"));
        assert!(skill.contains("If that command fails"));
        assert!(!skill.contains("Use Brainmap to learn decisions, not knowledge."));
    }

    #[test]
    fn codex_install_and_uninstall_preserve_existing_agents_content() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let agents = project.join("AGENTS.md");
        let original = "# Project instructions\n\nKeep this content.\n";
        fs::write(&agents, original).unwrap();

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap();

        let installed = fs::read_to_string(&agents).unwrap();
        assert!(installed.contains(original.trim()));
        assert!(installed.contains("BEGIN BRAINMAP MANAGED BLOCK"));
        let backups_after_first_install = fs::read_dir(&project)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.bak-")
            })
            .count();

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&agents).unwrap(), installed);
        let backups_after_second_install = fs::read_dir(&project)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.bak-")
            })
            .count();
        assert_eq!(backups_after_second_install, backups_after_first_install);

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: true,
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&agents).unwrap(), original);
        let backups = fs::read_dir(&project)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.bak-")
            })
            .count();
        assert_eq!(backups, 2);
    }

    #[test]
    fn opencode_install_preserves_json_and_existing_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let config = project.join("opencode.json");
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({
                "theme": "dark",
                "instructions": "Keep existing instructions."
            }))
            .unwrap(),
        )
        .unwrap();

        install_harness(InstallHarnessArgs {
            target: "opencode".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap();
        let installed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(installed["theme"], "dark");
        assert!(
            installed["instructions"]
                .as_str()
                .unwrap()
                .contains("Keep existing instructions.")
        );
        assert!(
            installed["instructions"]
                .as_str()
                .unwrap()
                .contains(MANAGED_START)
        );

        install_harness(InstallHarnessArgs {
            target: "opencode".into(),
            global: false,
            project: Some(project),
            vault: None,
            dry_run: false,
            uninstall: true,
        })
        .unwrap();
        let uninstalled: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
        assert_eq!(uninstalled["theme"], "dark");
        assert_eq!(uninstalled["instructions"], "Keep existing instructions.");
    }

    #[test]
    fn owned_files_are_never_overwritten_or_removed_when_unmanaged() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let skill_path = project.join(".codex/skills/build-decision-engine/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, "user-owned skill\n").unwrap();

        let install_error = install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap_err();
        assert!(install_error.to_string().contains("refusing to overwrite"));
        assert_eq!(
            fs::read_to_string(&skill_path).unwrap(),
            "user-owned skill\n"
        );

        let uninstall_error = install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project),
            vault: None,
            dry_run: false,
            uninstall: true,
        })
        .unwrap_err();
        assert!(uninstall_error.to_string().contains("refusing to remove"));
        assert_eq!(
            fs::read_to_string(&skill_path).unwrap(),
            "user-owned skill\n"
        );
    }
}
