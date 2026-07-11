use crate::cli::{DogfoodAbortArgs, DogfoodFinalizeArgs, DogfoodReviewArgs, DogfoodStartArgs};
use crate::{export, learning, privacy, qualification, util, vault};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(test)]
#[path = "../tests/support/qualification.rs"]
mod qualification_test_support;

const STATE_FORMAT: &str = "brainmap-dogfood-runs";
const STATE_VERSION: u32 = 3;
const START_CLOCK_TOLERANCE: Duration = Duration::minutes(5);
const MINIMUM_COMPLETE_PAIRS: u64 = 30;
const MINIMUM_DECISION_SCENARIOS: u64 = 5;
const MINIMUM_SCOPES_OR_DECISION_TYPES: u64 = 3;
const REVIEW_SCHEMA: &str = "brainmap-dogfood-review-v1";
const STATE_RELATIVE_PATH: &str = ".brainmap/dogfood.json";
const TRANSACTION_RELATIVE_PATH: &str = ".brainmap/locks/dogfood-transaction.json";
const LEDGER_RELATIVE_PATH: &str = "90-calibration/decision-ledger.jsonl";
const TRANSACTION_FORMAT: &str = "brainmap-dogfood-transaction";
const TRANSACTION_VERSION: u32 = 3;

#[cfg(test)]
static GATE_CONTEXT_LOADS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DogfoodState {
    pub(crate) format: String,
    pub(crate) version: u32,
    pub(crate) runs: Vec<DogfoodRunState>,
}

impl Default for DogfoodState {
    fn default() -> Self {
        Self {
            format: STATE_FORMAT.into(),
            version: STATE_VERSION,
            runs: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DogfoodRunState {
    pub(crate) run_id: String,
    pub(crate) status: DogfoodRunStatus,
    pub(crate) candidate_commit: String,
    pub(crate) candidate_binary_sha256: String,
    pub(crate) candidate_brainmapd_sha256: String,
    pub(crate) candidate_binary_identity: CandidateBinaryIdentity,
    pub(crate) host: HostProvenance,
    pub(crate) adapter: String,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) mode: String,
    pub(crate) gate_mode: String,
    pub(crate) autopilot_mode: String,
    pub(crate) autopilot_level: String,
    pub(crate) threshold: f64,
    pub(crate) start_backup: ChecksummedArtifact,
    pub(crate) qualification_bundle_sha256: String,
    pub(crate) qualification_manifest_sha256: String,
    pub(crate) qualification_bundle_relative_path: String,
    pub(crate) ledger_boundary_bytes: u64,
    pub(crate) ledger_boundary_lines: u64,
    pub(crate) ledger_boundary_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) aborted_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) abort_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finalized_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) final_export: Option<ChecksummedArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sign_off: Option<QualificationSignOff>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DogfoodRunStatus {
    Active,
    Aborted,
    Finalized,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostProvenance {
    pub(crate) os: String,
    pub(crate) arch: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChecksummedArtifact {
    pub(crate) relative_path: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CandidateBinaryIdentity {
    len: u64,
    modified_unix_nanos: Option<u64>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_unix_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DogfoodRunContext {
    pub(crate) run_id: String,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) ledger_boundary_bytes: u64,
    pub(crate) ledger_boundary_lines: u64,
    pub(crate) ledger_boundary_sha256: String,
    pub(crate) mode: String,
    pub(crate) gate_mode: String,
    pub(crate) autopilot_mode: String,
    pub(crate) autopilot_level: String,
    pub(crate) threshold: f64,
    pub(crate) candidate_commit: String,
    pub(crate) candidate_binary_sha256: String,
    pub(crate) candidate_binary_identity: CandidateBinaryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GateContextVersion {
    state: Option<GateFileVersion>,
    transaction: Option<GateFileVersion>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GateFileVersion {
    len: u64,
    modified_unix_nanos: Option<u128>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_unix_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GateProvenanceVersion {
    autopilot: Option<GateFileVersion>,
    gate_mode: Option<GateFileVersion>,
    candidate_binary_identity: CandidateBinaryIdentity,
}

type GateContextCache = HashMap<PathBuf, (GateContextVersion, Option<DogfoodRunContext>)>;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QualificationSignOff {
    pub(crate) signer: String,
    pub(crate) incident_disposition: String,
    pub(crate) signed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QualificationReport {
    schema_version: &'static str,
    status: &'static str,
    run_id: String,
    candidate_commit: String,
    candidate_binary_sha256: String,
    candidate_brainmapd_sha256: String,
    host: HostProvenance,
    adapter: String,
    mode: String,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    duration_seconds: i64,
    start_backup: ChecksummedArtifact,
    qualification: QualificationProvenance,
    ledger: LedgerProvenance,
    final_backup_sha256: String,
    shadow_metrics: serde_json::Value,
    safety: SafetyQualification,
    review_summary: ReviewSummary,
    sign_off: QualificationSignOff,
    raw_prompts_retained: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct QualificationProvenance {
    relative_path: String,
    run_relative_path: String,
    bundle_sha256: String,
    manifest_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LedgerProvenance {
    boundary_bytes: u64,
    boundary_lines: u64,
    boundary_sha256: String,
    final_bytes: u64,
    final_lines: u64,
    final_sha256: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SafetyQualification {
    passed: bool,
    false_proceeds: u64,
    confirmed_collisions: u64,
    confirmed_cross_domain_applications: u64,
    privacy_violations: u64,
    hard_rule_violations: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunHealth {
    qualification_bundle_matches: bool,
    binary_matches: bool,
    brainmapd_matches: bool,
    host_matches: bool,
    shadow_mode_intact: bool,
    start_backup_valid: bool,
    ledger_prefix_valid: bool,
    metrics_integrity_valid: bool,
    safety_clean: bool,
    intensive_session_ready: bool,
    review_integrity_valid: bool,
    review_ready: bool,
    healthy: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum IncidentStatus {
    Clear,
    Investigating,
    ResolvedNoViolation,
    CandidateFailed,
}

impl IncidentStatus {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "clear" => Ok(Self::Clear),
            "investigating" => Ok(Self::Investigating),
            "resolved-no-violation" => Ok(Self::ResolvedNoViolation),
            "candidate-failed" => Ok(Self::CandidateFailed),
            _ => bail!("unsupported dogfood incident status: {value}"),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Investigating => "investigating",
            Self::ResolvedNoViolation => "resolved-no-violation",
            Self::CandidateFailed => "candidate-failed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ReviewSafetyCounters {
    false_proceeds: u64,
    confirmed_cross_domain_applications: u64,
    privacy_violations: u64,
    hard_rule_violations: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct DogfoodReviewReceipt {
    schema_version: String,
    kind: String,
    id: String,
    dogfood_run_id: String,
    created_at: String,
    incident_status: IncidentStatus,
    ledger_prefix_bytes: u64,
    ledger_prefix_lines: u64,
    ledger_prefix_sha256: String,
    shadow_metrics_sha256: String,
    safety: ReviewSafetyCounters,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewSummary {
    schema_version: &'static str,
    review_count: usize,
    first_review_at: Option<String>,
    last_review_at: Option<String>,
    current_incident_status: Option<IncidentStatus>,
    candidate_failed: bool,
    unresolved_investigation: bool,
    integrity_valid: bool,
    final_review_covers_ledger: bool,
    review_ready: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "bytes", rename_all = "snake_case")]
enum FileSnapshot {
    Missing,
    File(Vec<u8>),
    Other,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransactionJournal {
    format: String,
    version: u32,
    transaction: DogfoodTransaction,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum DogfoodTransaction {
    Start(Box<StartTransaction>),
    Finalize(FinalizeTransaction),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum StartPhase {
    Prepared,
    BackupCreated,
    QualificationDirectoryPrepared,
    QualificationBundleCopied,
    IntentRecorded,
    AutopilotWritten,
    GateModeWritten,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartTransaction {
    phase: StartPhase,
    previous_state_existed: bool,
    previous_state: DogfoodState,
    prior_autopilot: FileSnapshot,
    prior_gate_mode: FileSnapshot,
    backup_relative_path: String,
    qualification_bundle_relative_path: String,
    intended_run: Option<DogfoodRunState>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FinalizePhase {
    Prepared,
    ArchivedStateWritten,
    StagingCreated,
    ExportWritten,
    QualificationBundleCopied,
    JsonReportWritten,
    MarkdownReportWritten,
    ChecksumsWritten,
    EvidenceActivated,
    FinalStateWritten,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizeTransaction {
    phase: FinalizePhase,
    run_id: String,
    active_index: usize,
    original_state: DogfoodState,
    archived_state: DogfoodState,
    out: PathBuf,
    staging: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReconcileOutcome {
    None,
    StartActivated,
    Finalized,
    RolledBack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartFault {
    JournalPrepared,
    BackupCreated,
    QualificationDirectoryPrepared,
    QualificationBundleCopied,
    IntentRecorded,
    AutopilotWritten,
    GateModeWritten,
    StateActivated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FinalizeFault {
    JournalPrepared,
    ArchivedStateWritten,
    StagingCreated,
    ExportWritten,
    QualificationBundleCopied,
    JsonReportWritten,
    MarkdownReportWritten,
    ChecksumsWritten,
    EvidenceActivated,
    FinalStateWritten,
}

impl FileSnapshot {
    fn capture(path: &Path) -> Result<Self> {
        if path.is_file() {
            return Ok(Self::File(
                fs::read(path).with_context(|| format!("snapshot {}", path.display()))?,
            ));
        }
        if path.exists() {
            Ok(Self::Other)
        } else {
            Ok(Self::Missing)
        }
    }

    fn restore(&self, path: &Path) -> Result<()> {
        match self {
            Self::File(bytes) => util::write_atomic(path, bytes),
            Self::Missing => {
                if path.is_file() {
                    fs::remove_file(path)
                        .with_context(|| format!("remove rollback file {}", path.display()))?;
                }
                Ok(())
            }
            Self::Other => Ok(()),
        }
    }
}

fn transaction_path(root: &Path) -> PathBuf {
    root.join(TRANSACTION_RELATIVE_PATH)
}

fn write_transaction(root: &Path, transaction: DogfoodTransaction) -> Result<()> {
    let journal = TransactionJournal {
        format: TRANSACTION_FORMAT.into(),
        version: TRANSACTION_VERSION,
        transaction,
    };
    util::write_atomic(
        &transaction_path(root),
        &serde_json::to_vec_pretty(&journal)?,
    )
}

fn load_transaction(root: &Path) -> Result<Option<DogfoodTransaction>> {
    let path = transaction_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let journal: TransactionJournal = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .context("parse dogfood transaction journal")?;
    if journal.format != TRANSACTION_FORMAT || journal.version != TRANSACTION_VERSION {
        bail!("unsupported dogfood transaction journal format or version");
    }
    validate_transaction_journal(root, &journal.transaction)?;
    Ok(Some(journal.transaction))
}

fn validate_transaction_journal(root: &Path, transaction: &DogfoodTransaction) -> Result<()> {
    match transaction {
        DogfoodTransaction::Start(transaction) => {
            validate_start_transaction_paths(root, transaction)?;
            validate_state_snapshot(root, "previous transaction", &transaction.previous_state)?;
            if transaction
                .previous_state
                .runs
                .iter()
                .any(|run| run.status == DogfoodRunStatus::Active)
            {
                bail!("invalid dogfood start transaction journal: previous state is active");
            }
            let intent_required = matches!(
                transaction.phase,
                StartPhase::IntentRecorded
                    | StartPhase::AutopilotWritten
                    | StartPhase::GateModeWritten
            );
            if intent_required != transaction.intended_run.is_some() {
                bail!("invalid dogfood start transaction journal phase or intent");
            }
            if let Some(run) = &transaction.intended_run
                && (run.status != DogfoodRunStatus::Active
                    || run.start_backup.relative_path != transaction.backup_relative_path
                    || run.qualification_bundle_relative_path
                        != transaction.qualification_bundle_relative_path
                    || run.aborted_at.is_some()
                    || run.abort_reason.is_some()
                    || run.finalized_at.is_some()
                    || run.final_export.is_some()
                    || run.sign_off.is_some())
            {
                bail!("invalid dogfood start transaction journal intent");
            }
        }
        DogfoodTransaction::Finalize(transaction) => {
            validate_finalize_transaction_paths(root, transaction)?;
            validate_state_snapshot(root, "original transaction", &transaction.original_state)?;
            validate_state_snapshot(root, "archived transaction", &transaction.archived_state)?;
            if transaction.original_state.runs.len() != transaction.archived_state.runs.len()
                || transaction.active_index >= transaction.original_state.runs.len()
            {
                bail!("invalid dogfood finalize transaction journal active index");
            }
            for (index, (original, archived)) in transaction
                .original_state
                .runs
                .iter()
                .zip(&transaction.archived_state.runs)
                .enumerate()
            {
                if index != transaction.active_index && original != archived {
                    bail!("invalid dogfood finalize transaction journal state snapshot");
                }
            }
            let original = &transaction.original_state.runs[transaction.active_index];
            let archived = &transaction.archived_state.runs[transaction.active_index];
            let mut normalized = archived.clone();
            normalized.status = DogfoodRunStatus::Active;
            normalized.finalized_at = None;
            normalized.final_export = None;
            normalized.sign_off = None;
            if original.run_id != transaction.run_id
                || original.status != DogfoodRunStatus::Active
                || archived.run_id != transaction.run_id
                || archived.status != DogfoodRunStatus::Finalized
                || archived.finalized_at.is_none()
                || archived.sign_off.is_none()
                || normalized != *original
            {
                bail!("invalid dogfood finalize transaction journal state transition");
            }
            let final_export_expected = transaction.phase == FinalizePhase::FinalStateWritten;
            if final_export_expected != archived.final_export.is_some() {
                bail!("invalid dogfood finalize transaction journal export state");
            }
        }
    }
    Ok(())
}

fn validate_state_snapshot(root: &Path, label: &str, state: &DogfoodState) -> Result<()> {
    if state.format != STATE_FORMAT || state.version != STATE_VERSION {
        bail!("invalid dogfood {label} state format or version");
    }
    if state
        .runs
        .iter()
        .filter(|run| run.status == DogfoodRunStatus::Active)
        .count()
        > 1
    {
        bail!("invalid dogfood {label} state: multiple active runs");
    }
    for run in &state.runs {
        validate_run_scoped_paths(root, run)?;
    }
    Ok(())
}

fn validate_run_scoped_paths(root: &Path, run: &DogfoodRunState) -> Result<()> {
    validate_safe_label("dogfood run id", &run.run_id)?;
    let expected_backup = format!("99-meta/backups/{}-start.brainmap.tar.zst", run.run_id);
    if run.start_backup.relative_path != expected_backup {
        bail!("dogfood start backup path is not scoped to its run ID");
    }
    validate_internal_transaction_path(root, "state start backup", &expected_backup)?;

    let expected_bundle = format!(".brainmap/dogfood/{}/qualification", run.run_id);
    if run.qualification_bundle_relative_path != expected_bundle {
        bail!("dogfood qualification bundle path is not scoped to its run ID");
    }
    validate_internal_transaction_path(root, "state qualification bundle", &expected_bundle)?;
    Ok(())
}

fn validate_start_transaction_paths(root: &Path, transaction: &StartTransaction) -> Result<()> {
    let backup = validate_internal_transaction_path(
        root,
        "start backup",
        &transaction.backup_relative_path,
    )?;
    let bundle = validate_internal_transaction_path(
        root,
        "qualification bundle",
        &transaction.qualification_bundle_relative_path,
    )?;
    if backup.parent() != Some(Path::new("99-meta/backups")) {
        bail!("unsafe dogfood transaction journal path for start backup");
    }
    let backup_name = backup
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_suffix("-start.brainmap.tar.zst"))
        .context("unsafe dogfood transaction journal path for start backup")?;
    let bundle_parts = bundle
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>();
    if bundle_parts.len() != 4
        || bundle_parts[0] != ".brainmap"
        || bundle_parts[1] != "dogfood"
        || bundle_parts[3] != "qualification"
        || bundle_parts[2] != backup_name
    {
        bail!("unsafe dogfood transaction journal path for qualification bundle");
    }
    validate_safe_label("dogfood transaction run id", backup_name)?;
    if let Some(run) = &transaction.intended_run
        && run.run_id != backup_name
    {
        bail!("invalid dogfood start transaction journal run ID");
    }
    Ok(())
}

fn validate_internal_transaction_path(root: &Path, label: &str, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("unsafe dogfood transaction journal path for {label}");
    }
    let candidate = root.join(relative);
    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("resolve dogfood vault root {}", root.display()))?;
    let mut existing = candidate.as_path();
    let canonical_existing = loop {
        match fs::canonicalize(existing) {
            Ok(path) => break path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .context("unsafe dogfood transaction journal path without an ancestor")?;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("validate dogfood transaction journal {label}"));
            }
        }
    };
    if !canonical_existing.starts_with(&canonical_root) {
        bail!("unsafe dogfood transaction journal path for {label}");
    }
    Ok(relative.to_path_buf())
}

fn validate_finalize_transaction_paths(
    root: &Path,
    transaction: &FinalizeTransaction,
) -> Result<()> {
    let out_parent = transaction
        .out
        .parent()
        .context("unsafe dogfood transaction journal path for evidence output")?;
    if !transaction.out.is_absolute()
        || !transaction.staging.is_absolute()
        || transaction.staging.parent() != Some(out_parent)
        || fs::canonicalize(out_parent).ok().as_deref() != Some(out_parent)
    {
        bail!("unsafe dogfood transaction journal path for evidence output");
    }
    let out_name = transaction
        .out
        .file_name()
        .and_then(|value| value.to_str())
        .context("unsafe dogfood transaction journal path for evidence output")?;
    let staging_name = transaction
        .staging
        .file_name()
        .and_then(|value| value.to_str())
        .context("unsafe dogfood transaction journal path for evidence staging")?;
    let staging_id = staging_name
        .strip_prefix(&format!(".{out_name}."))
        .and_then(|value| value.strip_suffix(".tmp"))
        .filter(|value| value.starts_with("evidence_"))
        .context("unsafe dogfood transaction journal path for evidence staging")?;
    validate_safe_label("dogfood transaction staging ID", staging_id)?;

    let canonical_root = fs::canonicalize(root)
        .with_context(|| format!("resolve dogfood vault root {}", root.display()))?;
    if transaction.out == transaction.staging
        || transaction.out.starts_with(&canonical_root)
        || canonical_root.starts_with(&transaction.out)
        || transaction.staging.starts_with(&canonical_root)
        || canonical_root.starts_with(&transaction.staging)
    {
        bail!("unsafe dogfood transaction journal path overlapping the vault");
    }
    Ok(())
}

fn clear_transaction(root: &Path) -> Result<()> {
    util::remove_file_and_sync(&transaction_path(root))
}

fn restore_previous_state(root: &Path, existed: bool, previous: &DogfoodState) -> Result<()> {
    if existed {
        write_state(root, previous)
    } else {
        util::remove_file_and_sync(&state_path(root))
    }
}

fn start_activation_valid(root: &Path, transaction: &StartTransaction) -> bool {
    let Some(intended) = transaction.intended_run.as_ref() else {
        return false;
    };
    let state_has_active = load_state(root).is_ok_and(|state| {
        state
            .runs
            .iter()
            .any(|run| run == intended && run.status == DogfoodRunStatus::Active)
    });
    state_has_active
        && verify_start_backup(root, intended).is_ok()
        && verify_qualification_provenance(root, intended).is_ok()
        && learning::gate_mode_config(root) == intended.gate_mode
        && {
            let autopilot = learning::autopilot_config(root);
            autopilot.mode == intended.autopilot_mode
                && autopilot.level == intended.autopilot_level
                && (autopilot.threshold - intended.threshold).abs() <= f64::EPSILON
        }
}

fn rollback_start_transaction(root: &Path, transaction: &StartTransaction) -> Result<()> {
    restore_previous_state(
        root,
        transaction.previous_state_existed,
        &transaction.previous_state,
    )?;
    transaction
        .prior_autopilot
        .restore(&root.join(".brainmap/autopilot.json"))?;
    transaction
        .prior_gate_mode
        .restore(&root.join(".brainmap/gate-mode"))?;
    if transaction.phase != StartPhase::Prepared {
        util::remove_file_and_sync(&root.join(&transaction.backup_relative_path))?;
    }
    let bundle = root.join(&transaction.qualification_bundle_relative_path);
    if matches!(
        transaction.phase,
        StartPhase::QualificationDirectoryPrepared
            | StartPhase::QualificationBundleCopied
            | StartPhase::IntentRecorded
            | StartPhase::AutopilotWritten
            | StartPhase::GateModeWritten
    ) {
        let run_directory = bundle
            .parent()
            .context("dogfood qualification bundle has no run directory")?;
        util::remove_dir_all_and_sync(run_directory)?;
        remove_empty_parents(run_directory, &root.join(".brainmap/dogfood"));
    }
    clear_transaction(root)
}

fn verify_evidence_bundle(
    root: &Path,
    directory: &Path,
    expected_run: &DogfoodRunState,
) -> Result<ChecksummedArtifact> {
    let metadata = fs::symlink_metadata(directory)
        .with_context(|| format!("inspect dogfood evidence directory {}", directory.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("dogfood evidence directory is missing");
    }
    let expected_top_level = [
        "SHA256SUMS",
        "dogfood-qualification.json",
        "dogfood-qualification.md",
        "qualification",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    let actual = fs::read_dir(directory)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("evidence filename is not UTF-8"))
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if actual != expected_top_level {
        bail!("dogfood evidence bundle has missing or unexpected files");
    }
    let actual_files = collect_evidence_files(directory)?;
    let checksum_text = fs::read_to_string(directory.join("SHA256SUMS"))?;
    if !checksum_text.ends_with('\n') || checksum_text.lines().any(str::is_empty) {
        bail!("invalid dogfood SHA256SUMS framing");
    }
    let mut checked = BTreeSet::new();
    let mut previous = None::<String>;
    for line in checksum_text.lines() {
        let (expected_sha, name) = line
            .split_once("  ")
            .context("invalid dogfood SHA256SUMS line")?;
        validate_sha256("dogfood evidence checksum", expected_sha)?;
        validate_evidence_relative_path(name)?;
        if previous.as_deref().is_some_and(|value| value >= name) {
            bail!("dogfood SHA256SUMS entries must be strictly sorted");
        }
        previous = Some(name.to_string());
        if name == "SHA256SUMS" || !checked.insert(name.to_string()) {
            bail!("invalid or duplicate dogfood checksum entry");
        }
        if file_sha256(&directory.join(name))? != expected_sha {
            bail!("dogfood evidence checksum mismatch for {name}");
        }
    }
    let expected_coverage = actual_files
        .iter()
        .filter(|name| name.as_str() != "SHA256SUMS")
        .cloned()
        .collect::<BTreeSet<_>>();
    if checked != expected_coverage {
        bail!("dogfood checksum coverage is incomplete");
    }
    let final_export = verify_private_final_export(root, expected_run)?;
    let qualification_path = directory.join("qualification");
    let verified = qualification::verify_bundle(&qualification_path)
        .context("verify qualification tree in dogfood evidence")?;
    if verified.bundle_sha256 != expected_run.qualification_bundle_sha256
        || file_sha256(&qualification_path.join("qualification.json"))?
            != expected_run.qualification_manifest_sha256
        || verified.candidate.commit != expected_run.candidate_commit
        || verified.candidate.brainmap_sha256 != expected_run.candidate_binary_sha256
        || verified.candidate.brainmapd_sha256 != expected_run.candidate_brainmapd_sha256
    {
        bail!("dogfood evidence qualification provenance does not match the run");
    }
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("dogfood-qualification.json"))?)?;
    let report_qualification = report.get("qualification");
    if report
        .get("schemaVersion")
        .and_then(serde_json::Value::as_str)
        != Some("brainmap-dogfood-qualification-v3")
        || report.get("status").and_then(serde_json::Value::as_str) != Some("passed")
        || report.get("plannedEnd").is_some()
        || report
            .get("rawPromptsRetained")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
        || report
            .get("shadowMetrics")
            .and_then(|value| value.get("intensiveSessionDistributionValid"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || report
            .get("reviewSummary")
            .and_then(|value| value.get("finalReviewCoversLedger"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || report
            .get("reviewSummary")
            .and_then(|value| value.get("reviewReady"))
            .and_then(serde_json::Value::as_bool)
            != Some(true)
        || report.get("runId").and_then(serde_json::Value::as_str)
            != Some(expected_run.run_id.as_str())
        || report
            .get("candidateCommit")
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.candidate_commit.as_str())
        || report
            .get("candidateBinarySha256")
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.candidate_binary_sha256.as_str())
        || report
            .get("candidateBrainmapdSha256")
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.candidate_brainmapd_sha256.as_str())
        || report
            .get("startBackup")
            .and_then(|value| value.get("relativePath"))
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.start_backup.relative_path.as_str())
        || report
            .get("startBackup")
            .and_then(|value| value.get("sha256"))
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.start_backup.sha256.as_str())
        || report_qualification
            .and_then(|value| value.get("relativePath"))
            .and_then(serde_json::Value::as_str)
            != Some("qualification")
        || report_qualification
            .and_then(|value| value.get("runRelativePath"))
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.qualification_bundle_relative_path.as_str())
        || report_qualification
            .and_then(|value| value.get("bundleSha256"))
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.qualification_bundle_sha256.as_str())
        || report_qualification
            .and_then(|value| value.get("manifestSha256"))
            .and_then(serde_json::Value::as_str)
            != Some(expected_run.qualification_manifest_sha256.as_str())
        || report
            .get("finalBackupSha256")
            .and_then(serde_json::Value::as_str)
            != Some(final_export.sha256.as_str())
        || report.get("finalExport").is_some()
    {
        bail!("dogfood qualification report does not identify a passed expected run");
    }
    let report_metrics = report
        .get("shadowMetrics")
        .context("dogfood qualification report is missing shadowMetrics")?;
    let validated_safety = validate_metrics(report_metrics, &expected_run.run_id)
        .context("revalidate dogfood qualification report metrics")?;
    let reported_safety: SafetyQualification = serde_json::from_value(
        report
            .get("safety")
            .cloned()
            .context("dogfood qualification report is missing safety")?,
    )
    .context("parse dogfood qualification report safety")?;
    if reported_safety != validated_safety {
        bail!("dogfood qualification report safety does not match its shadow metrics");
    }
    let report_review = report
        .get("reviewSummary")
        .context("dogfood qualification report is missing reviewSummary")?;
    let review_count = report_review
        .get("reviewCount")
        .and_then(serde_json::Value::as_u64)
        .context("dogfood qualification report has no review count")?;
    let final_incident_status = report_review
        .get("currentIncidentStatus")
        .and_then(serde_json::Value::as_str)
        .context("dogfood qualification report has no final incident status")?;
    if review_count == 0 || !matches!(final_incident_status, "clear" | "resolved-no-violation") {
        bail!("dogfood qualification report does not contain a qualifying final review");
    }
    let complete_pairs = metric_count(report_metrics, "completeGateActionPairs")?;
    let decision_scenarios = metric_count(report_metrics, "distinctDecisionScenarios")?;
    let scopes = metric_count(report_metrics, "distinctScopes")?;
    let decision_types = metric_count(report_metrics, "distinctDecisionTypes")?;
    let markdown = fs::read_to_string(directory.join("dogfood-qualification.md"))?;
    for expected in [
        format!("- Run ID: `{}`", expected_run.run_id),
        format!("- Candidate commit: `{}`", expected_run.candidate_commit),
        format!(
            "- Candidate binary SHA-256: `{}`",
            expected_run.candidate_binary_sha256
        ),
        format!(
            "- Candidate brainmapd SHA-256: `{}`",
            expected_run.candidate_brainmapd_sha256
        ),
        format!(
            "- Qualification run path: `{}`",
            expected_run.qualification_bundle_relative_path
        ),
        format!(
            "- Qualification bundle SHA-256: `{}`",
            expected_run.qualification_bundle_sha256
        ),
        format!(
            "- Qualification manifest SHA-256: `{}`",
            expected_run.qualification_manifest_sha256
        ),
        format!("- Complete gate/action pairs: {complete_pairs}"),
        format!("- Distinct decision scenarios: {decision_scenarios}"),
        format!("- Distinct scopes: {scopes}"),
        format!("- Distinct decision types: {decision_types}"),
        format!("- Reviews: {review_count}"),
        format!("- Final incident state: `{final_incident_status}`"),
        "- Final review covers ledger: true".into(),
        "- Review ready: true".into(),
    ] {
        if !markdown.contains(&expected) {
            bail!("dogfood qualification Markdown is missing required provenance or summary");
        }
    }
    Ok(final_export)
}

fn final_export_relative_path(run_id: &str) -> String {
    format!("99-meta/backups/{run_id}-final.brainmap.tar.zst")
}

fn verify_private_final_export(
    root: &Path,
    expected_run: &DogfoodRunState,
) -> Result<ChecksummedArtifact> {
    let relative_path = final_export_relative_path(&expected_run.run_id);
    validate_internal_transaction_path(root, "final dogfood backup", &relative_path)?;
    let path = root.join(&relative_path);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("inspect private final dogfood backup {}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("private final dogfood backup is not a regular file");
    }
    export::verify_export_archive(&path, None)?;
    Ok(ChecksummedArtifact {
        relative_path,
        sha256: file_sha256(&path)?,
    })
}

fn reconcile_pending_transaction_locked(root: &Path) -> Result<ReconcileOutcome> {
    let Some(transaction) = load_transaction(root)? else {
        return Ok(ReconcileOutcome::None);
    };
    match transaction {
        DogfoodTransaction::Start(transaction) => {
            if start_activation_valid(root, &transaction) {
                clear_transaction(root)?;
                Ok(ReconcileOutcome::StartActivated)
            } else {
                rollback_start_transaction(root, &transaction)?;
                Ok(ReconcileOutcome::RolledBack)
            }
        }
        DogfoodTransaction::Finalize(mut transaction) => {
            let expected_run = &transaction.archived_state.runs[transaction.active_index];
            let source_qualification_valid =
                verify_qualification_provenance(root, expected_run).is_ok();
            let mut complete = source_qualification_valid
                .then(|| verify_evidence_bundle(root, &transaction.out, expected_run).ok())
                .flatten();
            if complete.is_none()
                && source_qualification_valid
                && !transaction.out.exists()
                && verify_evidence_bundle(root, &transaction.staging, expected_run).is_ok()
            {
                util::rename_and_sync(&transaction.staging, &transaction.out)?;
                complete = verify_evidence_bundle(root, &transaction.out, expected_run).ok();
            }
            if let Some(final_export) = complete {
                transaction.archived_state.runs[transaction.active_index].final_export =
                    Some(final_export);
                write_state(root, &transaction.archived_state)?;
                util::remove_dir_all_and_sync(&transaction.staging)?;
                clear_transaction(root)?;
                Ok(ReconcileOutcome::Finalized)
            } else {
                util::remove_dir_all_and_sync(&transaction.staging)?;
                util::remove_dir_all_and_sync(&transaction.out)?;
                util::remove_file_and_sync(
                    &root.join(final_export_relative_path(&transaction.run_id)),
                )?;
                write_state(root, &transaction.original_state)?;
                clear_transaction(root)?;
                Ok(ReconcileOutcome::RolledBack)
            }
        }
    }
}

fn inject_start_fault(fault: Option<StartFault>, boundary: StartFault) -> Result<()> {
    if fault == Some(boundary) {
        bail!("injected dogfood start crash at {boundary:?}");
    }
    Ok(())
}

fn inject_finalize_fault(fault: Option<FinalizeFault>, boundary: FinalizeFault) -> Result<()> {
    if fault == Some(boundary) {
        bail!("injected dogfood finalize crash at {boundary:?}");
    }
    Ok(())
}

fn canonical_json_bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
    fn write(value: &serde_json::Value, out: &mut Vec<u8>) -> Result<()> {
        match value {
            serde_json::Value::Null
            | serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => out.extend(serde_json::to_vec(value)?),
            serde_json::Value::Array(values) => {
                out.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    write(value, out)?;
                }
                out.push(b']');
            }
            serde_json::Value::Object(values) => {
                out.push(b'{');
                let mut entries = values.iter().collect::<Vec<_>>();
                entries.sort_by_key(|(left, _)| *left);
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    out.extend(serde_json::to_vec(key)?);
                    out.push(b':');
                    write(value, out)?;
                }
                out.push(b'}');
            }
        }
        Ok(())
    }
    let mut bytes = Vec::new();
    write(value, &mut bytes)?;
    Ok(bytes)
}

fn review_safety_counters(metrics: &serde_json::Value) -> Result<ReviewSafetyCounters> {
    Ok(ReviewSafetyCounters {
        false_proceeds: metric_count(metrics, "falseProceeds")?,
        confirmed_cross_domain_applications: metric_count(
            metrics,
            "confirmedCrossDomainApplications",
        )?,
        privacy_violations: metric_count(metrics, "privacyViolations")?,
        hard_rule_violations: metric_count(metrics, "hardRuleViolations")?,
    })
}

fn incident_transition_valid(previous: Option<IncidentStatus>, next: IncidentStatus) -> bool {
    !matches!(
        (previous, next),
        (Some(IncidentStatus::CandidateFailed), _)
            | (None, IncidentStatus::ResolvedNoViolation)
            | (
                Some(IncidentStatus::Clear),
                IncidentStatus::ResolvedNoViolation
            )
            | (Some(IncidentStatus::Investigating), IncidentStatus::Clear)
    )
}

fn validate_review_receipts(
    root: &Path,
    run: &DogfoodRunState,
    ledger: &[u8],
    ended_at: DateTime<Utc>,
) -> Result<ReviewSummary> {
    let boundary = usize::try_from(run.ledger_boundary_bytes)
        .context("dogfood review ledger boundary does not fit this platform")?;
    if boundary > ledger.len()
        || util::sha256_hex(&ledger[..boundary]) != run.ledger_boundary_sha256
    {
        bail!("dogfood review validation detected ledger boundary drift");
    }
    let mut timestamps = Vec::new();
    let mut current_status = None;
    let mut seen_ids = BTreeSet::new();
    let mut offset = 0usize;
    let mut last_review_segment_end = None;
    for segment in ledger.split_inclusive(|byte| *byte == b'\n') {
        let prefix_end = offset;
        offset += segment.len();
        if prefix_end < boundary {
            continue;
        }
        let line = segment.strip_suffix(b"\n").unwrap_or(segment);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let value: serde_json::Value = serde_json::from_slice(line)
            .context("parse dogfood ledger while validating review receipts")?;
        if value.get("kind").and_then(serde_json::Value::as_str) != Some("dogfood-review") {
            continue;
        }
        let receipt: DogfoodReviewReceipt =
            serde_json::from_value(value).context("parse strict dogfood review receipt")?;
        if receipt.schema_version != REVIEW_SCHEMA || receipt.kind != "dogfood-review" {
            bail!("unsupported dogfood review receipt schema or kind");
        }
        util::validate_safe_component("dogfood review id", &receipt.id)?;
        if !seen_ids.insert(receipt.id.clone()) {
            bail!("duplicate dogfood review receipt id");
        }
        if receipt.dogfood_run_id != run.run_id {
            bail!("dogfood review receipt belongs to another run");
        }
        if receipt.ledger_prefix_bytes != prefix_end as u64
            || receipt.ledger_prefix_lines != nonempty_line_count(&ledger[..prefix_end])?
            || receipt.ledger_prefix_sha256 != util::sha256_hex(&ledger[..prefix_end])
        {
            bail!("dogfood review receipt ledger prefix integrity failed");
        }
        let timestamp = parse_timestamp("dogfood review createdAt", &receipt.created_at)?;
        if receipt.created_at != timestamp.to_rfc3339_opts(SecondsFormat::Millis, true) {
            bail!("dogfood review createdAt must be canonical millisecond UTC");
        }
        if timestamp < run.started_at || timestamp > ended_at {
            bail!("dogfood review timestamp is outside the qualification interval");
        }
        if timestamps
            .last()
            .is_some_and(|previous| timestamp <= *previous)
        {
            bail!("dogfood review timestamps must be strictly increasing");
        }
        if !incident_transition_valid(current_status, receipt.incident_status) {
            bail!("invalid dogfood review incident-state transition");
        }
        let metrics = learning::shadow_metrics_value_from_locked_ledger_at(
            root,
            timestamp,
            &ledger[..prefix_end],
        )?;
        if receipt.shadow_metrics_sha256 != util::sha256_hex(&canonical_json_bytes(&metrics)?)
            || receipt.safety != review_safety_counters(&metrics)?
        {
            bail!("dogfood review receipt metrics or safety binding failed");
        }
        current_status = Some(receipt.incident_status);
        timestamps.push(timestamp);
        last_review_segment_end = Some(offset);
    }

    let candidate_failed = current_status == Some(IncidentStatus::CandidateFailed);
    let unresolved_investigation = current_status == Some(IncidentStatus::Investigating);
    let final_review_covers_ledger = last_review_segment_end
        .is_some_and(|end| ledger[end..].iter().all(u8::is_ascii_whitespace));
    let review_ready = !timestamps.is_empty()
        && final_review_covers_ledger
        && !candidate_failed
        && !unresolved_investigation
        && matches!(
            current_status,
            Some(IncidentStatus::Clear | IncidentStatus::ResolvedNoViolation)
        );
    Ok(ReviewSummary {
        schema_version: REVIEW_SCHEMA,
        review_count: timestamps.len(),
        first_review_at: timestamps
            .first()
            .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)),
        last_review_at: timestamps
            .last()
            .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)),
        current_incident_status: current_status,
        candidate_failed,
        unresolved_investigation,
        integrity_valid: true,
        final_review_covers_ledger,
        review_ready,
    })
}

pub(crate) fn start(args: DogfoodStartArgs) -> Result<()> {
    start_at_with_fault(args, Utc::now(), None)
}

fn start_at_with_fault(
    args: DogfoodStartArgs,
    now: DateTime<Utc>,
    fault: Option<StartFault>,
) -> Result<()> {
    start_at_with_verifier(
        args,
        now,
        fault,
        qualification::verify_running_qualification,
    )
}

#[cfg(test)]
fn start_at_with_test_verifier(
    args: DogfoodStartArgs,
    now: DateTime<Utc>,
    fault: Option<StartFault>,
) -> Result<()> {
    start_at_with_verifier(args, now, fault, |verified| {
        let running = qualification::running_candidate_hashes()?;
        ensure!(
            running.brainmap_sha256 == verified.candidate.brainmap_sha256
                && running.brainmapd_sha256 == verified.candidate.brainmapd_sha256,
            "test qualification candidate hashes do not match the running binaries"
        );
        Ok(running)
    })
}

fn start_at_with_verifier(
    args: DogfoodStartArgs,
    now: DateTime<Utc>,
    fault: Option<StartFault>,
    verify_running: impl FnOnce(
        &qualification::VerifiedQualification,
    ) -> Result<qualification::RunningCandidateHashes>,
) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    ensure_initialized_vault(&root)?;
    let mut ledger_lock = util::lock_jsonl(&root.join(LEDGER_RELATIVE_PATH))?;
    let recovered_start =
        reconcile_pending_transaction_locked(&root)? == ReconcileOutcome::StartActivated;
    validate_commit(&args.candidate_commit)?;
    if args.adapter != "codex" {
        bail!("strict dogfood qualification requires --adapter codex");
    }
    let started_at = parse_timestamp("started-at", &args.started_at)?;
    if (now - started_at).abs() > START_CLOCK_TOLERANCE {
        bail!("started-at must be within five minutes of the current UTC time");
    }

    let qualification = qualification::verify_bundle(&args.qualification_bundle)
        .context("verify dogfood qualification bundle")?;
    if qualification.candidate.commit != args.candidate_commit {
        bail!("qualification candidate commit does not match --candidate-commit");
    }
    let identity_before_hash = current_binary_identity()?;
    let running_candidate = verify_running(&qualification)?;
    let candidate_binary_sha256 = running_candidate.brainmap_sha256;
    let candidate_brainmapd_sha256 = running_candidate.brainmapd_sha256;
    let candidate_binary_identity = current_binary_identity()?;
    if identity_before_hash != candidate_binary_identity {
        bail!("running Brainmap binary changed while its candidate checksum was calculated");
    }
    let mut state = load_state(&root)?;
    if recovered_start {
        let recovered = state
            .runs
            .iter()
            .find(|run| run.status == DogfoodRunStatus::Active)
            .context("recovered dogfood start has no active run")?;
        if recovered.candidate_commit == args.candidate_commit
            && recovered.adapter == args.adapter
            && recovered.started_at == started_at
            && recovered.qualification_bundle_sha256 == qualification.bundle_sha256
            && recovered.candidate_binary_sha256 == candidate_binary_sha256
            && recovered.candidate_brainmapd_sha256 == candidate_brainmapd_sha256
        {
            println!("dogfood run {} recovered in shadow mode", recovered.run_id);
            return Ok(());
        }
        bail!("recovered dogfood start conflicts with the requested run provenance");
    }
    if state
        .runs
        .iter()
        .any(|run| run.status == DogfoodRunStatus::Active)
    {
        bail!("an active dogfood run already exists; finalize or abort it first");
    }
    let ledger = ledger_lock.read_all()?;
    let run_id = util::id("dogfood", &args.candidate_commit);
    let backup_relative_path = format!("99-meta/backups/{run_id}-start.brainmap.tar.zst");
    let backup_path = root.join(&backup_relative_path);
    let qualification_bundle_relative_path = format!(".brainmap/dogfood/{run_id}/qualification");
    let qualification_bundle_path = root.join(&qualification_bundle_relative_path);
    let autopilot_path = root.join(".brainmap/autopilot.json");
    let gate_mode_path = root.join(".brainmap/gate-mode");
    let autopilot_snapshot = FileSnapshot::capture(&autopilot_path)?;
    let gate_mode_snapshot = FileSnapshot::capture(&gate_mode_path)?;
    let prior_autopilot = learning::autopilot_config(&root);
    let threshold = prior_autopilot.threshold;

    let mut journal = StartTransaction {
        phase: StartPhase::Prepared,
        previous_state_existed: state_path(&root).exists(),
        previous_state: state.clone(),
        prior_autopilot: autopilot_snapshot,
        prior_gate_mode: gate_mode_snapshot,
        backup_relative_path: backup_relative_path.clone(),
        qualification_bundle_relative_path: qualification_bundle_relative_path.clone(),
        intended_run: None,
    };
    let transaction = (|| -> Result<DogfoodRunState> {
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::JournalPrepared)?;

        export::export_portable_snapshot(&root, &backup_path)?;
        util::sync_file(&backup_path)?;
        export::verify_export_archive(&backup_path, None)?;
        journal.phase = StartPhase::BackupCreated;
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::BackupCreated)?;

        let start_backup = ChecksummedArtifact {
            relative_path: backup_relative_path,
            sha256: file_sha256(&backup_path)?,
        };

        let qualification_parent = qualification_bundle_path
            .parent()
            .context("dogfood qualification bundle has no run directory")?;
        let qualification_root = qualification_parent
            .parent()
            .context("dogfood qualification run directory has no parent")?;
        fs::create_dir_all(qualification_root).with_context(|| {
            format!(
                "create dogfood qualification root {}",
                qualification_root.display()
            )
        })?;
        fs::create_dir(qualification_parent).with_context(|| {
            format!(
                "reserve dogfood qualification run directory {}",
                qualification_parent.display()
            )
        })?;
        util::sync_directory(qualification_parent)?;
        util::sync_directory(qualification_root)?;
        journal.phase = StartPhase::QualificationDirectoryPrepared;
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::QualificationDirectoryPrepared)?;

        let copied_qualification = qualification::copy_verified_bundle(
            &args.qualification_bundle,
            &qualification_bundle_path,
        )?;
        ensure!(
            copied_qualification.candidate == qualification.candidate
                && copied_qualification.bundle_sha256 == qualification.bundle_sha256,
            "qualification bundle changed between dogfood preflight and preservation"
        );
        let qualification_manifest_sha256 =
            file_sha256(&qualification_bundle_path.join("qualification.json"))?;
        journal.phase = StartPhase::QualificationBundleCopied;
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::QualificationBundleCopied)?;

        let run = DogfoodRunState {
            run_id,
            status: DogfoodRunStatus::Active,
            candidate_commit: args.candidate_commit,
            candidate_binary_sha256,
            candidate_brainmapd_sha256,
            candidate_binary_identity,
            host: HostProvenance {
                os: std::env::consts::OS.into(),
                arch: std::env::consts::ARCH.into(),
            },
            adapter: args.adapter,
            started_at,
            mode: "shadow".into(),
            gate_mode: "shadow".into(),
            autopilot_mode: "shadow".into(),
            autopilot_level: "conservative".into(),
            threshold,
            start_backup,
            qualification_bundle_sha256: copied_qualification.bundle_sha256,
            qualification_manifest_sha256,
            qualification_bundle_relative_path,
            ledger_boundary_bytes: ledger.len() as u64,
            ledger_boundary_lines: nonempty_line_count(&ledger)?,
            ledger_boundary_sha256: util::sha256_hex(&ledger),
            aborted_at: None,
            abort_reason: None,
            finalized_at: None,
            final_export: None,
            sign_off: None,
        };
        journal.phase = StartPhase::IntentRecorded;
        journal.intended_run = Some(run.clone());
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::IntentRecorded)?;

        learning::write_autopilot_config(&root, "shadow", "conservative", threshold)?;
        journal.phase = StartPhase::AutopilotWritten;
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::AutopilotWritten)?;

        learning::write_gate_mode_config(&root, "shadow")?;
        journal.phase = StartPhase::GateModeWritten;
        write_transaction(&root, DogfoodTransaction::Start(Box::new(journal.clone())))?;
        inject_start_fault(fault, StartFault::GateModeWritten)?;

        state.runs.push(run.clone());
        write_state(&root, &state)?;
        inject_start_fault(fault, StartFault::StateActivated)?;
        clear_transaction(&root)?;
        Ok(run)
    })();
    let run = match transaction {
        Ok(run) => run,
        Err(error) => {
            if fault.is_some() {
                return Err(error);
            }
            return match reconcile_pending_transaction_locked(&root) {
                Ok(ReconcileOutcome::StartActivated) => {
                    let run = load_state(&root)?
                        .runs
                        .into_iter()
                        .find(|run| run.status == DogfoodRunStatus::Active)
                        .context("recovered start did not retain its active run")?;
                    println!("dogfood run {} recovered in shadow mode", run.run_id);
                    Ok(())
                }
                Ok(_) => Err(error),
                Err(recovery) => {
                    Err(error).context(format!("dogfood start recovery also failed: {recovery:#}"))
                }
            };
        }
    };
    println!("dogfood run {} started in shadow mode", run.run_id);
    Ok(())
}

pub(crate) fn status(vault_path: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault_path);
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    let mut ledger_lock = util::lock_jsonl(&root.join(LEDGER_RELATIVE_PATH))?;
    reconcile_pending_transaction_locked(&root)?;
    let ledger = ledger_lock.read_all()?;
    let state = load_state(&root)?;
    if let Some(run) = state.runs.last() {
        let now = Utc::now();
        let mut value = serde_json::to_value(run)?;
        let object = value
            .as_object_mut()
            .context("serialized dogfood run is not an object")?;
        object.insert(
            "elapsedSeconds".into(),
            serde_json::json!((now - run.started_at).num_seconds().max(0)),
        );
        let mut healthy = true;
        if run.status == DogfoodRunStatus::Active {
            let metrics = learning::shadow_metrics_value_from_locked_ledger_at(&root, now, &ledger);
            let reviews = validate_review_receipts(&root, run, &ledger, now);
            let health = run_health(
                &root,
                run,
                metrics.as_ref().ok(),
                reviews.as_ref().ok(),
                &ledger,
            );
            healthy = health.healthy;
            object.insert(
                "shadowMetrics".into(),
                metrics.unwrap_or(serde_json::Value::Null),
            );
            object.insert(
                "reviewSummary".into(),
                match &reviews {
                    Ok(summary) => serde_json::to_value(summary)?,
                    Err(_) => serde_json::Value::Null,
                },
            );
            object.insert("health".into(), serde_json::to_value(&health)?);
            let context = active_run_context(&root)?.context("active run context is missing")?;
            object.insert(
                "activeContext".into(),
                serde_json::json!({
                    "runId": context.run_id,
                    "startedAt": context.started_at,
                    "ledgerBoundaryBytes": context.ledger_boundary_bytes,
                    "ledgerBoundaryLines": context.ledger_boundary_lines,
                    "ledgerBoundarySha256": context.ledger_boundary_sha256,
                    "mode": context.mode,
                    "gateMode": context.gate_mode,
                    "autopilotMode": context.autopilot_mode,
                    "autopilotLevel": context.autopilot_level,
                    "threshold": context.threshold,
                    "candidateCommit": context.candidate_commit,
                    "candidateBinarySha256": context.candidate_binary_sha256
                }),
            );
        } else {
            verify_qualification_provenance(&root, run)
                .context("verify retained dogfood qualification bundle during status")?;
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
        if !healthy {
            bail!(
                "active dogfood run failed a provenance, mode, ledger, metrics, or review health check"
            );
        }
    } else {
        println!(r#"{{"status":"not_started"}}"#);
    }
    Ok(())
}

pub(crate) fn active_run_context(root: &Path) -> Result<Option<DogfoodRunContext>> {
    let state = match load_transaction(root)? {
        Some(DogfoodTransaction::Finalize(transaction)) => {
            if matches!(
                transaction.phase,
                FinalizePhase::ChecksumsWritten
                    | FinalizePhase::EvidenceActivated
                    | FinalizePhase::FinalStateWritten
            ) {
                bail!("dogfood finalization recovery is required before recording more events");
            }
            transaction.original_state
        }
        Some(DogfoodTransaction::Start(transaction)) => {
            let live = load_state(root)?;
            if live.runs.iter().any(|run| {
                transaction
                    .intended_run
                    .as_ref()
                    .is_some_and(|intended| intended.run_id == run.run_id)
                    && run.status == DogfoodRunStatus::Active
            }) {
                live
            } else {
                transaction.previous_state
            }
        }
        None => load_state(root)?,
    };
    let mut active = state
        .runs
        .iter()
        .filter(|run| run.status == DogfoodRunStatus::Active);
    let Some(run) = active.next() else {
        return Ok(None);
    };
    if active.next().is_some() {
        bail!("dogfood state contains multiple active runs");
    }
    Ok(Some(DogfoodRunContext {
        run_id: run.run_id.clone(),
        started_at: run.started_at,
        ledger_boundary_bytes: run.ledger_boundary_bytes,
        ledger_boundary_lines: run.ledger_boundary_lines,
        ledger_boundary_sha256: run.ledger_boundary_sha256.clone(),
        mode: run.mode.clone(),
        gate_mode: run.gate_mode.clone(),
        autopilot_mode: run.autopilot_mode.clone(),
        autopilot_level: run.autopilot_level.clone(),
        threshold: run.threshold,
        candidate_commit: run.candidate_commit.clone(),
        candidate_binary_sha256: run.candidate_binary_sha256.clone(),
        candidate_binary_identity: run.candidate_binary_identity.clone(),
    }))
}

pub(crate) fn active_run_context_for_gate(root: &Path) -> Result<Option<DogfoodRunContext>> {
    for _ in 0..3 {
        let before = gate_context_version(root)?;
        let cached = gate_context_cache()
            .lock()
            .expect("dogfood gate context cache lock")
            .get(root)
            .cloned();
        if let Some((version, context)) = cached
            && version == before
        {
            return Ok(context);
        }

        #[cfg(test)]
        GATE_CONTEXT_LOADS.fetch_add(1, Ordering::Relaxed);
        let context = active_run_context(root)?;
        let after = gate_context_version(root)?;
        if before != after {
            continue;
        }
        gate_context_cache()
            .lock()
            .expect("dogfood gate context cache lock")
            .insert(root.to_path_buf(), (after, context.clone()));
        return Ok(context);
    }
    bail!("dogfood state changed repeatedly while loading gate context")
}

fn gate_context_cache() -> &'static Mutex<GateContextCache> {
    static CACHE: OnceLock<Mutex<GateContextCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn gate_context_version(root: &Path) -> Result<GateContextVersion> {
    Ok(GateContextVersion {
        state: gate_file_version(&state_path(root))?,
        transaction: gate_file_version(&transaction_path(root))?,
    })
}

fn gate_file_version(path: &Path) -> Result<Option<GateFileVersion>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "dogfood gate state is not a regular file: {}",
        path.display()
    );
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Some(GateFileVersion {
            len: metadata.len(),
            modified_unix_nanos,
            device: metadata.dev(),
            inode: metadata.ino(),
            changed_unix_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        }))
    }
    #[cfg(not(unix))]
    {
        Ok(Some(GateFileVersion {
            len: metadata.len(),
            modified_unix_nanos,
        }))
    }
}

pub(crate) fn review(args: DogfoodReviewArgs) -> Result<()> {
    review_at(args, Utc::now())
}

fn review_at(args: DogfoodReviewArgs, now: DateTime<Utc>) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    let mut ledger_lock = util::lock_jsonl(&root.join(LEDGER_RELATIVE_PATH))?;
    reconcile_pending_transaction_locked(&root)?;
    let state = load_state(&root)?;
    let active_index = active_run_index(&state)?;
    let run = &state.runs[active_index];
    let ledger = ledger_lock.read_all()?;
    verify_run_provenance(&root, run, &ledger)?;
    let existing = validate_review_receipts(&root, run, &ledger, now)?;
    let incident_status = IncidentStatus::parse(&args.incident_status)?;
    if !incident_transition_valid(existing.current_incident_status, incident_status) {
        bail!("invalid dogfood review incident-state transition");
    }
    let created_at = now.to_rfc3339_opts(SecondsFormat::Millis, true);
    let receipt_at = parse_timestamp("dogfood review time", &created_at)?;
    if receipt_at < run.started_at {
        bail!("dogfood review timestamp cannot precede the active run");
    }
    if let Some(last_review_at) = existing.last_review_at.as_deref() {
        let last_review_at = parse_timestamp("last dogfood review time", last_review_at)?;
        if receipt_at <= last_review_at {
            bail!("dogfood review timestamps must be strictly increasing");
        }
    }
    let metrics = learning::shadow_metrics_value_from_locked_ledger_at(&root, receipt_at, &ledger)?;
    let receipt = DogfoodReviewReceipt {
        schema_version: REVIEW_SCHEMA.into(),
        kind: "dogfood-review".into(),
        id: util::id("review", &format!("{}:{created_at}", run.run_id)),
        dogfood_run_id: run.run_id.clone(),
        created_at,
        incident_status,
        ledger_prefix_bytes: ledger.len() as u64,
        ledger_prefix_lines: nonempty_line_count(&ledger)?,
        ledger_prefix_sha256: util::sha256_hex(&ledger),
        shadow_metrics_sha256: util::sha256_hex(&canonical_json_bytes(&metrics)?),
        safety: review_safety_counters(&metrics)?,
    };
    ledger_lock.append(&serde_json::to_value(&receipt)?)?;
    let final_ledger = ledger_lock.read_all()?;
    validate_review_receipts(&root, run, &final_ledger, receipt_at)?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}

pub(crate) fn abort(args: DogfoodAbortArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    let _ledger = util::lock_jsonl(&root.join(LEDGER_RELATIVE_PATH))?;
    reconcile_pending_transaction_locked(&root)?;
    let mut state = load_state(&root)?;
    let active_indices = state
        .runs
        .iter()
        .enumerate()
        .filter_map(|(index, run)| (run.status == DogfoodRunStatus::Active).then_some(index))
        .collect::<Vec<_>>();
    let [active_index] = active_indices.as_slice() else {
        if active_indices.is_empty() {
            bail!("no active dogfood run to abort");
        }
        bail!("dogfood state contains multiple active runs");
    };
    let reason = sanitized_reason(&args.reason)?;
    let run = &mut state.runs[*active_index];
    run.status = DogfoodRunStatus::Aborted;
    run.aborted_at = Some(Utc::now());
    run.abort_reason = Some(reason);
    let output = run.clone();
    write_state(&root, &state)?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub(crate) fn finalize(args: DogfoodFinalizeArgs) -> Result<()> {
    finalize_at(args, Utc::now())
}

fn finalize_at(args: DogfoodFinalizeArgs, now: DateTime<Utc>) -> Result<()> {
    finalize_at_with_fault(args, now, None)
}

fn finalize_at_with_fault(
    args: DogfoodFinalizeArgs,
    now: DateTime<Utc>,
    fault: Option<FinalizeFault>,
) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    let mut ledger_lock = util::lock_jsonl(&root.join(LEDGER_RELATIVE_PATH))?;
    if reconcile_pending_transaction_locked(&root)? == ReconcileOutcome::Finalized {
        let state = load_state(&root)?;
        let run = state
            .runs
            .last()
            .context("recovered dogfood finalization has no run state")?;
        println!("{}", serde_json::to_string_pretty(run)?);
        return Ok(());
    }
    let ledger = ledger_lock.read_all()?;
    let state = load_state(&root)?;
    let active_index = active_run_index(&state)?;
    let active = state.runs[active_index].clone();
    let sign_off = QualificationSignOff {
        signer: sanitized_text("signer", &args.signer, 200)?,
        incident_disposition: sanitized_text(
            "incident disposition",
            &args.incident_disposition,
            1_000,
        )?,
        signed_at: now,
    };
    verify_run_provenance(&root, &active, &ledger)?;

    let shadow_metrics = learning::shadow_metrics_value_from_locked_ledger_at(&root, now, &ledger)?;
    let safety = validate_metrics(&shadow_metrics, &active.run_id)?;
    verify_ledger_prefix(&active, &ledger)?;
    let review_summary = validate_review_receipts(&root, &active, &ledger, now)?;
    validate_review_for_finalize(&review_summary)?;
    let ledger_provenance = LedgerProvenance {
        boundary_bytes: active.ledger_boundary_bytes,
        boundary_lines: active.ledger_boundary_lines,
        boundary_sha256: active.ledger_boundary_sha256.clone(),
        final_bytes: ledger.len() as u64,
        final_lines: nonempty_line_count(&ledger)?,
        final_sha256: util::sha256_hex(&ledger),
    };

    let (out, staging) = prepare_evidence_paths(&root, &args.out)?;
    let original_state = state.clone();
    let mut archived_state = state.clone();
    {
        let run = &mut archived_state.runs[active_index];
        run.status = DogfoodRunStatus::Finalized;
        run.finalized_at = Some(now);
        run.final_export = None;
        run.sign_off = Some(sign_off.clone());
    }
    let mut journal = FinalizeTransaction {
        phase: FinalizePhase::Prepared,
        run_id: active.run_id.clone(),
        active_index,
        original_state,
        archived_state: archived_state.clone(),
        out: out.clone(),
        staging: staging.clone(),
    };
    let result = (|| -> Result<QualificationReport> {
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::JournalPrepared)?;

        write_state(&root, &archived_state)?;
        journal.phase = FinalizePhase::ArchivedStateWritten;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::ArchivedStateWritten)?;

        fs::create_dir(&staging)
            .with_context(|| format!("create staging evidence directory {}", staging.display()))?;
        util::sync_directory(
            staging
                .parent()
                .context("dogfood staging directory has no parent")?,
        )?;
        journal.phase = FinalizePhase::StagingCreated;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::StagingCreated)?;

        let export_relative = final_export_relative_path(&active.run_id);
        let qualification_name = "qualification";
        let export_path = root.join(&export_relative);
        if export_path.exists() || fs::symlink_metadata(&export_path).is_ok() {
            bail!("private final dogfood backup already exists");
        }
        export::export_portable_snapshot(&root, &export_path)?;
        util::sync_file(&export_path)?;
        export::verify_export_archive(&export_path, None)?;
        journal.phase = FinalizePhase::ExportWritten;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::ExportWritten)?;

        let final_export = ChecksummedArtifact {
            relative_path: export_relative,
            sha256: file_sha256(&export_path)?,
        };
        let report = QualificationReport {
            schema_version: "brainmap-dogfood-qualification-v3",
            status: "passed",
            run_id: active.run_id.clone(),
            candidate_commit: active.candidate_commit.clone(),
            candidate_binary_sha256: active.candidate_binary_sha256.clone(),
            candidate_brainmapd_sha256: active.candidate_brainmapd_sha256.clone(),
            host: active.host.clone(),
            adapter: active.adapter.clone(),
            mode: active.mode.clone(),
            started_at: active.started_at,
            ended_at: now,
            duration_seconds: (now - active.started_at).num_seconds(),
            start_backup: active.start_backup.clone(),
            qualification: QualificationProvenance {
                relative_path: qualification_name.into(),
                run_relative_path: active.qualification_bundle_relative_path.clone(),
                bundle_sha256: active.qualification_bundle_sha256.clone(),
                manifest_sha256: active.qualification_manifest_sha256.clone(),
            },
            ledger: ledger_provenance,
            final_backup_sha256: final_export.sha256.clone(),
            shadow_metrics,
            safety,
            review_summary,
            sign_off: sign_off.clone(),
            raw_prompts_retained: false,
        };
        let json_name = "dogfood-qualification.json";
        let markdown_name = "dogfood-qualification.md";
        let copied_qualification = qualification::copy_verified_bundle(
            &root.join(&active.qualification_bundle_relative_path),
            &staging.join(qualification_name),
        )?;
        ensure!(
            copied_qualification.bundle_sha256 == active.qualification_bundle_sha256
                && copied_qualification.candidate.commit == active.candidate_commit
                && copied_qualification.candidate.brainmap_sha256 == active.candidate_binary_sha256
                && copied_qualification.candidate.brainmapd_sha256
                    == active.candidate_brainmapd_sha256,
            "dogfood final evidence qualification copy changed provenance"
        );
        ensure!(
            file_sha256(&staging.join(qualification_name).join("qualification.json"))?
                == active.qualification_manifest_sha256,
            "dogfood final evidence qualification manifest changed provenance"
        );
        journal.phase = FinalizePhase::QualificationBundleCopied;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::QualificationBundleCopied)?;

        util::write_atomic(
            &staging.join(json_name),
            &serde_json::to_vec_pretty(&report)?,
        )?;
        journal.phase = FinalizePhase::JsonReportWritten;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::JsonReportWritten)?;

        util::write_atomic(
            &staging.join(markdown_name),
            qualification_markdown(&report).as_bytes(),
        )?;
        journal.phase = FinalizePhase::MarkdownReportWritten;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::MarkdownReportWritten)?;

        journal.phase = FinalizePhase::ChecksumsWritten;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        write_checksums(&staging)?;
        util::sync_directory(&staging)?;
        inject_finalize_fault(fault, FinalizeFault::ChecksumsWritten)?;

        util::rename_and_sync(&staging, &out)?;
        journal.phase = FinalizePhase::EvidenceActivated;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::EvidenceActivated)?;

        let verified_final_export =
            verify_evidence_bundle(&root, &out, &archived_state.runs[active_index])?;
        ensure!(
            verified_final_export == final_export,
            "activated dogfood evidence export provenance changed"
        );

        let mut final_state = archived_state.clone();
        final_state.runs[active_index].final_export = Some(final_export);
        write_state(&root, &final_state)?;
        journal.phase = FinalizePhase::FinalStateWritten;
        journal.archived_state = final_state;
        write_transaction(&root, DogfoodTransaction::Finalize(journal.clone()))?;
        inject_finalize_fault(fault, FinalizeFault::FinalStateWritten)?;

        clear_transaction(&root)?;
        Ok(report)
    })();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            if fault.is_some() {
                return Err(error);
            }
            return match reconcile_pending_transaction_locked(&root) {
                Ok(ReconcileOutcome::Finalized) => {
                    let report: serde_json::Value =
                        serde_json::from_slice(&fs::read(out.join("dogfood-qualification.json"))?)?;
                    println!("{}", serde_json::to_string_pretty(&report)?);
                    Ok(())
                }
                Ok(_) => Err(error),
                Err(recovery) => Err(error).context(format!(
                    "dogfood finalize recovery also failed: {recovery:#}"
                )),
            };
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn ensure_initialized_vault(root: &Path) -> Result<()> {
    if !root.join(".brainmap").is_dir() || !root.join("README.md").is_file() {
        bail!("dogfood requires an initialized Brainmap vault");
    }
    Ok(())
}

fn state_path(root: &Path) -> PathBuf {
    root.join(STATE_RELATIVE_PATH)
}

fn load_state(root: &Path) -> Result<DogfoodState> {
    let path = state_path(root);
    if !path.exists() {
        return Ok(DogfoodState::default());
    }
    let state: DogfoodState = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("read {}", path.display()))?,
    )
    .context("parse dogfood state")?;
    if state.format != STATE_FORMAT || state.version != STATE_VERSION {
        bail!("unsupported dogfood state format or version");
    }
    validate_state_snapshot(root, "persisted", &state)?;
    Ok(state)
}

fn write_state(root: &Path, state: &DogfoodState) -> Result<()> {
    validate_state_snapshot(root, "write", state)?;
    util::write_atomic(&state_path(root), &serde_json::to_vec_pretty(state)?)
}

fn active_run_index(state: &DogfoodState) -> Result<usize> {
    let indices = state
        .runs
        .iter()
        .enumerate()
        .filter_map(|(index, run)| (run.status == DogfoodRunStatus::Active).then_some(index))
        .collect::<Vec<_>>();
    let [index] = indices.as_slice() else {
        if indices.is_empty() {
            bail!("no active dogfood run");
        }
        bail!("dogfood state contains multiple active runs");
    };
    Ok(*index)
}

fn verify_run_provenance(root: &Path, run: &DogfoodRunState, ledger: &[u8]) -> Result<()> {
    if run.mode != "shadow" {
        bail!("dogfood state mode drift: expected shadow");
    }
    let autopilot = learning::autopilot_config(root);
    if run.gate_mode != "shadow" || learning::gate_mode_config(root) != run.gate_mode {
        bail!("dogfood gate mode drift: expected frozen shadow mode");
    }
    if run.autopilot_mode != "shadow"
        || autopilot.mode != run.autopilot_mode
        || autopilot.level != run.autopilot_level
        || (autopilot.threshold - run.threshold).abs() > f64::EPSILON
    {
        bail!("dogfood autopilot mode, level, or threshold drift");
    }
    if std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() == Some("1") {
        bail!("dogfood kill-switch drift: autopilot is disabled");
    }
    if run.host.os != std::env::consts::OS || run.host.arch != std::env::consts::ARCH {
        bail!("dogfood host drift: run must finish on its starting OS and architecture");
    }
    if current_binary_identity()? != run.candidate_binary_identity {
        bail!("dogfood binary metadata identity changed");
    }
    let running_candidate = qualification::running_candidate_hashes()?;
    if running_candidate.brainmap_sha256 != run.candidate_binary_sha256 {
        bail!("dogfood binary drift: running binary checksum changed");
    }
    if running_candidate.brainmapd_sha256 != run.candidate_brainmapd_sha256 {
        bail!("dogfood brainmapd drift: companion binary checksum changed");
    }

    verify_qualification_provenance(root, run)?;
    verify_start_backup(root, run)?;
    verify_ledger_prefix(run, ledger)
}

fn verify_qualification_provenance(root: &Path, run: &DogfoodRunState) -> Result<()> {
    let bundle_path = root.join(&run.qualification_bundle_relative_path);
    let verified = qualification::verify_bundle(&bundle_path).with_context(|| {
        format!(
            "verify copied dogfood qualification bundle {}",
            bundle_path.display()
        )
    })?;
    if verified.bundle_sha256 != run.qualification_bundle_sha256 {
        bail!("dogfood qualification bundle digest drift");
    }
    if file_sha256(&bundle_path.join("qualification.json"))? != run.qualification_manifest_sha256 {
        bail!("dogfood qualification manifest checksum drift");
    }
    if verified.candidate.commit != run.candidate_commit {
        bail!("dogfood candidate commit drift from qualification bundle");
    }
    if verified.candidate.brainmap_sha256 != run.candidate_binary_sha256 {
        bail!("dogfood candidate binary drift from qualification bundle");
    }
    if verified.candidate.brainmapd_sha256 != run.candidate_brainmapd_sha256 {
        bail!("dogfood candidate brainmapd drift from qualification bundle");
    }
    Ok(())
}

fn verify_start_backup(root: &Path, run: &DogfoodRunState) -> Result<()> {
    let backup_path = root.join(&run.start_backup.relative_path);
    if file_sha256(&backup_path)? != run.start_backup.sha256 {
        bail!("dogfood start backup checksum drift");
    }
    export::verify_export_archive(&backup_path, None)
}

fn run_health(
    root: &Path,
    run: &DogfoodRunState,
    metrics: Option<&serde_json::Value>,
    reviews: Option<&ReviewSummary>,
    ledger: &[u8],
) -> RunHealth {
    let qualification_bundle_matches = verify_qualification_provenance(root, run).is_ok();
    let running_candidate = qualification::running_candidate_hashes();
    let binary_matches = current_binary_identity()
        .is_ok_and(|identity| identity == run.candidate_binary_identity)
        && running_candidate
            .as_ref()
            .is_ok_and(|hashes| hashes.brainmap_sha256 == run.candidate_binary_sha256);
    let brainmapd_matches = running_candidate
        .as_ref()
        .is_ok_and(|hashes| hashes.brainmapd_sha256 == run.candidate_brainmapd_sha256);
    let host_matches =
        run.host.os == std::env::consts::OS && run.host.arch == std::env::consts::ARCH;
    let autopilot = learning::autopilot_config(root);
    let shadow_mode_intact = run.mode == "shadow"
        && run.gate_mode == "shadow"
        && run.autopilot_mode == "shadow"
        && learning::gate_mode_config(root) == run.gate_mode
        && autopilot.mode == run.autopilot_mode
        && autopilot.level == run.autopilot_level
        && (autopilot.threshold - run.threshold).abs() <= f64::EPSILON
        && std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() != Some("1");
    let start_backup_valid = verify_start_backup(root, run).is_ok();
    let ledger_prefix_valid = verify_ledger_prefix(run, ledger).is_ok();
    let metrics_integrity_valid = metrics.is_some_and(metrics_integrity_valid);
    let safety_clean = metrics.is_some_and(metrics_safety_clean);
    let intensive_session_ready = metrics.is_some_and(|value| {
        value
            .get("intensiveSessionDistributionValid")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    });
    let review_integrity_valid = reviews.is_some_and(|summary| summary.integrity_valid);
    let review_ready = reviews.is_some_and(|summary| summary.review_ready);
    let healthy = qualification_bundle_matches
        && binary_matches
        && brainmapd_matches
        && host_matches
        && shadow_mode_intact
        && start_backup_valid
        && ledger_prefix_valid
        && metrics_integrity_valid
        && safety_clean
        && review_integrity_valid;
    RunHealth {
        qualification_bundle_matches,
        binary_matches,
        brainmapd_matches,
        host_matches,
        shadow_mode_intact,
        start_backup_valid,
        ledger_prefix_valid,
        metrics_integrity_valid,
        safety_clean,
        intensive_session_ready,
        review_integrity_valid,
        review_ready,
        healthy,
    }
}

fn validate_review_for_finalize(summary: &ReviewSummary) -> Result<()> {
    if !summary.integrity_valid {
        bail!("dogfood review receipt integrity validation failed");
    }
    if summary.candidate_failed {
        bail!("dogfood candidate was failed by a terminal review receipt");
    }
    if summary.unresolved_investigation {
        bail!("dogfood incident investigation must be resolved before finalization");
    }
    if summary.review_count == 0 {
        bail!("dogfood finalization requires a persisted prompt-free review receipt");
    }
    if !summary.final_review_covers_ledger {
        bail!("final dogfood review must cover the complete qualification ledger");
    }
    if !summary.review_ready {
        bail!("final dogfood review must have clear or resolved-no-violation status");
    }
    Ok(())
}

fn verify_ledger_prefix(run: &DogfoodRunState, ledger: &[u8]) -> Result<()> {
    let boundary = usize::try_from(run.ledger_boundary_bytes)
        .context("dogfood ledger boundary does not fit this platform")?;
    if boundary > ledger.len() {
        bail!("dogfood ledger was truncated before its start boundary");
    }
    if util::sha256_hex(&ledger[..boundary]) != run.ledger_boundary_sha256 {
        bail!("dogfood ledger prefix checksum drift");
    }
    Ok(())
}

fn validate_metrics(
    metrics: &serde_json::Value,
    expected_run_id: &str,
) -> Result<SafetyQualification> {
    let run_id = metrics
        .get("runId")
        .and_then(serde_json::Value::as_str)
        .context("shadow metrics are not scoped to an active dogfood run")?;
    if run_id != expected_run_id {
        bail!("shadow metrics dogfood run ID does not match the active run");
    }
    if metrics
        .get("rawPromptsRetained")
        .and_then(serde_json::Value::as_bool)
        != Some(false)
    {
        bail!("shadow metrics must explicitly report rawPromptsRetained=false");
    }
    if !metrics_integrity_valid(metrics) {
        bail!("dogfood action coverage or ledger integrity is incomplete");
    }
    let complete_pairs = metric_count(metrics, "completeGateActionPairs")?;
    let decision_scenarios = metric_count(metrics, "distinctDecisionScenarios")?;
    let scopes = metric_count(metrics, "distinctScopes")?;
    let decision_types = metric_count(metrics, "distinctDecisionTypes")?;
    let intensive_session_valid = metrics
        .get("intensiveSessionDistributionValid")
        .and_then(serde_json::Value::as_bool)
        == Some(true);
    if complete_pairs < MINIMUM_COMPLETE_PAIRS
        || decision_scenarios < MINIMUM_DECISION_SCENARIOS
        || (scopes < MINIMUM_SCOPES_OR_DECISION_TYPES
            && decision_types < MINIMUM_SCOPES_OR_DECISION_TYPES)
        || !intensive_session_valid
    {
        bail!(
            "dogfood finalization requires >=30 complete gate/action pairs across >=5 distinct decision scenarios and >=3 distinct scopes or decision types"
        );
    }
    let safety = SafetyQualification {
        passed: true,
        false_proceeds: metric_count(metrics, "falseProceeds")?,
        confirmed_collisions: metric_count(metrics, "confirmedCollisions")?,
        confirmed_cross_domain_applications: metric_count(
            metrics,
            "confirmedCrossDomainApplications",
        )?,
        privacy_violations: metric_count(metrics, "privacyViolations")?,
        hard_rule_violations: metric_count(metrics, "hardRuleViolations")?,
    };
    if safety.false_proceeds > 0
        || safety.confirmed_cross_domain_applications > 0
        || safety.privacy_violations > 0
        || safety.hard_rule_violations > 0
    {
        bail!("dogfood candidate has a confirmed safety or policy incident");
    }
    Ok(safety)
}

fn metrics_integrity_valid(metrics: &serde_json::Value) -> bool {
    metrics
        .get("ledgerIntegrityValid")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && [
            "missingActionRecords",
            "invalidActualActionRecords",
            "duplicateGateIds",
            "duplicateActionRecords",
            "duplicateEventIds",
            "orphanActionRecords",
            "orphanFeedbackRecords",
            "feedbackMissingPacketIds",
            "unpreviewedFeedbackPackets",
            "unappliedFeedbackPackets",
            "orphanPreviewUpdateRecords",
            "orphanApplyUpdateRecords",
            "unapprovedApplyUpdateRecords",
            "packetLifecycleOrderViolations",
            "provenanceMismatches",
            "outOfIntervalEvents",
        ]
        .into_iter()
        .all(|field| metric_count(metrics, field).ok() == Some(0))
}

fn metrics_safety_clean(metrics: &serde_json::Value) -> bool {
    [
        "falseProceeds",
        "confirmedCrossDomainApplications",
        "privacyViolations",
        "hardRuleViolations",
    ]
    .into_iter()
    .all(|field| metric_count(metrics, field).ok() == Some(0))
}

fn metric_count(metrics: &serde_json::Value, field: &str) -> Result<u64> {
    metrics
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .with_context(|| format!("shadow metrics are missing integer field {field}"))
}

fn prepare_evidence_paths(root: &Path, requested: &Path) -> Result<(PathBuf, PathBuf)> {
    if requested.file_name().is_none() {
        bail!("dogfood evidence output must name a new directory");
    }
    let parent = requested
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        bail!(
            "dogfood evidence parent must already exist: {}",
            parent.display()
        );
    }
    let canonical_parent = fs::canonicalize(parent)
        .with_context(|| format!("resolve evidence parent {}", parent.display()))?;
    let out = canonical_parent.join(requested.file_name().context("missing output name")?);
    if out.exists() {
        bail!("dogfood evidence output already exists: {}", out.display());
    }
    let canonical_root =
        fs::canonicalize(root).with_context(|| format!("resolve vault root {}", root.display()))?;
    if out.starts_with(&canonical_root) || canonical_root.starts_with(&out) {
        bail!("dogfood evidence output must not overlap the vault");
    }
    let staging = canonical_parent.join(format!(
        ".{}.{}.tmp",
        requested
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("dogfood-evidence"),
        util::id("evidence", "dogfood")
    ));
    Ok((out, staging))
}

fn write_checksums(directory: &Path) -> Result<()> {
    let file_names = collect_evidence_files(directory)?;
    let mut lines = Vec::with_capacity(file_names.len());
    for file_name in file_names {
        if file_name == "SHA256SUMS" {
            continue;
        }
        lines.push(format!(
            "{}  {file_name}",
            file_sha256(&directory.join(&file_name))?
        ));
    }
    let mut contents = lines.join("\n");
    contents.push('\n');
    util::write_atomic(&directory.join("SHA256SUMS"), contents.as_bytes())
}

fn collect_evidence_files(directory: &Path) -> Result<BTreeSet<String>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeSet<String>) -> Result<()> {
        for entry in fs::read_dir(directory)
            .with_context(|| format!("read dogfood evidence directory {}", directory.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!("dogfood evidence contains a symlink");
            }
            if metadata.is_dir() {
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .context("dogfood evidence file escaped its root")?;
                let relative = relative
                    .components()
                    .map(|component| {
                        component
                            .as_os_str()
                            .to_str()
                            .context("dogfood evidence path is not UTF-8")
                    })
                    .collect::<Result<Vec<_>>>()?
                    .join("/");
                validate_evidence_relative_path(&relative)?;
                if !files.insert(relative) {
                    bail!("duplicate dogfood evidence path");
                }
            } else {
                bail!("dogfood evidence contains a non-regular file");
            }
        }
        Ok(())
    }

    let mut files = BTreeSet::new();
    visit(directory, directory, &mut files)?;
    Ok(files)
}

fn validate_evidence_relative_path(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("invalid dogfood evidence relative path");
    }
    for component in path.components() {
        let component = component
            .as_os_str()
            .to_str()
            .context("dogfood evidence path is not UTF-8")?;
        if component.is_empty()
            || !component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            bail!("dogfood evidence paths must use portable ASCII components");
        }
    }
    Ok(())
}

fn qualification_markdown(report: &QualificationReport) -> String {
    let metric = |field| {
        report
            .shadow_metrics
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
    };
    format!(
        "# Brainmap dogfood qualification\n\n\
         - Status: passed\n\
         - Run ID: `{}`\n\
         - Candidate commit: `{}`\n\
         - Candidate binary SHA-256: `{}`\n\
         - Candidate brainmapd SHA-256: `{}`\n\
         - Adapter: `{}`\n\
         - Host: `{}-{}`\n\
         - Mode: `shadow`\n\
         - Started: `{}`\n\
         - Ended: `{}`\n\
         - Duration: {} seconds\n\
         - Qualification bundle path: `{}`\n\
         - Qualification run path: `{}`\n\
         - Qualification bundle SHA-256: `{}`\n\
         - Qualification manifest SHA-256: `{}`\n\
         - Start backup SHA-256: `{}`\n\
         - Private final backup SHA-256: `{}`\n\n\
         ## Intensive-session coverage\n\n\
         - Complete gate/action pairs: {}\n\
         - Distinct decision scenarios: {}\n\
         - Distinct scopes: {}\n\
         - Distinct decision types: {}\n\n\
         ## Final prompt-free review\n\n\
         - Reviews: {}\n\
         - First review: `{}`\n\
         - Final review: `{}`\n\
         - Final incident state: `{}`\n\
         - Final review covers ledger: {}\n\
         - Review ready: {}\n\n\
         ## Sign-off\n\n\
         - Signer: {}\n\
         - Signed at: {}\n\
         - Incident disposition: {}\n\n\
         Aggregate metrics are recorded in `dogfood-qualification.json`; raw prompts, situations, scopes, and decision types are not retained in aggregate evidence. Longer monitoring is optional and is not a qualification gate.\n",
        report.run_id,
        report.candidate_commit,
        report.candidate_binary_sha256,
        report.candidate_brainmapd_sha256,
        report.adapter,
        report.host.os,
        report.host.arch,
        format_timestamp(report.started_at),
        format_timestamp(report.ended_at),
        report.duration_seconds,
        report.qualification.relative_path,
        report.qualification.run_relative_path,
        report.qualification.bundle_sha256,
        report.qualification.manifest_sha256,
        report.start_backup.sha256,
        report.final_backup_sha256,
        metric("completeGateActionPairs"),
        metric("distinctDecisionScenarios"),
        metric("distinctScopes"),
        metric("distinctDecisionTypes"),
        report.review_summary.review_count,
        report
            .review_summary
            .first_review_at
            .as_deref()
            .unwrap_or("none"),
        report
            .review_summary
            .last_review_at
            .as_deref()
            .unwrap_or("none"),
        report
            .review_summary
            .current_incident_status
            .map(IncidentStatus::as_str)
            .unwrap_or("none"),
        report.review_summary.final_review_covers_ledger,
        report.review_summary.review_ready,
        report.sign_off.signer,
        format_timestamp(report.sign_off.signed_at),
        report.sign_off.incident_disposition,
    )
}

fn parse_timestamp(label: &str, raw: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("{label} must be an RFC 3339 timestamp"))
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn validate_commit(commit: &str) -> Result<()> {
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("candidate commit must be a full 40-character lowercase Git object ID");
    }
    Ok(())
}

fn validate_sha256(label: &str, sha256: &str) -> Result<()> {
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("{label} must be a lowercase SHA-256 digest");
    }
    Ok(())
}

fn validate_safe_label(label: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || privacy::contains_secret(value)
    {
        bail!("{label} must be a safe 1-64 character identifier");
    }
    Ok(())
}

fn sanitized_reason(reason: &str) -> Result<String> {
    let redacted = privacy::redact(reason);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        bail!("abort reason must not be empty");
    }
    Ok(normalized.chars().take(500).collect())
}

pub(crate) fn current_binary_identity() -> Result<CandidateBinaryIdentity> {
    let executable = std::env::current_exe().context("resolve running Brainmap binary identity")?;
    binary_identity(&executable, "running Brainmap binary")
}

fn binary_identity(path: &Path, label: &str) -> Result<CandidateBinaryIdentity> {
    let metadata =
        fs::metadata(path).with_context(|| format!("read {label} metadata {}", path.display()))?;
    ensure!(metadata.is_file(), "{label} is not a regular file");
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| u64::try_from(duration.as_nanos()).ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(CandidateBinaryIdentity {
            len: metadata.len(),
            modified_unix_nanos,
            device: metadata.dev(),
            inode: metadata.ino(),
            changed_unix_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(CandidateBinaryIdentity {
            len: metadata.len(),
            modified_unix_nanos,
        })
    }
}

pub(crate) fn capture_gate_provenance_version(root: &Path) -> Result<GateProvenanceVersion> {
    Ok(GateProvenanceVersion {
        autopilot: gate_file_version(&root.join(".brainmap/autopilot.json"))?,
        gate_mode: gate_file_version(&root.join(".brainmap/gate-mode"))?,
        candidate_binary_identity: current_binary_identity()?,
    })
}

pub(crate) fn validate_gate_provenance_snapshot(
    run: &DogfoodRunContext,
    gate_mode: &str,
    autopilot: &learning::AutopilotConfig,
    provenance: &GateProvenanceVersion,
) -> Result<()> {
    if gate_mode != run.gate_mode
        || autopilot.mode != run.autopilot_mode
        || autopilot.level != run.autopilot_level
        || (autopilot.threshold - run.threshold).abs() > f64::EPSILON
    {
        bail!("active dogfood gate configuration drifted from its frozen provenance");
    }
    if provenance.candidate_binary_identity != run.candidate_binary_identity {
        bail!("active dogfood gate binary identity drifted from its candidate provenance");
    }
    Ok(())
}

fn sanitized_text(label: &str, value: &str, max_chars: usize) -> Result<String> {
    let redacted = privacy::redact(value);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        bail!("{label} must not be empty");
    }
    Ok(normalized.chars().take(max_chars).collect())
}

fn file_sha256(path: &Path) -> Result<String> {
    Ok(util::sha256_hex(&fs::read(path).with_context(|| {
        format!("read {} for checksum", path.display())
    })?))
}

fn nonempty_line_count(bytes: &[u8]) -> Result<u64> {
    let text = std::str::from_utf8(bytes).context("decision ledger is not valid UTF-8")?;
    Ok(text.lines().filter(|line| !line.trim().is_empty()).count() as u64)
}

fn remove_empty_parents(path: &Path, stop: &Path) {
    let mut current = path.parent();
    while let Some(directory) = current {
        if !directory.starts_with(stop) {
            break;
        }
        if fs::remove_dir(directory).is_err() {
            break;
        }
        current = directory.parent();
    }
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn every_start_crash_boundary_reconciles_to_prior_state_or_active() {
        for fault in [
            StartFault::JournalPrepared,
            StartFault::BackupCreated,
            StartFault::QualificationDirectoryPrepared,
            StartFault::QualificationBundleCopied,
            StartFault::IntentRecorded,
            StartFault::AutopilotWritten,
            StartFault::GateModeWritten,
            StartFault::StateActivated,
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("BrainMap");
            let qualification_bundle = tmp.path().join("qualification");
            vault::init_vault(Some(root.clone()), false, true).unwrap();
            learning::write_gate_mode_config(&root, "active").unwrap();
            let original_gate_mode = fs::read(root.join(".brainmap/gate-mode")).unwrap();
            let started_at = Utc::now();
            write_test_qualification_bundle(&qualification_bundle);

            let error = start_at_with_test_verifier(
                DogfoodStartArgs {
                    candidate_commit: COMMIT.into(),
                    adapter: "codex".into(),
                    started_at: format_timestamp(started_at),
                    qualification_bundle,
                    vault: Some(root.clone()),
                },
                started_at,
                Some(fault),
            )
            .unwrap_err();
            assert!(error.to_string().contains("injected dogfood start crash"));
            assert!(transaction_path(&root).is_file());

            status(Some(root.clone())).unwrap();
            assert!(!transaction_path(&root).exists());
            let state = load_state(&root).unwrap();
            if fault == StartFault::StateActivated {
                let run = state.runs.last().unwrap();
                assert_eq!(run.status, DogfoodRunStatus::Active);
                assert!(root.join(&run.start_backup.relative_path).is_file());
                assert!(root.join(&run.qualification_bundle_relative_path).is_dir());
                assert_eq!(learning::gate_mode_config(&root), "shadow");
            } else {
                assert!(state.runs.is_empty());
                assert_eq!(
                    fs::read(root.join(".brainmap/gate-mode")).unwrap(),
                    original_gate_mode
                );
                assert!(!root.join(".brainmap/autopilot.json").exists());
                assert_eq!(
                    fs::read_dir(root.join("99-meta/backups")).unwrap().count(),
                    0
                );
                assert!(!root.join(".brainmap/dogfood").exists());
            }
        }
    }

    #[test]
    fn recovered_start_retries_require_exact_start_time_and_bundle_provenance() {
        for exact_retry in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("BrainMap");
            let qualification_bundle = tmp.path().join("qualification");
            vault::init_vault(Some(root.clone()), false, true).unwrap();
            write_test_qualification_bundle(&qualification_bundle);
            let started_at = Utc::now();

            start_at_with_test_verifier(
                test_start_args(&root, &qualification_bundle, started_at),
                started_at,
                Some(StartFault::StateActivated),
            )
            .unwrap_err();

            let retry_start = if exact_retry {
                started_at
            } else {
                started_at + Duration::seconds(1)
            };
            let retry = start_at_with_test_verifier(
                test_start_args(&root, &qualification_bundle, retry_start),
                started_at,
                None,
            );
            if exact_retry {
                retry.unwrap();
            } else {
                assert!(
                    retry
                        .unwrap_err()
                        .to_string()
                        .contains("conflicts with the requested run provenance")
                );
            }
            assert!(!transaction_path(&root).exists());
            assert_eq!(
                load_state(&root)
                    .unwrap()
                    .runs
                    .iter()
                    .filter(|run| run.status == DogfoodRunStatus::Active)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn start_recovery_rolls_back_a_tampered_nested_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        let qualification_bundle = tmp.path().join("qualification");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        write_test_qualification_bundle(&qualification_bundle);
        let started_at = Utc::now();

        let error = start_at_with_test_verifier(
            test_start_args(&root, &qualification_bundle, started_at),
            started_at,
            Some(StartFault::StateActivated),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("injected dogfood start crash"),
            "unexpected start failure: {error:#}"
        );
        let copied = match load_transaction(&root).unwrap().unwrap() {
            DogfoodTransaction::Start(transaction) => root.join(
                transaction
                    .intended_run
                    .unwrap()
                    .qualification_bundle_relative_path,
            ),
            DogfoodTransaction::Finalize(_) => panic!("unexpected finalize journal"),
        };
        fs::write(
            copied.join("runner/reports/fia1.json"),
            b"{\"tampered\":true}\n",
        )
        .unwrap();

        status(Some(root.clone())).unwrap();
        assert!(load_state(&root).unwrap().runs.is_empty());
        assert!(!transaction_path(&root).exists());
        assert!(!root.join(".brainmap/dogfood").exists());
        assert_eq!(
            fs::read_dir(root.join("99-meta/backups")).unwrap().count(),
            0
        );
    }

    #[test]
    fn every_finalize_crash_boundary_reconciles_without_partial_success() {
        let tmp = tempfile::tempdir().unwrap();
        let baseline = tmp.path().join("Baseline");
        let finalized_at = prepare_qualifiable_vault(&baseline, tmp.path());
        for (index, fault) in [
            FinalizeFault::JournalPrepared,
            FinalizeFault::ArchivedStateWritten,
            FinalizeFault::StagingCreated,
            FinalizeFault::ExportWritten,
            FinalizeFault::QualificationBundleCopied,
            FinalizeFault::JsonReportWritten,
            FinalizeFault::MarkdownReportWritten,
            FinalizeFault::ChecksumsWritten,
            FinalizeFault::EvidenceActivated,
            FinalizeFault::FinalStateWritten,
        ]
        .into_iter()
        .enumerate()
        {
            let root = tmp.path().join(format!("CrashVault-{index}"));
            copy_tree(&baseline, &root);
            let out = tmp.path().join(format!("evidence-{index}"));
            let error = finalize_at_with_fault(
                DogfoodFinalizeArgs {
                    out: out.clone(),
                    signer: "Local developer".into(),
                    incident_disposition: "No incidents observed".into(),
                    vault: Some(root.clone()),
                },
                finalized_at,
                Some(fault),
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("injected dogfood finalize crash")
            );
            let staging = match load_transaction(&root).unwrap().unwrap() {
                DogfoodTransaction::Finalize(transaction) => transaction.staging,
                DogfoodTransaction::Start(_) => panic!("unexpected start journal"),
            };
            let should_finalize = matches!(
                fault,
                FinalizeFault::ChecksumsWritten
                    | FinalizeFault::EvidenceActivated
                    | FinalizeFault::FinalStateWritten
            );
            if should_finalize {
                assert!(
                    active_run_context(&root)
                        .unwrap_err()
                        .to_string()
                        .contains("recovery is required")
                );
            } else {
                assert!(active_run_context(&root).unwrap().is_some());
            }

            status(Some(root.clone())).unwrap();
            assert!(!transaction_path(&root).exists());
            assert!(!staging.exists());
            let run = load_state(&root).unwrap().runs.pop().unwrap();
            if should_finalize {
                assert_eq!(run.status, DogfoodRunStatus::Finalized);
                assert!(run.final_export.is_some());
                verify_evidence_bundle(&root, &out, &run).unwrap();
            } else {
                assert_eq!(run.status, DogfoodRunStatus::Active);
                assert!(run.final_export.is_none());
                assert!(!out.exists());
            }
        }
    }

    #[test]
    fn finalize_recovery_rejects_nested_qualification_tampering() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        let out = tmp.path().join("evidence");
        let finalized_at = prepare_qualifiable_vault(&root, tmp.path());

        finalize_at_with_fault(
            DogfoodFinalizeArgs {
                out: out.clone(),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(root.clone()),
            },
            finalized_at,
            Some(FinalizeFault::ChecksumsWritten),
        )
        .unwrap_err();
        let staging = match load_transaction(&root).unwrap().unwrap() {
            DogfoodTransaction::Finalize(transaction) => transaction.staging,
            DogfoodTransaction::Start(_) => panic!("unexpected start journal"),
        };
        fs::write(
            staging.join("qualification/runner/reports/fia1.json"),
            b"{\"tampered\":true}\n",
        )
        .unwrap();

        status(Some(root.clone())).unwrap();
        assert!(!transaction_path(&root).exists());
        assert!(!staging.exists());
        assert!(!out.exists());
        let run = load_state(&root).unwrap().runs.pop().unwrap();
        assert_eq!(run.status, DogfoodRunStatus::Active);
        assert!(run.final_export.is_none());
    }

    #[test]
    fn tampered_transaction_paths_fail_closed_without_deleting_external_data() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();

        let external_file = tmp.path().join("must-survive.txt");
        fs::write(&external_file, b"keep me").unwrap();
        write_transaction(
            &root,
            DogfoodTransaction::Start(Box::new(StartTransaction {
                phase: StartPhase::Prepared,
                previous_state_existed: false,
                previous_state: DogfoodState::default(),
                prior_autopilot: FileSnapshot::Missing,
                prior_gate_mode: FileSnapshot::Missing,
                backup_relative_path: "../must-survive.txt".into(),
                qualification_bundle_relative_path: ".brainmap/dogfood/run/qualification".into(),
                intended_run: None,
            })),
        )
        .unwrap();

        let error = status(Some(root.clone())).unwrap_err();
        assert!(error.to_string().contains("transaction journal path"));
        assert_eq!(fs::read(&external_file).unwrap(), b"keep me");

        clear_transaction(&root).unwrap();
        let external_directory = tmp.path().join("must-survive");
        fs::create_dir(&external_directory).unwrap();
        fs::write(external_directory.join("sentinel"), b"keep me too").unwrap();
        write_transaction(
            &root,
            DogfoodTransaction::Finalize(FinalizeTransaction {
                phase: FinalizePhase::Prepared,
                run_id: "dogfood_safe".into(),
                active_index: 0,
                original_state: DogfoodState::default(),
                archived_state: DogfoodState::default(),
                out: tmp.path().join("unused-output"),
                staging: external_directory.clone(),
            }),
        )
        .unwrap();

        let error = status(Some(root.clone())).unwrap_err();
        assert!(error.to_string().contains("transaction journal path"));
        assert_eq!(
            fs::read(external_directory.join("sentinel")).unwrap(),
            b"keep me too"
        );

        clear_transaction(&root).unwrap();
        let out = tmp.path().join("safe-output");
        let staging = tmp
            .path()
            .join(".safe-output.evidence_1720000000000_aaaaaaaaaaaa.tmp");
        write_transaction(
            &root,
            DogfoodTransaction::Finalize(FinalizeTransaction {
                phase: FinalizePhase::Prepared,
                run_id: "dogfood_safe".into(),
                active_index: 0,
                original_state: DogfoodState::default(),
                archived_state: DogfoodState::default(),
                out: out.clone(),
                staging: staging.clone(),
            }),
        )
        .unwrap();

        let error = status(Some(root)).unwrap_err();
        assert!(error.to_string().contains("active index"));
        assert!(!out.exists());
        assert!(!staging.exists());
    }

    #[test]
    fn state_and_transaction_v3_are_explicit_and_older_versions_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();

        write_state(&root, &DogfoodState::default()).unwrap();
        let mut state_json: Value =
            serde_json::from_slice(&fs::read(state_path(&root)).unwrap()).unwrap();
        assert_eq!(state_json["version"], STATE_VERSION);
        for rejected_version in [1, 2] {
            state_json["version"] = json!(rejected_version);
            util::write_atomic(
                &state_path(&root),
                &serde_json::to_vec_pretty(&state_json).unwrap(),
            )
            .unwrap();
            assert!(
                load_state(&root)
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported dogfood state format or version")
            );
        }
        util::remove_file_and_sync(&state_path(&root)).unwrap();

        write_transaction(
            &root,
            DogfoodTransaction::Start(Box::new(StartTransaction {
                phase: StartPhase::Prepared,
                previous_state_existed: false,
                previous_state: DogfoodState::default(),
                prior_autopilot: FileSnapshot::Missing,
                prior_gate_mode: FileSnapshot::Missing,
                backup_relative_path: "99-meta/backups/schema_test-start.brainmap.tar.zst".into(),
                qualification_bundle_relative_path: ".brainmap/dogfood/schema_test/qualification"
                    .into(),
                intended_run: None,
            })),
        )
        .unwrap();
        let mut journal_json: Value =
            serde_json::from_slice(&fs::read(transaction_path(&root)).unwrap()).unwrap();
        assert_eq!(journal_json["version"], TRANSACTION_VERSION);
        for rejected_version in [1, 2] {
            journal_json["version"] = json!(rejected_version);
            util::write_atomic(
                &transaction_path(&root),
                &serde_json::to_vec_pretty(&journal_json).unwrap(),
            )
            .unwrap();
            assert!(
                load_transaction(&root)
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported dogfood transaction journal format or version")
            );
        }
    }

    #[test]
    fn rollback_does_not_delete_a_bundle_the_transaction_did_not_create() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let run_id = "preexisting_run";
        let bundle = root.join(format!(".brainmap/dogfood/{run_id}/qualification"));
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("sentinel"), b"keep").unwrap();
        let backup_relative = format!("99-meta/backups/{run_id}-start.brainmap.tar.zst");
        fs::write(root.join(&backup_relative), b"owned backup").unwrap();

        write_transaction(
            &root,
            DogfoodTransaction::Start(Box::new(StartTransaction {
                phase: StartPhase::BackupCreated,
                previous_state_existed: false,
                previous_state: DogfoodState::default(),
                prior_autopilot: FileSnapshot::Missing,
                prior_gate_mode: FileSnapshot::Missing,
                backup_relative_path: backup_relative.clone(),
                qualification_bundle_relative_path: format!(
                    ".brainmap/dogfood/{run_id}/qualification"
                ),
                intended_run: None,
            })),
        )
        .unwrap();

        status(Some(root.clone())).unwrap();
        assert_eq!(fs::read(bundle.join("sentinel")).unwrap(), b"keep");
        assert!(!root.join(backup_relative).exists());
        assert!(!transaction_path(&root).exists());
    }

    fn write_test_qualification_bundle(path: &Path) {
        let running = qualification::running_candidate_hashes().unwrap();
        let fixture = crate::dogfood::qualification_test_support::ValidBundle::new_for(
            COMMIT,
            running.brainmap_sha256,
            running.brainmapd_sha256,
        );
        copy_tree(&fixture.bundle, path);
    }

    fn test_start_args(
        root: &Path,
        qualification_bundle: &Path,
        started_at: DateTime<Utc>,
    ) -> DogfoodStartArgs {
        DogfoodStartArgs {
            candidate_commit: COMMIT.into(),
            adapter: "codex".into(),
            started_at: format_timestamp(started_at),
            qualification_bundle: qualification_bundle.to_path_buf(),
            vault: Some(root.to_path_buf()),
        }
    }

    fn prepare_qualifiable_vault(root: &Path, parent: &Path) -> DateTime<Utc> {
        let (started_at, finalized_at) = prepare_intensive_vault_without_review(root, parent);
        review_at(
            DogfoodReviewArgs {
                incident_status: "clear".into(),
                vault: Some(root.to_path_buf()),
            },
            started_at + Duration::minutes(61),
        )
        .unwrap();
        finalized_at
    }

    fn prepare_intensive_vault_without_review(
        root: &Path,
        parent: &Path,
    ) -> (DateTime<Utc>, DateTime<Utc>) {
        let qualification_bundle =
            parent.join(format!("qualification-{}", util::id("fixture", "dogfood")));
        vault::init_vault(Some(root.to_path_buf()), false, true).unwrap();
        let started_at = Utc::now() - Duration::hours(2);
        write_test_qualification_bundle(&qualification_bundle);
        start_at_with_test_verifier(
            DogfoodStartArgs {
                candidate_commit: COMMIT.into(),
                adapter: "codex".into(),
                started_at: format_timestamp(started_at),
                qualification_bundle,
                vault: Some(root.to_path_buf()),
            },
            started_at,
            None,
        )
        .unwrap();
        let context = active_run_context(root).unwrap().unwrap();
        let ledger = root.join(LEDGER_RELATIVE_PATH);
        for index in 0..MINIMUM_COMPLETE_PAIRS {
            let gate_at = started_at + Duration::minutes(index as i64 * 2 + 1);
            let action_at = gate_at + Duration::minutes(1);
            util::append_jsonl(
                &ledger,
                &json!({
                    "kind": "decision-gate",
                    "id": format!("fault-gate-{index}"),
                    "dogfoodRunId": context.run_id.as_str(),
                    "createdAt": format_timestamp(gate_at),
                    "outcome": "ask_user",
                    "predictedOutcome": "proceed",
                    "predictedSelectedOption": "local-option",
                    "matchKind": "exact",
                    "candidateCollision": false,
                    "matchMargin": 0.5,
                    "evaluationLatencyMicros": 100,
                    "gateMode": context.gate_mode.as_str(),
                    "autopilotMode": context.autopilot_mode.as_str(),
                    "autopilotLevel": context.autopilot_level.as_str(),
                    "dogfoodThreshold": context.threshold,
                    "dogfoodCandidateBinarySha256": context.candidate_binary_sha256.as_str(),
                    "situation": format!("qualification scenario {}", index % 5),
                    "scope": format!("project:qualification-{}", index % 3),
                    "decisionType": format!("tooling-{}", index % 2)
                }),
            )
            .unwrap();
            util::append_jsonl(
                &ledger,
                &json!({
                    "kind": "record-decision",
                    "id": format!("fault-action-{index}"),
                    "decisionId": format!("fault-gate-{index}"),
                    "dogfoodRunId": context.run_id.as_str(),
                    "createdAt": format_timestamp(action_at),
                    "chosen": "local-option",
                    "wasAsked": false
                }),
            )
            .unwrap();
        }
        (started_at, started_at + Duration::minutes(62))
    }

    #[test]
    fn gate_context_cache_is_versioned_by_state_file_identity() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        GATE_CONTEXT_LOADS.store(0, Ordering::SeqCst);

        assert!(active_run_context_for_gate(&root).unwrap().is_none());
        assert!(active_run_context_for_gate(&root).unwrap().is_none());
        assert_eq!(GATE_CONTEXT_LOADS.load(Ordering::SeqCst), 1);

        write_state(&root, &DogfoodState::default()).unwrap();
        assert!(active_run_context_for_gate(&root).unwrap().is_none());
        assert!(active_run_context_for_gate(&root).unwrap().is_none());
        assert_eq!(GATE_CONTEXT_LOADS.load(Ordering::SeqCst), 2);

        util::remove_file_and_sync(&state_path(&root)).unwrap();
        assert!(active_run_context_for_gate(&root).unwrap().is_none());
        assert_eq!(GATE_CONTEXT_LOADS.load(Ordering::SeqCst), 3);

        let provenance_before = capture_gate_provenance_version(&root).unwrap();
        util::write_atomic(&root.join(".brainmap/gate-mode"), b"active\n").unwrap();
        let provenance_after = capture_gate_provenance_version(&root).unwrap();
        assert_ne!(provenance_before, provenance_after);
    }

    #[test]
    fn confirmed_collision_is_retained_but_not_an_automatic_candidate_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        prepare_intensive_vault_without_review(&root, tmp.path());
        let state = load_state(&root).unwrap();
        let run = state.runs.last().unwrap();
        let status = learning::autopilot_status_value(&root).unwrap();
        let mut metrics = status["shadowMetrics"].clone();
        metrics["confirmedCollisions"] = serde_json::json!(1);

        let safety = validate_metrics(&metrics, &run.run_id).unwrap();

        assert_eq!(safety.confirmed_collisions, 1);
        assert!(safety.passed);
        assert!(metrics_safety_clean(&metrics));
    }

    fn active_review_summary(root: &Path, ended_at: DateTime<Utc>) -> Result<ReviewSummary> {
        let state = load_state(root)?;
        let run = &state.runs[active_run_index(&state)?];
        let ledger = fs::read(root.join(LEDGER_RELATIVE_PATH))?;
        validate_review_receipts(root, run, &ledger, ended_at)
    }

    #[test]
    fn tampered_review_prefix_blocks_finalization_without_partial_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("TamperedVault");
        let finalized_at = prepare_qualifiable_vault(&root, tmp.path());
        let ledger_path = root.join(LEDGER_RELATIVE_PATH);
        let mut lines = fs::read_to_string(&ledger_path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut changed = false;
        for line in &mut lines {
            let mut value: Value = serde_json::from_str(line).unwrap();
            if value.get("kind").and_then(Value::as_str) == Some("dogfood-review") {
                value["ledgerPrefixSha256"] = Value::String("0".repeat(64));
                *line = serde_json::to_string(&value).unwrap();
                changed = true;
                break;
            }
        }
        assert!(changed);
        fs::write(&ledger_path, format!("{}\n", lines.join("\n"))).unwrap();

        let out = tmp.path().join("tampered-evidence");
        let error = finalize_at(
            DogfoodFinalizeArgs {
                out: out.clone(),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(root.clone()),
            },
            finalized_at,
        )
        .unwrap_err();
        assert!(error.to_string().contains("ledger prefix integrity failed"));
        assert!(!out.exists());
        assert!(!transaction_path(&root).exists());
        assert!(active_run_context(&root).unwrap().is_some());
    }

    #[test]
    fn finalization_requires_a_final_covering_prompt_free_review() {
        let tmp = tempfile::tempdir().unwrap();
        let missing_root = tmp.path().join("MissingReviewVault");
        let (_, finalized_at) = prepare_intensive_vault_without_review(&missing_root, tmp.path());
        let missing_error = finalize_at(
            DogfoodFinalizeArgs {
                out: tmp.path().join("missing-review-evidence"),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(missing_root.clone()),
            },
            finalized_at,
        )
        .unwrap_err();
        assert!(
            missing_error
                .to_string()
                .contains("requires a persisted prompt-free review receipt")
        );
        assert!(active_run_context(&missing_root).unwrap().is_some());

        let stale_root = tmp.path().join("StaleReviewVault");
        let (started_at, finalized_at) =
            prepare_intensive_vault_without_review(&stale_root, tmp.path());
        review_at(
            DogfoodReviewArgs {
                incident_status: "clear".into(),
                vault: Some(stale_root.clone()),
            },
            started_at + Duration::minutes(61),
        )
        .unwrap();
        util::append_jsonl(
            &stale_root.join(LEDGER_RELATIVE_PATH),
            &json!({"kind": "qualification-note", "id": "post-review-event"}),
        )
        .unwrap();
        let stale_error = finalize_at(
            DogfoodFinalizeArgs {
                out: tmp.path().join("stale-review-evidence"),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(stale_root.clone()),
            },
            finalized_at,
        )
        .unwrap_err();
        assert!(
            stale_error
                .to_string()
                .contains("must cover the complete qualification ledger")
        );
        assert!(active_run_context(&stale_root).unwrap().is_some());
    }

    #[test]
    fn incident_state_machine_resolves_investigations_and_enforces_terminal_failure() {
        let tmp = tempfile::tempdir().unwrap();

        let resolved_root = tmp.path().join("ResolvedVault");
        let (started_at, finalized_at) =
            prepare_intensive_vault_without_review(&resolved_root, tmp.path());
        let ledger_before_early = fs::read(resolved_root.join(LEDGER_RELATIVE_PATH)).unwrap();
        let early_review = review_at(
            DogfoodReviewArgs {
                incident_status: "clear".into(),
                vault: Some(resolved_root.clone()),
            },
            started_at - Duration::seconds(1),
        )
        .unwrap_err();
        assert!(early_review.to_string().contains("cannot precede"));
        assert_eq!(
            fs::read(resolved_root.join(LEDGER_RELATIVE_PATH)).unwrap(),
            ledger_before_early
        );
        let invalid_first = review_at(
            DogfoodReviewArgs {
                incident_status: "resolved-no-violation".into(),
                vault: Some(resolved_root.clone()),
            },
            started_at + Duration::minutes(60) + Duration::seconds(10),
        )
        .unwrap_err();
        assert!(
            invalid_first
                .to_string()
                .contains("incident-state transition")
        );
        review_at(
            DogfoodReviewArgs {
                incident_status: "investigating".into(),
                vault: Some(resolved_root.clone()),
            },
            started_at + Duration::minutes(60) + Duration::seconds(10),
        )
        .unwrap();
        let ledger_before_duplicate = fs::read(resolved_root.join(LEDGER_RELATIVE_PATH)).unwrap();
        let duplicate_time = review_at(
            DogfoodReviewArgs {
                incident_status: "investigating".into(),
                vault: Some(resolved_root.clone()),
            },
            started_at + Duration::minutes(60) + Duration::seconds(10),
        )
        .unwrap_err();
        assert!(
            duplicate_time
                .to_string()
                .contains("timestamps must be strictly increasing")
        );
        assert_eq!(
            fs::read(resolved_root.join(LEDGER_RELATIVE_PATH)).unwrap(),
            ledger_before_duplicate
        );
        let invalid_clear = review_at(
            DogfoodReviewArgs {
                incident_status: "clear".into(),
                vault: Some(resolved_root.clone()),
            },
            started_at + Duration::minutes(60) + Duration::seconds(20),
        )
        .unwrap_err();
        assert!(
            invalid_clear
                .to_string()
                .contains("incident-state transition")
        );
        review_at(
            DogfoodReviewArgs {
                incident_status: "resolved-no-violation".into(),
                vault: Some(resolved_root.clone()),
            },
            started_at + Duration::minutes(61),
        )
        .unwrap();
        let summary = active_review_summary(&resolved_root, finalized_at).unwrap();
        assert!(summary.review_ready);
        assert!(summary.final_review_covers_ledger);
        assert_eq!(
            summary.current_incident_status,
            Some(IncidentStatus::ResolvedNoViolation)
        );

        let unresolved_root = tmp.path().join("UnresolvedVault");
        let (unresolved_start, unresolved_end) =
            prepare_intensive_vault_without_review(&unresolved_root, tmp.path());
        review_at(
            DogfoodReviewArgs {
                incident_status: "investigating".into(),
                vault: Some(unresolved_root.clone()),
            },
            unresolved_start + Duration::minutes(61),
        )
        .unwrap();
        let unresolved_error = finalize_at(
            DogfoodFinalizeArgs {
                out: tmp.path().join("unresolved-evidence"),
                signer: "Local developer".into(),
                incident_disposition: "Investigation remains open".into(),
                vault: Some(unresolved_root),
            },
            unresolved_end,
        )
        .unwrap_err();
        assert!(
            unresolved_error
                .to_string()
                .contains("investigation must be resolved")
        );

        let failed_root = tmp.path().join("FailedVault");
        let (failed_start, failed_end) =
            prepare_intensive_vault_without_review(&failed_root, tmp.path());
        review_at(
            DogfoodReviewArgs {
                incident_status: "candidate-failed".into(),
                vault: Some(failed_root.clone()),
            },
            failed_start + Duration::minutes(61),
        )
        .unwrap();
        let ledger_before = fs::read(failed_root.join(LEDGER_RELATIVE_PATH)).unwrap();
        let terminal_error = review_at(
            DogfoodReviewArgs {
                incident_status: "clear".into(),
                vault: Some(failed_root.clone()),
            },
            failed_start + Duration::minutes(61) + Duration::seconds(1),
        )
        .unwrap_err();
        assert!(
            terminal_error
                .to_string()
                .contains("incident-state transition")
        );
        assert_eq!(
            fs::read(failed_root.join(LEDGER_RELATIVE_PATH)).unwrap(),
            ledger_before
        );
        let failed_error = finalize_at(
            DogfoodFinalizeArgs {
                out: tmp.path().join("failed-evidence"),
                signer: "Local developer".into(),
                incident_disposition: "Candidate failed".into(),
                vault: Some(failed_root),
            },
            failed_end,
        )
        .unwrap_err();
        assert!(failed_error.to_string().contains("candidate was failed"));
    }

    #[test]
    fn concurrent_review_and_finalize_leave_one_coherent_state() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ConcurrentVault");
        let out = tmp.path().join("concurrent-evidence");
        let finalized_at = prepare_qualifiable_vault(&root, tmp.path());
        let barrier = Arc::new(Barrier::new(3));
        let review_barrier = Arc::clone(&barrier);
        let review_root = root.clone();
        let review = thread::spawn(move || {
            review_barrier.wait();
            review_at(
                DogfoodReviewArgs {
                    incident_status: "clear".into(),
                    vault: Some(review_root),
                },
                finalized_at - Duration::seconds(30),
            )
        });
        let finalize_barrier = Arc::clone(&barrier);
        let finalize_root = root.clone();
        let finalize_out = out.clone();
        let finalize = thread::spawn(move || {
            finalize_barrier.wait();
            finalize_at(
                DogfoodFinalizeArgs {
                    out: finalize_out,
                    signer: "Local developer".into(),
                    incident_disposition: "No incidents observed".into(),
                    vault: Some(finalize_root),
                },
                finalized_at,
            )
        });
        barrier.wait();
        let review_result = review.join().unwrap();
        let finalize_result = finalize.join().unwrap();
        assert!(review_result.is_ok() || finalize_result.is_ok());
        assert!(!transaction_path(&root).exists());

        let ledger = fs::read(root.join(LEDGER_RELATIVE_PATH)).unwrap();
        for line in ledger.split(|byte| *byte == b'\n') {
            if !line.is_empty() {
                serde_json::from_slice::<Value>(line).unwrap();
            }
        }
        let state = load_state(&root).unwrap();
        let run = state.runs.last().unwrap();
        match run.status {
            DogfoodRunStatus::Finalized => {
                assert!(finalize_result.is_ok());
                verify_evidence_bundle(&root, &out, run).unwrap();
            }
            DogfoodRunStatus::Active => {
                assert!(review_result.is_ok());
                assert!(finalize_result.is_err());
                assert!(!out.exists());
                let summary = validate_review_receipts(&root, run, &ledger, finalized_at).unwrap();
                assert!(summary.integrity_valid);
            }
            DogfoodRunStatus::Aborted => panic!("concurrent operations aborted the run"),
        }
    }

    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            if source_path.is_dir() {
                copy_tree(&source_path, &target_path);
            } else {
                fs::copy(source_path, target_path).unwrap();
            }
        }
    }

    #[test]
    fn intensive_session_finalize_emits_verified_sanitized_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        let qualification_bundle = tmp.path().join("qualification");
        let out = tmp.path().join("dogfood-evidence");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = Utc::now();
        write_test_qualification_bundle(&qualification_bundle);
        start_at_with_test_verifier(
            DogfoodStartArgs {
                candidate_commit: COMMIT.into(),
                adapter: "codex".into(),
                started_at: format_timestamp(started_at),
                qualification_bundle,
                vault: Some(root.clone()),
            },
            started_at,
            None,
        )
        .unwrap();
        let context = active_run_context(&root).unwrap().unwrap();
        let finalized_at = started_at + Duration::minutes(62);
        let private_prompt = "private local prompt that must not enter aggregate evidence";
        let ledger_path = root.join(LEDGER_RELATIVE_PATH);
        for index in 0..MINIMUM_COMPLETE_PAIRS {
            let timestamp = started_at + Duration::minutes(index as i64 * 2 + 1);
            util::append_jsonl(
                &ledger_path,
                &json!({
                    "kind": "decision-gate",
                    "id": format!("dec_dogfood_{index}"),
                    "dogfoodRunId": context.run_id.as_str(),
                    "createdAt": format_timestamp(timestamp),
                    "outcome": "ask_user",
                    "predictedOutcome": "proceed",
                    "predictedSelectedOption": "local-option",
                    "matchKind": "exact",
                    "candidateCollision": false,
                    "matchMargin": 0.5,
                    "evaluationLatencyMicros": 1200,
                    "gateMode": context.gate_mode.as_str(),
                    "autopilotMode": context.autopilot_mode.as_str(),
                    "autopilotLevel": context.autopilot_level.as_str(),
                    "dogfoodThreshold": context.threshold,
                    "dogfoodCandidateBinarySha256": context.candidate_binary_sha256.as_str(),
                    "situation": if index % 5 == 0 { private_prompt.to_string() } else { format!("local qualification scenario {}", index % 5) },
                    "scope": format!("project:qualification-{}", index % 3),
                    "decisionType": format!("tooling-{}", index % 2)
                }),
            )
            .unwrap();
        }

        let missing_action_out = tmp.path().join("missing-action-evidence");
        let missing_action_error = finalize_at(
            DogfoodFinalizeArgs {
                out: missing_action_out.clone(),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(root.clone()),
            },
            finalized_at,
        )
        .unwrap_err();
        assert!(
            missing_action_error.to_string().contains("action coverage"),
            "unexpected finalization error: {missing_action_error:#}"
        );
        assert!(!missing_action_out.exists());
        assert!(active_run_context(&root).unwrap().is_some());

        for index in 0..MINIMUM_COMPLETE_PAIRS {
            let timestamp = started_at + Duration::minutes(index as i64 * 2 + 2);
            util::append_jsonl(
                &ledger_path,
                &json!({
                    "kind": "record-decision",
                    "id": format!("action_dogfood_{index}"),
                    "decisionId": format!("dec_dogfood_{index}"),
                    "dogfoodRunId": context.run_id.as_str(),
                    "createdAt": format_timestamp(timestamp),
                    "chosen": "local-option",
                    "wasAsked": false
                }),
            )
            .unwrap();
        }
        review_at(
            DogfoodReviewArgs {
                incident_status: "clear".into(),
                vault: Some(root.clone()),
            },
            started_at + Duration::minutes(61),
        )
        .unwrap();

        fs::write(root.join(".brainmap/gate-mode"), "active\n").unwrap();
        let drift_out = tmp.path().join("drift-evidence");
        let drift_error = finalize_at(
            DogfoodFinalizeArgs {
                out: drift_out.clone(),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(root.clone()),
            },
            finalized_at,
        )
        .unwrap_err();
        assert!(drift_error.to_string().contains("gate mode drift"));
        assert!(!drift_out.exists());
        assert!(active_run_context(&root).unwrap().is_some());
        fs::write(root.join(".brainmap/gate-mode"), "shadow\n").unwrap();

        let overlapping_out = root.join("dogfood-evidence");
        let overlap_error = finalize_at(
            DogfoodFinalizeArgs {
                out: overlapping_out.clone(),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(root.clone()),
            },
            finalized_at,
        )
        .unwrap_err();
        assert!(
            overlap_error
                .to_string()
                .contains("must not overlap the vault")
        );
        assert!(!overlapping_out.exists());
        assert!(active_run_context(&root).unwrap().is_some());

        finalize_at(
            DogfoodFinalizeArgs {
                out: out.clone(),
                signer: "Local developer".into(),
                incident_disposition: "No incidents observed".into(),
                vault: Some(root.clone()),
            },
            finalized_at,
        )
        .unwrap();

        let json_path = out.join("dogfood-qualification.json");
        let markdown_path = out.join("dogfood-qualification.md");
        let export_relative = format!("99-meta/backups/{}-final.brainmap.tar.zst", context.run_id);
        let export_path = root.join(&export_relative);
        let qualification_path = out.join("qualification");
        let checksums_path = out.join("SHA256SUMS");
        for path in [&json_path, &markdown_path, &export_path, &checksums_path] {
            assert!(path.is_file(), "missing {}", path.display());
        }
        assert!(!out.join("dogfood-final.brainmap.tar.zst").exists());
        assert!(
            collect_evidence_files(&out)
                .unwrap()
                .iter()
                .all(|path| !path.ends_with(".tar.zst"))
        );
        assert!(qualification_path.is_dir());
        assert!(!out.join("fia-manifest.json").exists());
        export::verify_export_archive(&export_path, None).unwrap();
        verify_checksum_file(&out, &checksums_path);

        let report_text = fs::read_to_string(&json_path).unwrap();
        assert!(!report_text.contains(private_prompt));
        assert!(!report_text.contains(tmp.path().to_string_lossy().as_ref()));
        assert!(!report_text.contains(&export_relative));
        let markdown_text = fs::read_to_string(&markdown_path).unwrap();
        assert!(!markdown_text.contains(private_prompt));
        assert!(!markdown_text.contains(tmp.path().to_string_lossy().as_ref()));
        let report: Value = serde_json::from_str(&report_text).unwrap();
        assert_eq!(report["status"], "passed");
        assert_eq!(report["candidateCommit"], COMMIT);
        assert_eq!(report["adapter"], "codex");
        assert_eq!(report["durationSeconds"], 3_720);
        assert_eq!(report["shadowMetrics"]["runId"], context.run_id);
        assert_eq!(report["shadowMetrics"]["decisions"], 30);
        assert_eq!(report["shadowMetrics"]["completeGateActionPairs"], 30);
        assert_eq!(report["shadowMetrics"]["distinctDecisionScenarios"], 5);
        assert_eq!(report["shadowMetrics"]["distinctScopes"], 3);
        assert_eq!(report["shadowMetrics"]["distinctDecisionTypes"], 2);
        assert_eq!(
            report["shadowMetrics"]["intensiveSessionDistributionValid"],
            true
        );
        assert!(
            report["shadowMetrics"]
                .get("distinctObservationBuckets")
                .is_none()
        );
        assert!(report["shadowMetrics"].get("eventSpanSeconds").is_none());
        assert!(report.get("plannedEnd").is_none());
        assert_eq!(report["shadowMetrics"]["rawPromptsRetained"], false);
        assert_eq!(report["safety"]["passed"], true);
        assert_eq!(report["safety"]["confirmedCollisions"], 0);
        assert_eq!(report["reviewSummary"]["reviewCount"], 1);
        assert_eq!(report["reviewSummary"]["currentIncidentStatus"], "clear");
        assert_eq!(report["reviewSummary"]["finalReviewCoversLedger"], true);
        assert_eq!(report["reviewSummary"]["reviewReady"], true);
        assert_eq!(report["signOff"]["signer"], "Local developer");
        assert_eq!(
            report["signOff"]["incidentDisposition"],
            "No incidents observed"
        );
        assert_eq!(report["schemaVersion"], "brainmap-dogfood-qualification-v3");
        assert!(report.get("finalExport").is_none());
        assert_eq!(
            report["finalBackupSha256"],
            file_sha256(&export_path).unwrap()
        );
        assert_eq!(report["qualification"]["relativePath"], "qualification");
        assert_eq!(
            report["qualification"]["manifestSha256"],
            file_sha256(&qualification_path.join("qualification.json")).unwrap()
        );

        let state = load_state(&root).unwrap();
        let run = state.runs.last().unwrap();
        assert_eq!(
            report["qualification"]["runRelativePath"],
            run.qualification_bundle_relative_path
        );
        assert_eq!(run.status, DogfoodRunStatus::Finalized);
        assert_eq!(run.finalized_at, Some(finalized_at));
        assert_eq!(
            run.final_export.as_ref().unwrap().sha256,
            file_sha256(&export_path).unwrap()
        );
        assert_eq!(
            report["qualification"]["bundleSha256"],
            run.qualification_bundle_sha256
        );
        verify_evidence_bundle(&root, &out, run).unwrap();
        assert!(active_run_context(&root).unwrap().is_none());

        let restored = tmp.path().join("Restored");
        export::import_cmd(crate::cli::ImportArgs {
            file: export_path,
            to: restored.clone(),
            dry_run: false,
            identity: None,
        })
        .unwrap();
        let restored_state = load_state(&restored).unwrap();
        assert_eq!(
            restored_state.runs.last().unwrap().status,
            DogfoodRunStatus::Finalized
        );
        assert!(active_run_context(&restored).unwrap().is_none());
    }

    fn verify_checksum_file(directory: &Path, checksum_file: &Path) {
        let text = fs::read_to_string(checksum_file).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert!(lines.len() > 4);
        let mut checked = BTreeSet::new();
        let mut ordered_names = Vec::new();
        for line in lines {
            let (expected, file_name) = line.split_once("  ").unwrap();
            assert_eq!(file_sha256(&directory.join(file_name)).unwrap(), expected);
            checked.insert(file_name.to_string());
            ordered_names.push(file_name.to_string());
        }
        assert!(ordered_names.windows(2).all(|pair| pair[0] < pair[1]));
        let expected = collect_evidence_files(directory)
            .unwrap()
            .into_iter()
            .filter(|path| path != "SHA256SUMS")
            .collect::<BTreeSet<_>>();
        assert_eq!(checked, expected);
    }
}
