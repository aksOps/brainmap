use crate::{privacy, util};
use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::sync::OnceLock;

const ROOT_SCHEMA: &str = "brainmap-m8-qualification-bundle-v1";
const LEGACY_SCHEMA: &str = "brainmap-m8-fia-v1";
const MAX_FILES: usize = 512;
const MAX_ENTRIES: usize = 1_024;
const MAX_DEPTH: usize = 8;
const MAX_PATH_BYTES: usize = 240;
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const QUALIFYING_CODEX_VERSION: &str = "codex-cli 0.144.0";
const QUALIFYING_CODEX_TARGET: &str = "x86_64-unknown-linux-musl";
const QUALIFYING_CODEX_ARCHIVE_SHA256: &str =
    "6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd";
const QUALIFYING_CODEX_BINARY_SHA256: &str =
    "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QualificationCandidate {
    pub commit: String,
    pub brainmap_sha256: String,
    pub brainmapd_sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedQualification {
    pub schema_version: &'static str,
    pub verified: bool,
    pub candidate: QualificationCandidate,
    pub fias: Vec<&'static str>,
    pub bundle_sha256: String,
    #[serde(skip_serializing)]
    build_provenance: VerifiedBuildProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedBuildProvenance {
    build_info_sha256: String,
    producer_digests: ReproProducerDigests,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunningCandidateHashes {
    pub(crate) brainmap_sha256: String,
    pub(crate) brainmapd_sha256: String,
}

#[derive(Debug)]
struct FileInfo {
    sha256: String,
    text: String,
}

#[derive(Debug)]
struct Inventory {
    files: BTreeMap<String, FileInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactRef {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrivacyClaims {
    raw_prompts_retained: bool,
    secrets_retained: bool,
    private_paths_retained: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SyntheticPrivacyClaims {
    raw_prompts_retained: bool,
    secrets_retained: bool,
    private_paths_retained: bool,
    synthetic_inputs_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootManifest {
    schema_version: String,
    candidate: QualificationCandidate,
    evidence: RootEvidence,
    privacy: PrivacyClaims,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RootEvidence {
    reproducibility_manifest: ArtifactRef,
    runner_manifest: ArtifactRef,
    runner_checksums: ArtifactRef,
    host_manifest: ArtifactRef,
    host_checksums: ArtifactRef,
    release_manifest: ArtifactRef,
    release_checksums: ArtifactRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReproducibilityManifest {
    schema_version: String,
    candidate_commit: String,
    profile: String,
    locked: bool,
    two_root_byte_identical: bool,
    clean_tree: bool,
    brainmap_sha256: String,
    brainmapd_sha256: String,
    build_info_sha256: String,
    producer_digests: ReproProducerDigests,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReproProducerDigests {
    integrated_qualification_sha256: String,
    codex_fia5_sha256: String,
    release_qualification_sha256: String,
    assemble_qualification_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerManifest {
    schema_version: String,
    qualification_eligible: bool,
    result: String,
    candidate: QualificationCandidate,
    started_at: String,
    completed_at: String,
    execution_mode: String,
    provenance: RunnerProvenance,
    build: RunnerBuild,
    commands: ArtifactRef,
    reports: RunnerReports,
    privacy: SyntheticPrivacyClaims,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerProvenance {
    host: HostPlatform,
    qualification_environment: HostPlatform,
    container: ContainerProvenance,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostPlatform {
    kernel_name: String,
    kernel_release: String,
    architecture: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContainerProvenance {
    image: String,
    image_id: String,
    network: String,
    root_filesystem: String,
    capabilities: String,
    no_new_privileges: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerBuild {
    profile: String,
    locked: bool,
    two_root_byte_identical: bool,
    reproducibility_manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerReports {
    fia1: ArtifactRef,
    fia2: ArtifactRef,
    fia3: ArtifactRef,
    fia4: ArtifactRef,
    fia6: ArtifactRef,
    fia7: ArtifactRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunnerCommand {
    sequence: usize,
    fia: String,
    id: String,
    command: String,
    expected_exit: Value,
    exit_code: i64,
    passed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fia1Report {
    answers: u64,
    previews: u64,
    approved_packets: u64,
    automatic_rebuild: bool,
    behavior_derived: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fia2Report {
    exact: u64,
    paraphrases: u64,
    negatives: u64,
    correct_predictions: u64,
    non_leaks: u64,
    negatives_retained_compatible_learned_options: bool,
    rule_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fia3Report {
    non_dry_decision: bool,
    action_recorded: bool,
    previewed: bool,
    approved: bool,
    before_choice: String,
    after_choice: String,
    scope_isolation: bool,
    relevance_isolation: bool,
    more_relevant_competing_choice: String,
    more_relevant_competing_rule_wins: bool,
    decision_id: String,
    before_rule: String,
    after_rule: String,
    more_relevant_rule: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fia4Report {
    added: bool,
    rebuilt_active: bool,
    active_prediction: String,
    active_decoy_policy: bool,
    exact_causal_policy_set: bool,
    causally_named: bool,
    unrelated_not_named: bool,
    retired: bool,
    rebuilt_retired: bool,
    retired_not_applied: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fia6Report {
    operating_system_processes: u64,
    gate_processes: u64,
    record_processes: u64,
    capture_processes: u64,
    feedback_processes: u64,
    ledger_events: u64,
    unique_ledger_ids: u64,
    capture_events: u64,
    unique_capture_ids: u64,
    applied_packets: u64,
    canonical_notes: u64,
    pending_packets: u64,
    gate_record_overlap_barrier: bool,
    capture_feedback_overlap_barrier: bool,
    simultaneous_gate_record_workers: u64,
    simultaneous_capture_feedback_workers: u64,
    jsonl_complete: bool,
    notes_complete: bool,
    ledger_sha256: String,
    capture_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fia7Report {
    export_verified: bool,
    archive_sha256: String,
    old_tree_hash: String,
    new_tree_hash: String,
    behavior_pairs: u64,
    fault_phases: u64,
    canonical_fault_states: u64,
    behavior_equivalent: bool,
    learned_equivalent: bool,
    corrected_equivalent: bool,
    policy_equivalent: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostManifest {
    schema_version: String,
    qualification_eligible: bool,
    mode: String,
    candidate: QualificationCandidate,
    started_at: String,
    completed_at: String,
    adapter: HostAdapter,
    provenance: HostProvenance,
    artifacts: HostArtifacts,
    privacy: SyntheticPrivacyClaims,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostAdapter {
    target: String,
    host_version: String,
    launch_mode: String,
    trust_bypass_used: bool,
    persisted_hook_accepted: bool,
    project_trusted: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostProvenance {
    kernel_name: String,
    kernel_release: String,
    architecture: String,
    configured_brainmap_sha256: String,
    configured_brainmapd_sha256: String,
    codex_target: String,
    official_codex_archive_sha256: String,
    official_codex_binary_sha256: String,
    observed_codex_binary_sha256: String,
    official_codex_verified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostArtifacts {
    events: ArtifactRef,
    install_dry_run: ArtifactRef,
    doctor: ArtifactRef,
    host_observation: ArtifactRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostInstallDryRun {
    schema_version: String,
    target: String,
    dry_run: bool,
    candidate: QualificationCandidate,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostDoctor {
    schema_version: String,
    target: String,
    healthy: bool,
    health_scope: String,
    host_hook_trust_verified: bool,
    host_probe_required: bool,
    candidate: QualificationCandidate,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostEvent {
    sequence: usize,
    kind: String,
    success: bool,
    #[serde(default)]
    decision_id: Option<String>,
    #[serde(default)]
    packet_id: Option<String>,
    #[serde(default)]
    changed: Option<bool>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    selected_option: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostObservation {
    schema_version: String,
    qualification_eligible: bool,
    mode: String,
    candidate: QualificationCandidate,
    official_codex: HostOfficialCodex,
    config: HostSafeConfig,
    launch: HostLaunch,
    hooks: HostHooks,
    calls: HostCalls,
    ledger: HostLedger,
    project: HostProject,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostOfficialCodex {
    version: String,
    target: String,
    archive_sha256: String,
    binary_sha256: String,
    observed_binary_sha256: String,
    archive_verified: bool,
    binary_verified: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostSafeConfig {
    approval_policy: String,
    approvals_reviewer: String,
    sandbox_mode: String,
    workspace_write_network_access: bool,
    bypass_hook_trust: bool,
    bypass_approvals_and_sandbox: bool,
    feedback_approval_mode: String,
    apply_approval_mode: String,
    codex_home_sha256: String,
    gate_mode: String,
    autopilot_mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostLaunch {
    launcher_sha256: String,
    argv_sha256: String,
    argv: Vec<HostArgDescriptor>,
    app_server_argv_sha256: String,
    app_server_argv: Vec<HostArgDescriptor>,
    codex_home_bound: bool,
    project_inventory_bound: bool,
    session: HostSession,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostArgDescriptor {
    position: usize,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    literal: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostSession {
    source: String,
    id_sha256: String,
    created_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostHooks {
    trusted_hook_count: u64,
    entries: Vec<HostHookEntry>,
    executed_hook_gate_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostHookEntry {
    event_name: String,
    current_hash: String,
    trust_status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostCalls {
    count: u64,
    order: Vec<String>,
    first: HostFirstCall,
    feedback: HostFeedbackCall,
    second: HostSecondCall,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostFirstCall {
    decision_id: String,
    outcome: String,
    selected_option: Option<String>,
    action: HostAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostFeedbackCall {
    packet_id: String,
    previewed: bool,
    approved: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostSecondCall {
    decision_id: String,
    outcome: String,
    selected_option: String,
    changed: bool,
    action: HostAction,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostAction {
    chosen: String,
    was_asked: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostLedger {
    correlation: String,
    correlated_event_count: u64,
    post_boundary_event_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostProject {
    inventory_sha256: String,
    workflow_sha256: String,
    unchanged: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseManifest {
    schema_version: String,
    qualification_eligible: bool,
    candidate: QualificationCandidate,
    source_tree_dirty_before: bool,
    source_tree_dirty_after: bool,
    started_at: String,
    completed_at: String,
    host: HostPlatform,
    toolchain: Toolchain,
    reproducibility_manifest_sha256: String,
    gates: ReleaseGates,
    privacy: SyntheticPrivacyClaims,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Toolchain {
    rustc: String,
    cargo: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseGates {
    format: ArtifactRef,
    clippy: ArtifactRef,
    workspace_tests: ArtifactRef,
    audit: ArtifactRef,
    deny: ArtifactRef,
    sbom: ArtifactRef,
    locked_release_build: ArtifactRef,
    package_smoke: ArtifactRef,
    scale1000: ArtifactRef,
    scale5000: ArtifactRef,
    performance: ArtifactRef,
    clean_worktree: ArtifactRef,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseGateResult {
    schema_version: String,
    gate: String,
    command_id: String,
    passed: bool,
    exit_code: i64,
    log_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseQualificationManifest {
    schema_version: String,
    source_commit: String,
    source_tree_dirty: bool,
    started_at: String,
    completed_at: String,
    host: String,
    toolchain: Toolchain,
    binaries: ReleaseQualificationBinaries,
    portable_archive_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReleaseQualificationBinaries {
    brainmap_sha256: String,
    brainmapd_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RestoreFaultObservation {
    phase: String,
    complete_state: String,
    tree_hash: String,
    old_tree_hash: String,
    new_tree_hash: String,
}

pub fn verify_bundle(bundle: &Path) -> Result<VerifiedQualification> {
    let inventory = build_inventory(bundle)?;
    verify_inventory(&inventory)
}

fn verify_inventory(inventory: &Inventory) -> Result<VerifiedQualification> {
    let root_sums = verify_checksum_file(inventory, "SHA256SUMS", "")?;

    let root_value: Value = read_json(inventory, "qualification.json")?;
    let schema = root_value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if schema == LEGACY_SCHEMA {
        bail!("legacy flat FIA self-attestation is not accepted");
    }
    let root: RootManifest = read_json(inventory, "qualification.json")?;
    ensure!(
        root.schema_version == ROOT_SCHEMA,
        "unsupported qualification bundle schema: {}",
        root.schema_version
    );
    validate_candidate(&root.candidate, "root candidate")?;
    validate_privacy(&root.privacy, "root privacy")?;

    // Validate each recursive subtree manifest before consuming any reference
    // to it. This keeps a malformed checksum file from being reported as a
    // downstream reference mismatch and ensures its exact-coverage contract is
    // independently established.
    verify_checksum_file(inventory, "runner/SHA256SUMS", "runner/")?;
    verify_checksum_file(inventory, "host/SHA256SUMS", "host/")?;
    verify_checksum_file(inventory, "release/SHA256SUMS", "release/")?;

    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.reproducibility_manifest,
        "reproducibility/manifest.json",
    )?;
    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.runner_manifest,
        "runner/manifest.json",
    )?;
    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.runner_checksums,
        "runner/SHA256SUMS",
    )?;
    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.host_manifest,
        "host/manifest.json",
    )?;
    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.host_checksums,
        "host/SHA256SUMS",
    )?;
    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.release_manifest,
        "release/manifest.json",
    )?;
    verify_root_ref(
        inventory,
        &root_sums,
        &root.evidence.release_checksums,
        "release/SHA256SUMS",
    )?;

    let repro: ReproducibilityManifest = read_json(inventory, "reproducibility/manifest.json")?;
    validate_reproducibility(&repro, &root.candidate)?;
    let repro_sha = file(inventory, "reproducibility/manifest.json")?
        .sha256
        .as_str();

    let runner: RunnerManifest = read_json(inventory, "runner/manifest.json")?;
    validate_runner(inventory, &runner, &root.candidate, repro_sha)?;

    let host: HostManifest = read_json(inventory, "host/manifest.json")?;
    validate_host(inventory, &host, &root.candidate)?;

    let release: ReleaseManifest = read_json(inventory, "release/manifest.json")?;
    validate_release(inventory, &release, &root.candidate, repro_sha)?;

    let bundle_sha256 = file(inventory, "SHA256SUMS")?.sha256.clone();
    Ok(VerifiedQualification {
        schema_version: "brainmap-m8-qualification-verification-v1",
        verified: true,
        candidate: root.candidate,
        fias: vec![
            "FIA-1", "FIA-2", "FIA-3", "FIA-4", "FIA-5", "FIA-6", "FIA-7", "FIA-8",
        ],
        bundle_sha256,
        build_provenance: VerifiedBuildProvenance {
            build_info_sha256: repro.build_info_sha256,
            producer_digests: repro.producer_digests,
        },
    })
}

pub(crate) fn copy_verified_bundle(
    source: &Path,
    destination: &Path,
) -> Result<VerifiedQualification> {
    let source_inventory = build_inventory(source)?;
    let source_verified = verify_inventory(&source_inventory)?;
    match fs::symlink_metadata(destination) {
        Ok(_) => bail!(
            "qualification copy destination already exists: {}",
            destination.display()
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "inspect qualification copy destination {}",
                    destination.display()
                )
            });
        }
    }
    let destination_parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = fs::symlink_metadata(destination_parent).with_context(|| {
        format!(
            "inspect qualification copy parent {}",
            destination_parent.display()
        )
    })?;
    ensure!(
        parent_metadata.is_dir() && !parent_metadata.file_type().is_symlink(),
        "qualification copy parent must be an existing non-symlink directory"
    );

    fs::create_dir(destination).with_context(|| {
        format!(
            "create qualification copy destination {}",
            destination.display()
        )
    })?;
    let copy_result = copy_inventory(&source_inventory, destination).and_then(|()| {
        util::sync_directory(destination_parent)?;
        let copied = verify_bundle(destination)?;
        ensure!(
            copied.candidate == source_verified.candidate
                && copied.bundle_sha256 == source_verified.bundle_sha256
                && copied.build_provenance == source_verified.build_provenance,
            "qualification copy identity changed during preservation"
        );
        Ok(copied)
    });
    match copy_result {
        Ok(copied) => Ok(copied),
        Err(error) => {
            if let Err(cleanup_error) = fs::remove_dir_all(destination) {
                return Err(error).context(format!(
                    "remove incomplete qualification copy {}: {cleanup_error}",
                    destination.display()
                ));
            }
            if let Err(sync_error) = util::sync_directory(destination_parent) {
                return Err(error).context(format!(
                    "sync qualification copy parent after rollback {}: {sync_error}",
                    destination_parent.display()
                ));
            }
            Err(error)
        }
    }
}

fn copy_inventory(inventory: &Inventory, destination: &Path) -> Result<()> {
    let mut relative_directories = BTreeSet::new();
    for relative in inventory.files.keys() {
        let components = relative.split('/').collect::<Vec<_>>();
        for count in 1..components.len() {
            relative_directories.insert(components[..count].join("/"));
        }
    }

    let mut created_directories = vec![destination.to_path_buf()];
    for relative in relative_directories {
        let directory = destination.join(&relative);
        fs::create_dir(&directory)
            .with_context(|| format!("create qualification copy directory {relative}"))?;
        created_directories.push(directory);
    }
    for (relative, artifact) in &inventory.files {
        let target = destination.join(relative);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .with_context(|| format!("create qualification copy artifact {relative}"))?;
        file.write_all(artifact.text.as_bytes())
            .with_context(|| format!("write qualification copy artifact {relative}"))?;
        file.sync_all()
            .with_context(|| format!("sync qualification copy artifact {relative}"))?;
    }
    created_directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| right.cmp(left))
    });
    for directory in created_directories {
        util::sync_directory(&directory)?;
    }
    Ok(())
}

pub(crate) fn verify_cmd(bundle: &Path) -> Result<()> {
    let verified = verify_bundle(bundle)?;
    verify_running_qualification(&verified)?;
    println!("{}", serde_json::to_string_pretty(&verified)?);
    Ok(())
}

pub(crate) fn verify_running_qualification(
    verified: &VerifiedQualification,
) -> Result<RunningCandidateHashes> {
    let running = verify_running_candidate(&verified.candidate)?;
    let build_info_json = crate::build_info::build_info_json()?;
    ensure!(
        util::sha256_hex(build_info_json.as_bytes()) == verified.build_provenance.build_info_sha256,
        "qualification reproducibility build-info hash does not match the running candidate"
    );
    let embedded = crate::build_info::build_info().producer_digests;
    let expected = &verified.build_provenance.producer_digests;
    ensure!(
        embedded.integrated_qualification_sha256 == expected.integrated_qualification_sha256
            && embedded.codex_fia5_sha256 == expected.codex_fia5_sha256
            && embedded.release_qualification_sha256 == expected.release_qualification_sha256
            && embedded.assemble_qualification_sha256 == expected.assemble_qualification_sha256,
        "qualification producer digests do not match the running candidate"
    );
    Ok(running)
}

pub(crate) fn verify_running_candidate(
    candidate: &QualificationCandidate,
) -> Result<RunningCandidateHashes> {
    let running = running_candidate_hashes()?;
    ensure!(
        candidate.brainmap_sha256 == running.brainmap_sha256,
        "qualification candidate brainmap hash does not match the running brainmap binary"
    );
    ensure!(
        candidate.brainmapd_sha256 == running.brainmapd_sha256,
        "qualification candidate brainmapd hash does not match the companion brainmapd binary"
    );
    let build_info = crate::build_info::build_info();
    ensure!(
        build_info.qualification.eligible
            && build_info.qualification.release
            && build_info.qualification.locked
            && build_info.qualification.two_root_candidate
            && build_info.qualification.marker == crate::build_info::QUALIFICATION_MARKER,
        "candidate binary was not built by the clean locked two-root qualification workflow"
    );
    ensure!(
        build_info.cargo_profile == "release" && build_info.candidate_commit == candidate.commit,
        "qualification candidate commit/profile does not match embedded build provenance"
    );
    Ok(running)
}

pub(crate) fn running_candidate_hashes() -> Result<RunningCandidateHashes> {
    #[cfg(target_os = "linux")]
    let brainmap = Path::new("/proc/self/exe").to_path_buf();
    #[cfg(not(target_os = "linux"))]
    let brainmap = std::env::current_exe().context("locate running brainmap executable")?;

    let executable = std::env::current_exe().context("locate running brainmap executable")?;
    let executable_parent = executable
        .parent()
        .context("running brainmap executable has no parent directory")?;
    #[cfg(test)]
    let executable_parent =
        if executable_parent.file_name().and_then(|name| name.to_str()) == Some("deps") {
            executable_parent
                .parent()
                .context("brainmap test executable directory has no target parent")?
        } else {
            executable_parent
        };
    let brainmapd = executable_parent.join(format!("brainmapd{}", std::env::consts::EXE_SUFFIX));
    #[cfg(test)]
    let brainmapd_sha256 = {
        // Library tests share target/debug/brainmapd with concurrent Cargo
        // invocations. Freeze the first observed fixture hash per test process;
        // production and integration binaries always re-hash the companion.
        static TEST_BRAINMAPD_SHA256: OnceLock<String> = OnceLock::new();
        if let Some(sha256) = TEST_BRAINMAPD_SHA256.get() {
            sha256.clone()
        } else {
            let sha256 = stable_file_sha256(&brainmapd, "companion brainmapd binary")?;
            let _ = TEST_BRAINMAPD_SHA256.set(sha256.clone());
            sha256
        }
    };
    #[cfg(not(test))]
    let brainmapd_sha256 = stable_file_sha256(&brainmapd, "companion brainmapd binary")?;

    Ok(RunningCandidateHashes {
        brainmap_sha256: stable_file_sha256(&brainmap, "running brainmap binary")?,
        brainmapd_sha256,
    })
}

fn stable_file_sha256(path: &Path, label: &str) -> Result<String> {
    let before =
        fs::metadata(path).with_context(|| format!("inspect {label} {}", path.display()))?;
    ensure!(before.is_file(), "{label} is not a regular file");
    let bytes = fs::read(path).with_context(|| format!("read {label} {}", path.display()))?;
    let after =
        fs::metadata(path).with_context(|| format!("reinspect {label} {}", path.display()))?;
    ensure!(
        before.len() == bytes.len() as u64
            && after.len() == before.len()
            && after.modified().ok() == before.modified().ok(),
        "{label} changed while its candidate checksum was calculated"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            after.dev() == before.dev()
                && after.ino() == before.ino()
                && after.ctime() == before.ctime()
                && after.ctime_nsec() == before.ctime_nsec(),
            "{label} changed while its candidate checksum was calculated"
        );
    }
    Ok(util::sha256_hex(&bytes))
}

fn build_inventory(root: &Path) -> Result<Inventory> {
    let metadata = fs::symlink_metadata(root)
        .with_context(|| format!("inspect qualification bundle {}", root.display()))?;
    ensure!(
        metadata.is_dir(),
        "qualification bundle must be a directory"
    );
    ensure!(
        !metadata.file_type().is_symlink(),
        "qualification bundle cannot be a symlink"
    );

    let mut files = BTreeMap::new();
    let mut folded_paths = BTreeMap::<String, String>::new();
    let mut entries = 0usize;
    let mut total_bytes = 0u64;
    visit_directory(
        root,
        root,
        0,
        &mut entries,
        &mut total_bytes,
        &mut folded_paths,
        &mut files,
    )?;
    ensure!(
        files.contains_key("qualification.json"),
        "qualification bundle is missing qualification.json"
    );
    ensure!(
        files.contains_key("SHA256SUMS"),
        "qualification bundle is missing SHA256SUMS"
    );
    Ok(Inventory { files })
}

#[allow(clippy::too_many_arguments)]
fn visit_directory(
    root: &Path,
    directory: &Path,
    depth: usize,
    entries: &mut usize,
    total_bytes: &mut u64,
    folded_paths: &mut BTreeMap<String, String>,
    files: &mut BTreeMap<String, FileInfo>,
) -> Result<()> {
    ensure!(
        depth <= MAX_DEPTH,
        "qualification bundle exceeds directory depth limit"
    );
    let mut children = fs::read_dir(directory)
        .with_context(|| format!("read qualification directory {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        *entries += 1;
        ensure!(
            *entries <= MAX_ENTRIES,
            "qualification bundle exceeds entry count limit"
        );
        let name = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("qualification paths must be UTF-8"))?;
        validate_component(&name)?;
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("inspect qualification artifact {}", path.display()))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "qualification bundle contains a symlink: {}",
            path.display()
        );
        if metadata.is_dir() {
            visit_directory(
                root,
                &path,
                depth + 1,
                entries,
                total_bytes,
                folded_paths,
                files,
            )?;
            continue;
        }
        ensure!(
            metadata.is_file(),
            "qualification bundle contains a non-regular file: {}",
            path.display()
        );
        reject_hard_link(&metadata, &path)?;
        ensure!(
            metadata.len() <= MAX_FILE_BYTES,
            "qualification artifact exceeds per-file size limit: {}",
            path.display()
        );
        *total_bytes = total_bytes
            .checked_add(metadata.len())
            .context("qualification bundle size overflow")?;
        ensure!(
            *total_bytes <= MAX_TOTAL_BYTES,
            "qualification bundle exceeds total size limit"
        );
        ensure!(
            files.len() < MAX_FILES,
            "qualification bundle exceeds file count limit"
        );

        let relative = path
            .strip_prefix(root)
            .context("qualification artifact escaped bundle root")?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        validate_relative_path(&relative)?;
        let folded = relative.to_ascii_lowercase();
        if let Some(existing) = folded_paths.insert(folded, relative.clone()) {
            bail!("qualification bundle contains case-colliding paths: {existing} and {relative}");
        }
        let bytes = fs::read(&path)
            .with_context(|| format!("read qualification artifact {}", path.display()))?;
        ensure!(
            bytes.len() as u64 == metadata.len(),
            "qualification artifact changed while reading: {relative}"
        );
        let text = String::from_utf8(bytes)
            .with_context(|| format!("qualification artifact is not UTF-8: {relative}"))?;
        scan_private_content(&relative, &text)?;
        validate_json_artifact(&relative, &text)?;
        let after = fs::symlink_metadata(&path)?;
        ensure!(
            after.is_file() && after.len() == metadata.len(),
            "qualification artifact changed while reading: {relative}"
        );
        files.insert(
            relative,
            FileInfo {
                sha256: util::sha256_hex(text.as_bytes()),
                text,
            },
        );
    }
    Ok(())
}

#[cfg(unix)]
fn reject_hard_link(metadata: &fs::Metadata, path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    ensure!(
        metadata.nlink() == 1,
        "qualification bundle contains a hard link: {}",
        path.display()
    );
    Ok(())
}

#[cfg(not(unix))]
fn reject_hard_link(_metadata: &fs::Metadata, _path: &Path) -> Result<()> {
    Ok(())
}

fn validate_component(component: &str) -> Result<()> {
    ensure!(
        !component.is_empty() && component != "." && component != "..",
        "invalid qualification path component"
    );
    ensure!(
        component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "qualification paths must use portable ASCII components: {component}"
    );
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= MAX_PATH_BYTES,
        "invalid qualification relative path length"
    );
    ensure!(
        !path.starts_with('/') && !path.ends_with('/') && !path.contains('\\'),
        "qualification artifact path must be canonical and relative: {path}"
    );
    for component in path.split('/') {
        validate_component(component)?;
    }
    Ok(())
}

fn scan_private_content(relative: &str, text: &str) -> Result<()> {
    let lower = text.to_ascii_lowercase();
    const PRIVATE_PATHS: [&str; 7] = [
        "/home/",
        "/users/",
        "/tmp/",
        "/opt/",
        "/root/",
        "/var/folders/",
        "c:\\users\\",
    ];
    ensure!(
        !PRIVATE_PATHS.iter().any(|pattern| lower.contains(pattern)),
        "qualification artifact contains a private absolute path: {relative}"
    );
    ensure!(
        !raw_evidence_field_regex().is_match(text),
        "qualification artifact contains a raw prompt or transcript field: {relative}"
    );
    ensure!(
        !contains_qualification_secret(relative, text),
        "qualification artifact contains secret-like material: {relative}"
    );
    Ok(())
}

fn raw_evidence_field_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)\"(?:prompt|messages|transcript|situation|options|toolarguments)\"\s*:"#)
            .expect("raw evidence field regex")
    })
}

fn contains_qualification_secret(_relative: &str, text: &str) -> bool {
    privacy::contains_secret(text)
}

fn validate_json_artifact(relative: &str, text: &str) -> Result<()> {
    if relative.ends_with(".json") {
        serde_json::from_str::<Value>(text)
            .with_context(|| format!("qualification artifact is invalid JSON: {relative}"))?;
    } else if relative.ends_with(".jsonl") {
        ensure!(
            text.ends_with('\n'),
            "qualification JSONL is not newline-complete: {relative}"
        );
        for (index, line) in text.lines().enumerate() {
            ensure!(
                !line.is_empty(),
                "qualification JSONL contains an empty line: {relative}"
            );
            serde_json::from_str::<Value>(line).with_context(|| {
                format!(
                    "qualification JSONL line {} is invalid: {relative}",
                    index + 1
                )
            })?;
        }
    }
    Ok(())
}

fn verify_checksum_file(
    inventory: &Inventory,
    checksum_path: &str,
    subtree_prefix: &str,
) -> Result<BTreeMap<String, String>> {
    let text = read_text(inventory, checksum_path)?;
    ensure!(
        text.ends_with('\n'),
        "checksum file is not newline-complete: {checksum_path}"
    );
    let expected = inventory
        .files
        .keys()
        .filter(|path| {
            if subtree_prefix.is_empty() {
                path.as_str() != checksum_path
            } else {
                path.starts_with(subtree_prefix) && path.as_str() != checksum_path
            }
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut sums = BTreeMap::new();
    let mut previous = None::<String>;
    for (line_index, line) in text.lines().enumerate() {
        ensure!(!line.is_empty(), "empty line in {checksum_path}");
        let (digest, relative) = line.split_once("  ").with_context(|| {
            format!(
                "invalid checksum line {} in {checksum_path}",
                line_index + 1
            )
        })?;
        ensure!(
            is_lower_hex(digest, 64),
            "invalid SHA-256 in {checksum_path}"
        );
        validate_relative_path(relative)?;
        let full_path = if subtree_prefix.is_empty() {
            relative.to_string()
        } else {
            format!("{subtree_prefix}{relative}")
        };
        if let Some(previous) = &previous {
            ensure!(
                previous < &full_path,
                "checksum entries must be sorted and unique in {checksum_path}"
            );
        }
        previous = Some(full_path.clone());
        ensure!(
            sums.insert(full_path.clone(), digest.to_string()).is_none(),
            "duplicate checksum path in {checksum_path}: {full_path}"
        );
        let actual = file(inventory, &full_path)?;
        ensure!(actual.sha256 == digest, "checksum mismatch for {full_path}");
    }
    ensure!(
        sums.keys().cloned().collect::<BTreeSet<_>>() == expected,
        "{checksum_path} does not exactly cover its artifact set"
    );
    Ok(sums)
}

fn verify_root_ref(
    inventory: &Inventory,
    root_sums: &BTreeMap<String, String>,
    reference: &ArtifactRef,
    expected_path: &str,
) -> Result<()> {
    ensure!(
        reference.path == expected_path,
        "qualification evidence path must be {expected_path}"
    );
    verify_ref(inventory, reference, expected_path)?;
    ensure!(
        root_sums.get(expected_path) == Some(&reference.sha256),
        "root checksum and evidence reference disagree for {expected_path}"
    );
    Ok(())
}

fn verify_ref(inventory: &Inventory, reference: &ArtifactRef, full_path: &str) -> Result<()> {
    validate_relative_path(&reference.path)?;
    ensure!(
        is_lower_hex(&reference.sha256, 64),
        "invalid artifact SHA-256 for {full_path}"
    );
    ensure!(
        file(inventory, full_path)?.sha256 == reference.sha256,
        "artifact reference checksum mismatch for {full_path}"
    );
    Ok(())
}

fn verify_subtree_ref(
    inventory: &Inventory,
    reference: &ArtifactRef,
    subtree: &str,
    expected_relative: &str,
) -> Result<()> {
    ensure!(
        reference.path == expected_relative,
        "{subtree} artifact path must be {expected_relative}"
    );
    verify_ref(
        inventory,
        reference,
        &format!("{subtree}/{expected_relative}"),
    )
}

fn validate_reproducibility(
    manifest: &ReproducibilityManifest,
    candidate: &QualificationCandidate,
) -> Result<()> {
    ensure!(
        manifest.schema_version == "brainmap-release-reproducibility-v2",
        "unsupported reproducibility manifest schema"
    );
    ensure!(
        manifest.candidate_commit == candidate.commit,
        "reproducibility candidate commit mismatch"
    );
    ensure!(
        manifest.brainmap_sha256 == candidate.brainmap_sha256,
        "reproducibility brainmap hash mismatch"
    );
    ensure!(
        manifest.brainmapd_sha256 == candidate.brainmapd_sha256,
        "reproducibility brainmapd hash mismatch"
    );
    ensure!(
        manifest.profile == "release" && manifest.locked,
        "reproducibility build is not locked release"
    );
    ensure!(
        manifest.two_root_byte_identical,
        "reproducibility builds are not byte-identical"
    );
    ensure!(
        manifest.clean_tree,
        "reproducibility source tree was not clean"
    );
    ensure!(
        is_lower_hex(&manifest.build_info_sha256, 64),
        "reproducibility build-info hash is invalid"
    );
    for (label, digest) in [
        (
            "integrated qualification",
            &manifest.producer_digests.integrated_qualification_sha256,
        ),
        ("Codex FIA-5", &manifest.producer_digests.codex_fia5_sha256),
        (
            "release qualification",
            &manifest.producer_digests.release_qualification_sha256,
        ),
        (
            "qualification assembler",
            &manifest.producer_digests.assemble_qualification_sha256,
        ),
    ] {
        ensure!(
            is_lower_hex(digest, 64),
            "reproducibility {label} producer digest is invalid"
        );
    }
    Ok(())
}

fn validate_runner(
    inventory: &Inventory,
    manifest: &RunnerManifest,
    candidate: &QualificationCandidate,
    repro_sha: &str,
) -> Result<()> {
    ensure!(
        manifest.schema_version == "brainmap-m8-runner-v2",
        "unsupported runner manifest schema"
    );
    ensure!(
        manifest.candidate == *candidate,
        "runner candidate or binary hash mismatch"
    );
    ensure!(
        manifest.qualification_eligible && manifest.result == "passed",
        "runner evidence is non-qualifying"
    );
    ensure!(
        manifest.execution_mode == "docker",
        "runner must use qualifying docker mode"
    );
    validate_interval(&manifest.started_at, &manifest.completed_at, "runner")?;
    validate_platform(&manifest.provenance.host, "runner host")?;
    validate_platform(
        &manifest.provenance.qualification_environment,
        "runner environment",
    )?;
    ensure!(
        manifest.provenance.qualification_environment.kernel_name == "Linux",
        "runner qualification environment must be Linux"
    );
    ensure!(
        manifest.provenance.qualification_environment.architecture == "x86_64",
        "runner qualification environment must be x86_64"
    );
    let container = &manifest.provenance.container;
    ensure!(
        container.image == "ubuntu:24.04",
        "runner container image mismatch"
    );
    ensure!(
        container.image_id.starts_with("sha256:") && is_lower_hex(&container.image_id[7..], 64),
        "runner container image ID is not immutable"
    );
    ensure!(
        container.network == "none",
        "runner container network was not disabled"
    );
    ensure!(
        container.root_filesystem == "read-only",
        "runner container root filesystem was not read-only"
    );
    ensure!(
        container.capabilities == "dropped",
        "runner container capabilities were not dropped"
    );
    ensure!(
        container.no_new_privileges,
        "runner container did not set no-new-privileges"
    );
    ensure!(
        manifest.build.profile == "release" && manifest.build.locked,
        "runner did not use a locked release build"
    );
    ensure!(
        manifest.build.two_root_byte_identical,
        "runner build lacks two-root reproducibility"
    );
    ensure!(
        manifest.build.reproducibility_manifest_sha256 == repro_sha,
        "runner reproducibility manifest hash mismatch"
    );
    ensure!(
        file(inventory, "runner/release-reproducibility-manifest.json")?.sha256 == repro_sha,
        "runner retained reproducibility manifest mismatch"
    );
    validate_synthetic_privacy(&manifest.privacy, "runner privacy")?;

    verify_subtree_ref(inventory, &manifest.commands, "runner", "commands.json")?;
    for (reference, expected) in [
        (&manifest.reports.fia1, "reports/fia1.json"),
        (&manifest.reports.fia2, "reports/fia2.json"),
        (&manifest.reports.fia3, "reports/fia3.json"),
        (&manifest.reports.fia4, "reports/fia4.json"),
        (&manifest.reports.fia6, "reports/fia6.json"),
        (&manifest.reports.fia7, "reports/fia7.json"),
    ] {
        verify_subtree_ref(inventory, reference, "runner", expected)?;
    }
    validate_runner_commands(inventory)?;
    validate_fia_reports(inventory)?;
    Ok(())
}

fn validate_runner_commands(inventory: &Inventory) -> Result<()> {
    let commands: Vec<RunnerCommand> = read_json(inventory, "runner/commands.json")?;
    ensure!(!commands.is_empty(), "runner command evidence is empty");
    let required = BTreeSet::from(["FIA-1", "FIA-2", "FIA-3", "FIA-4", "FIA-6", "FIA-7"]);
    let allowed = BTreeSet::from([
        "PRECHECK", "FIA-1", "FIA-2", "FIA-3", "FIA-4", "FIA-6", "FIA-7",
    ]);
    let mut covered = BTreeSet::new();
    let mut command_ids = BTreeSet::new();
    for (index, command) in commands.iter().enumerate() {
        ensure!(
            command.sequence == index + 1,
            "runner command sequence is not contiguous"
        );
        ensure!(
            !command.id.is_empty() && !command.command.is_empty(),
            "runner command identity is incomplete"
        );
        ensure!(
            command_ids.insert(command.id.as_str()),
            "runner command ID is duplicated: {}",
            command.id
        );
        ensure!(
            allowed.contains(command.fia.as_str()),
            "runner command claims an unsupported FIA: {}",
            command.fia
        );
        ensure!(
            command.passed,
            "runner command did not pass: {}",
            command.id
        );
        match &command.expected_exit {
            Value::Number(number) => ensure!(
                number.as_i64() == Some(command.exit_code),
                "runner command exit code mismatch: {}",
                command.id
            ),
            Value::String(value) if value == "nonzero" => ensure!(
                command.exit_code != 0,
                "runner command expected failure but exited zero: {}",
                command.id
            ),
            _ => bail!("runner command has invalid expectedExit: {}", command.id),
        }
        covered.insert(command.fia.as_str());
    }
    for required in required {
        ensure!(
            covered.contains(required),
            "runner commands do not cover {required}"
        );
    }
    Ok(())
}

fn validate_fia_reports(inventory: &Inventory) -> Result<()> {
    let fia1: Fia1Report = read_json(inventory, "runner/reports/fia1.json")?;
    ensure!(
        fia1.answers >= 3
            && fia1.previews == fia1.answers
            && fia1.approved_packets == fia1.answers
            && fia1.automatic_rebuild
            && fia1.behavior_derived,
        "FIA-1 report does not qualify"
    );

    let fia2: Fia2Report = read_json(inventory, "runner/reports/fia2.json")?;
    ensure!(
        fia2.exact == 1
            && fia2.paraphrases >= 5
            && fia2.negatives >= 4
            && fia2.correct_predictions >= 6
            && fia2.non_leaks == fia2.negatives
            && fia2.negatives_retained_compatible_learned_options
            && !fia2.rule_id.is_empty(),
        "FIA-2 report does not qualify"
    );
    ensure!(
        fia2.exact.checked_add(fia2.paraphrases) == Some(fia2.correct_predictions),
        "FIA-2 correct prediction count is inconsistent"
    );

    let fia3: Fia3Report = read_json(inventory, "runner/reports/fia3.json")?;
    ensure!(
        fia3.non_dry_decision
            && fia3.action_recorded
            && fia3.previewed
            && fia3.approved
            && fia3.before_choice == "npm"
            && fia3.after_choice == "pnpm"
            && fia3.scope_isolation
            && fia3.relevance_isolation
            && fia3.more_relevant_competing_choice == "npm"
            && fia3.more_relevant_competing_rule_wins,
        "FIA-3 report does not qualify"
    );
    ensure!(
        !fia3.decision_id.is_empty()
            && !fia3.before_rule.is_empty()
            && !fia3.after_rule.is_empty()
            && !fia3.more_relevant_rule.is_empty(),
        "FIA-3 report identities are incomplete"
    );
    ensure!(
        BTreeSet::from([
            fia3.before_rule.as_str(),
            fia3.after_rule.as_str(),
            fia3.more_relevant_rule.as_str(),
        ])
        .len()
            == 3,
        "FIA-3 rule identities are not distinct"
    );

    let fia4: Fia4Report = read_json(inventory, "runner/reports/fia4.json")?;
    ensure!(
        fia4.added
            && fia4.rebuilt_active
            && fia4.active_prediction == "cargo nextest"
            && fia4.active_decoy_policy
            && fia4.exact_causal_policy_set
            && fia4.causally_named
            && fia4.unrelated_not_named
            && fia4.retired
            && fia4.rebuilt_retired
            && fia4.retired_not_applied,
        "FIA-4 report does not qualify"
    );

    let fia6: Fia6Report = read_json(inventory, "runner/reports/fia6.json")?;
    ensure!(
        fia6.operating_system_processes >= 64
            && fia6.gate_processes >= 16
            && fia6.record_processes >= 16
            && fia6.capture_processes >= 16
            && fia6.feedback_processes >= 16,
        "FIA-6 process counts do not qualify"
    );
    let gate_and_record = fia6
        .gate_processes
        .checked_add(fia6.record_processes)
        .context("FIA-6 gate/record process count overflow")?;
    let capture_and_feedback = fia6
        .capture_processes
        .checked_add(fia6.feedback_processes)
        .context("FIA-6 capture/feedback process count overflow")?;
    let expected_processes = gate_and_record
        .checked_add(capture_and_feedback)
        .context("FIA-6 total process count overflow")?;
    let expected_ledger_events = gate_and_record
        .checked_add(fia6.feedback_processes)
        .context("FIA-6 ledger event count overflow")?;
    ensure!(
        fia6.operating_system_processes == expected_processes
            && fia6.ledger_events == expected_ledger_events
            && fia6.capture_events == fia6.capture_processes
            && fia6.applied_packets == fia6.feedback_processes
            && fia6.canonical_notes == fia6.feedback_processes
            && fia6.simultaneous_gate_record_workers == gate_and_record
            && fia6.simultaneous_capture_feedback_workers == capture_and_feedback,
        "FIA-6 process and event counts are inconsistent"
    );
    ensure!(
        fia6.unique_ledger_ids == fia6.ledger_events
            && fia6.unique_capture_ids == fia6.capture_events,
        "FIA-6 event identity evidence does not qualify"
    );
    ensure!(
        fia6.pending_packets == 0
            && fia6.gate_record_overlap_barrier
            && fia6.capture_feedback_overlap_barrier
            && fia6.jsonl_complete
            && fia6.notes_complete,
        "FIA-6 durability evidence does not qualify"
    );
    ensure!(
        is_lower_hex(&fia6.ledger_sha256, 64) && is_lower_hex(&fia6.capture_sha256, 64),
        "FIA-6 evidence hashes are invalid"
    );

    let fia7: Fia7Report = read_json(inventory, "runner/reports/fia7.json")?;
    ensure!(
        fia7.export_verified
            && fia7.behavior_pairs == 3
            && fia7.fault_phases == 8
            && fia7.canonical_fault_states == 8
            && fia7.behavior_equivalent
            && fia7.learned_equivalent
            && fia7.corrected_equivalent
            && fia7.policy_equivalent,
        "FIA-7 report does not qualify"
    );
    ensure!(
        is_lower_hex(&fia7.archive_sha256, 64)
            && is_lower_hex(&fia7.old_tree_hash, 64)
            && is_lower_hex(&fia7.new_tree_hash, 64)
            && fia7.old_tree_hash != fia7.new_tree_hash,
        "FIA-7 evidence hashes are invalid"
    );
    Ok(())
}

fn validate_host(
    inventory: &Inventory,
    manifest: &HostManifest,
    candidate: &QualificationCandidate,
) -> Result<()> {
    ensure!(
        manifest.schema_version == "brainmap-m8-host-v2",
        "unsupported host manifest schema"
    );
    ensure!(
        manifest.candidate == *candidate,
        "host candidate or binary hash mismatch"
    );
    ensure!(
        manifest.qualification_eligible && manifest.mode == "qualification",
        "host evidence is non-qualifying"
    );
    validate_interval(&manifest.started_at, &manifest.completed_at, "host")?;
    let adapter = &manifest.adapter;
    ensure!(
        adapter.target == "codex" && adapter.launch_mode == "normal",
        "FIA-5 did not use a normal Codex host launch"
    );
    ensure!(
        is_codex_cli_version(&adapter.host_version),
        "FIA-5 Codex host version is invalid"
    );
    ensure!(
        adapter.host_version == QUALIFYING_CODEX_VERSION,
        "FIA-5 Codex host version is not the pinned qualifying release"
    );
    ensure!(
        !adapter.trust_bypass_used && adapter.persisted_hook_accepted && adapter.project_trusted,
        "FIA-5 host trust was bypassed or not accepted"
    );
    ensure!(
        !manifest.provenance.kernel_name.is_empty()
            && !manifest.provenance.kernel_release.is_empty()
            && !manifest.provenance.architecture.is_empty(),
        "FIA-5 host provenance is incomplete"
    );
    ensure!(
        manifest.provenance.configured_brainmap_sha256 == candidate.brainmap_sha256,
        "FIA-5 configured brainmap hash mismatch"
    );
    ensure!(
        manifest.provenance.configured_brainmapd_sha256 == candidate.brainmapd_sha256,
        "FIA-5 configured brainmapd hash mismatch"
    );
    ensure!(
        manifest.provenance.codex_target == QUALIFYING_CODEX_TARGET
            && manifest.provenance.official_codex_archive_sha256 == QUALIFYING_CODEX_ARCHIVE_SHA256
            && manifest.provenance.official_codex_binary_sha256 == QUALIFYING_CODEX_BINARY_SHA256
            && manifest.provenance.observed_codex_binary_sha256 == QUALIFYING_CODEX_BINARY_SHA256
            && manifest.provenance.official_codex_verified,
        "FIA-5 official Codex provenance does not qualify"
    );
    validate_synthetic_privacy(&manifest.privacy, "host privacy")?;

    verify_subtree_ref(
        inventory,
        &manifest.artifacts.events,
        "host",
        "events.jsonl",
    )?;
    verify_subtree_ref(
        inventory,
        &manifest.artifacts.install_dry_run,
        "host",
        "install-dry-run.json",
    )?;
    verify_subtree_ref(inventory, &manifest.artifacts.doctor, "host", "doctor.json")?;
    verify_subtree_ref(
        inventory,
        &manifest.artifacts.host_observation,
        "host",
        "host-observation.json",
    )?;

    let install: HostInstallDryRun = read_json(inventory, "host/install-dry-run.json")?;
    ensure!(
        install.schema_version == "brainmap-m8-host-install-dry-run-v1"
            && install.target == "codex"
            && install.dry_run,
        "FIA-5 installer dry-run evidence is invalid"
    );
    ensure!(
        install.candidate == *candidate,
        "FIA-5 installer candidate mismatch"
    );
    let doctor: HostDoctor = read_json(inventory, "host/doctor.json")?;
    ensure!(
        doctor.schema_version == "brainmap-m8-host-doctor-v1"
            && doctor.target == "codex"
            && doctor.healthy,
        "FIA-5 doctor evidence is unhealthy"
    );
    ensure!(
        doctor.health_scope == "local-adapter-files-and-contract"
            && !doctor.host_hook_trust_verified
            && doctor.host_probe_required,
        "FIA-5 doctor evidence overclaims host trust"
    );
    ensure!(
        doctor.candidate == *candidate,
        "FIA-5 doctor candidate mismatch"
    );
    let observation: HostObservation = read_json(inventory, "host/host-observation.json")?;
    validate_host_observation(&observation, manifest, candidate)?;
    validate_host_events(inventory, &observation)
}

fn validate_host_observation(
    observation: &HostObservation,
    manifest: &HostManifest,
    candidate: &QualificationCandidate,
) -> Result<()> {
    ensure!(
        observation.schema_version == "brainmap-m8-host-observation-v2",
        "unsupported FIA-5 host observation schema"
    );
    ensure!(
        observation.qualification_eligible
            && observation.mode == "qualification"
            && observation.mode == manifest.mode,
        "FIA-5 host observation is non-qualifying"
    );
    ensure!(
        observation.candidate == *candidate,
        "FIA-5 host observation candidate mismatch"
    );

    let codex = &observation.official_codex;
    ensure!(
        codex.version == QUALIFYING_CODEX_VERSION
            && codex.target == QUALIFYING_CODEX_TARGET
            && codex.archive_sha256 == QUALIFYING_CODEX_ARCHIVE_SHA256
            && codex.binary_sha256 == QUALIFYING_CODEX_BINARY_SHA256
            && codex.observed_binary_sha256 == QUALIFYING_CODEX_BINARY_SHA256
            && codex.archive_verified
            && codex.binary_verified
            && codex.version == manifest.adapter.host_version
            && codex.target == manifest.provenance.codex_target
            && codex.archive_sha256 == manifest.provenance.official_codex_archive_sha256
            && codex.binary_sha256 == manifest.provenance.official_codex_binary_sha256
            && codex.observed_binary_sha256 == manifest.provenance.observed_codex_binary_sha256,
        "FIA-5 official Codex provenance does not qualify"
    );

    let config = &observation.config;
    ensure!(
        config.approval_policy == "on-request"
            && config.approvals_reviewer == "user"
            && config.sandbox_mode == "workspace-write"
            && !config.workspace_write_network_access
            && !config.bypass_hook_trust
            && !config.bypass_approvals_and_sandbox
            && config.feedback_approval_mode == "prompt"
            && config.apply_approval_mode == "prompt"
            && config.gate_mode == "active"
            && config.autopilot_mode == "conservative"
            && is_lower_hex(&config.codex_home_sha256, 64),
        "FIA-5 host observation safe config does not qualify"
    );

    const NORMAL_ARGV: [&str; 13] = [
        "kind:codex-executable",
        "--ask-for-approval",
        "on-request",
        "--sandbox",
        "workspace-write",
        "-c",
        "approvals_reviewer=\"user\"",
        "-c",
        "sandbox_workspace_write.network_access=false",
        "--cd",
        "kind:synthetic-project",
        "--no-alt-screen",
        "kind:fixed-workflow-directive",
    ];
    const APP_SERVER_ARGV: [&str; 11] = [
        "kind:codex-executable",
        "-c",
        "approval_policy=\"on-request\"",
        "-c",
        "approvals_reviewer=\"user\"",
        "-c",
        "sandbox_mode=\"workspace-write\"",
        "-c",
        "sandbox_workspace_write.network_access=false",
        "app-server",
        "--stdio",
    ];
    let launch = &observation.launch;
    ensure!(
        is_lower_hex(&launch.launcher_sha256, 64)
            && is_lower_hex(&launch.argv_sha256, 64)
            && is_lower_hex(&launch.app_server_argv_sha256, 64)
            && launch.codex_home_bound
            && launch.project_inventory_bound,
        "FIA-5 host launch binding does not qualify"
    );
    validate_host_argv(&launch.argv, &NORMAL_ARGV, "normal launch")?;
    validate_host_argv(
        &launch.app_server_argv,
        &APP_SERVER_ARGV,
        "app-server launch",
    )?;
    ensure!(
        launch.argv[0].sha256.as_deref() == Some(QUALIFYING_CODEX_BINARY_SHA256)
            && launch.app_server_argv[0].sha256.as_deref() == Some(QUALIFYING_CODEX_BINARY_SHA256)
            && launch.session.source == "cli"
            && is_lower_hex(&launch.session.id_sha256, 64)
            && launch.session.created_at > 0,
        "FIA-5 host launch identity does not qualify"
    );

    let hooks = &observation.hooks;
    let hook_names = hooks
        .entries
        .iter()
        .map(|entry| entry.event_name.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        hooks.trusted_hook_count == 2
            && hooks.entries.len() == 2
            && hooks.executed_hook_gate_count >= 1
            && hook_names == BTreeSet::from(["preToolUse", "userPromptSubmit"])
            && hooks.entries.iter().all(|entry| {
                entry.trust_status == "trusted"
                    && entry
                        .current_hash
                        .strip_prefix("sha256:")
                        .is_some_and(|hash| is_lower_hex(hash, 64))
            }),
        "FIA-5 host hook evidence does not qualify"
    );

    const CALL_ORDER: [&str; 7] = [
        "brainmap_decision_gate",
        "brainmap_record_decision",
        "brainmap_learn_feedback",
        "brainmap_preview_update",
        "brainmap_apply_update",
        "brainmap_decision_gate",
        "brainmap_record_decision",
    ];
    let calls = &observation.calls;
    ensure!(
        calls.count == 7
            && calls.order.iter().map(String::as_str).eq(CALL_ORDER)
            && is_runtime_id(&calls.first.decision_id, "dec")
            && calls.first.outcome == "ask_user"
            && calls.first.selected_option.is_none()
            && calls.first.action.chosen == "biome"
            && calls.first.action.was_asked
            && is_runtime_id(&calls.feedback.packet_id, "upd")
            && calls.feedback.previewed
            && calls.feedback.approved
            && is_runtime_id(&calls.second.decision_id, "dec")
            && calls.second.decision_id != calls.first.decision_id
            && calls.second.outcome == "proceed"
            && calls.second.selected_option == "prettier"
            && calls.second.changed
            && calls.second.action.chosen == "prettier"
            && !calls.second.action.was_asked,
        "FIA-5 host observation call lifecycle does not qualify"
    );
    ensure!(
        observation.ledger.correlation == "complete"
            && observation.ledger.correlated_event_count == 5
            && observation.ledger.post_boundary_event_count >= 6,
        "FIA-5 host ledger correlation does not qualify"
    );
    ensure!(
        observation.project.unchanged
            && is_lower_hex(&observation.project.inventory_sha256, 64)
            && is_lower_hex(&observation.project.workflow_sha256, 64),
        "FIA-5 host project binding does not qualify"
    );
    Ok(())
}

fn validate_host_argv(
    arguments: &[HostArgDescriptor],
    expected: &[&str],
    label: &str,
) -> Result<()> {
    ensure!(
        arguments.len() == expected.len(),
        "FIA-5 {label} argv length is invalid"
    );
    for (index, (argument, expected_value)) in arguments.iter().zip(expected).enumerate() {
        ensure!(
            argument.position == index,
            "FIA-5 {label} argv position is invalid"
        );
        if let Some(expected_kind) = expected_value.strip_prefix("kind:") {
            ensure!(
                argument.kind.as_deref() == Some(expected_kind)
                    && argument.literal.is_none()
                    && argument
                        .sha256
                        .as_deref()
                        .is_some_and(|hash| is_lower_hex(hash, 64)),
                "FIA-5 {label} argv descriptor is invalid"
            );
        } else {
            ensure!(
                argument.literal.as_deref() == Some(*expected_value)
                    && argument.kind.is_none()
                    && argument.sha256.is_none(),
                "FIA-5 {label} argv descriptor is invalid"
            );
        }
    }
    Ok(())
}

fn validate_host_events(inventory: &Inventory, observation: &HostObservation) -> Result<()> {
    let text = read_text(inventory, "host/events.jsonl")?;
    let events = text
        .lines()
        .map(|line| {
            serde_json::from_str::<HostEvent>(line).context("parse strict FIA-5 host event")
        })
        .collect::<Result<Vec<_>>>()?;
    const KINDS: [&str; 12] = [
        "installer-dry-run",
        "installed",
        "doctor-healthy",
        "host-launched",
        "initial-gate",
        "initial-outcome-followed",
        "initial-action-recorded",
        "feedback-created",
        "preview-observed",
        "update-approved",
        "changed-outcome-followed",
        "changed-action-recorded",
    ];
    ensure!(
        events.len() == KINDS.len(),
        "FIA-5 host event sequence is incomplete"
    );
    for (index, (event, expected_kind)) in events.iter().zip(KINDS).enumerate() {
        ensure!(
            event.sequence == index + 1 && event.kind == expected_kind,
            "FIA-5 host event order is invalid"
        );
        ensure!(event.success, "FIA-5 host event failed: {}", event.kind);
    }
    for event in &events[..4] {
        ensure!(
            event.decision_id.is_none()
                && event.packet_id.is_none()
                && event.changed.is_none()
                && event.outcome.is_none()
                && event.selected_option.is_none(),
            "FIA-5 setup event contains unexpected correlation data"
        );
    }
    let first_decision_id = events[4]
        .decision_id
        .as_deref()
        .context("FIA-5 gate event lacks decision ID")?;
    ensure!(
        is_runtime_id(first_decision_id, "dec"),
        "FIA-5 decision ID is not a Brainmap runtime ID"
    );
    for event in &events[4..10] {
        ensure!(
            event.decision_id.as_deref() == Some(first_decision_id),
            "FIA-5 decision correlation mismatch"
        );
    }
    for event in &events[4..7] {
        ensure!(
            event.packet_id.is_none() && event.changed.is_none(),
            "FIA-5 pre-feedback event contains unexpected packet data"
        );
    }
    let packet_id = events[7]
        .packet_id
        .as_deref()
        .context("FIA-5 feedback event lacks packet ID")?;
    ensure!(!packet_id.is_empty(), "FIA-5 packet ID is empty");
    ensure!(
        is_runtime_id(packet_id, "upd"),
        "FIA-5 packet ID is not a Brainmap runtime ID"
    );
    for event in &events[7..] {
        ensure!(
            event.packet_id.as_deref() == Some(packet_id),
            "FIA-5 packet correlation mismatch"
        );
    }
    for event in &events[7..10] {
        ensure!(
            event.changed.is_none(),
            "FIA-5 event claims a prediction change before approval"
        );
    }
    ensure!(
        events[10].changed == Some(true),
        "FIA-5 next host prediction did not change"
    );
    let second_decision_id = events[10]
        .decision_id
        .as_deref()
        .context("FIA-5 changed outcome event lacks decision ID")?;
    ensure!(
        is_runtime_id(second_decision_id, "dec"),
        "FIA-5 decision ID is not a Brainmap runtime ID"
    );
    ensure!(
        second_decision_id != first_decision_id
            && events[11].decision_id.as_deref() == Some(second_decision_id),
        "FIA-5 second decision correlation mismatch"
    );
    ensure!(
        events[10].outcome.as_deref() == Some("proceed")
            && events[10].selected_option.as_deref() == Some("prettier")
            && events[11].changed.is_none()
            && events[11].outcome.is_none()
            && events[11].selected_option.is_none(),
        "FIA-5 changed outcome evidence is invalid"
    );
    ensure!(
        first_decision_id == observation.calls.first.decision_id
            && second_decision_id == observation.calls.second.decision_id
            && packet_id == observation.calls.feedback.packet_id,
        "FIA-5 events do not match the host observation"
    );
    Ok(())
}

fn validate_release(
    inventory: &Inventory,
    manifest: &ReleaseManifest,
    candidate: &QualificationCandidate,
    repro_sha: &str,
) -> Result<()> {
    ensure!(
        manifest.schema_version == "brainmap-m8-release-v1",
        "unsupported release manifest schema"
    );
    ensure!(
        manifest.candidate == *candidate,
        "release candidate or binary hash mismatch"
    );
    ensure!(
        manifest.qualification_eligible,
        "release evidence is non-qualifying"
    );
    ensure!(
        !manifest.source_tree_dirty_before && !manifest.source_tree_dirty_after,
        "release qualification worktree was dirty"
    );
    ensure!(
        manifest.reproducibility_manifest_sha256 == repro_sha,
        "release reproducibility manifest hash mismatch"
    );
    ensure!(
        file(inventory, "release/reproducibility-manifest.json")?.sha256 == repro_sha,
        "release retained reproducibility manifest mismatch"
    );
    validate_interval(&manifest.started_at, &manifest.completed_at, "release")?;
    validate_platform(&manifest.host, "release host")?;
    ensure!(
        !manifest.toolchain.rustc.is_empty() && !manifest.toolchain.cargo.is_empty(),
        "release toolchain provenance is incomplete"
    );
    validate_synthetic_privacy(&manifest.privacy, "release privacy")?;
    let sbom: Value = read_json(inventory, "release/sbom/brainmap.cdx.json")?;
    ensure!(
        sbom.get("bomFormat").and_then(Value::as_str) == Some("CycloneDX")
            && (sbom.get("authors").is_some() || sbom.get("metadata").is_some()),
        "release SBOM evidence is not a CycloneDX document"
    );

    for (key, file_stem, reference) in [
        ("format", "format", &manifest.gates.format),
        ("clippy", "clippy", &manifest.gates.clippy),
        (
            "workspace-tests",
            "workspace-tests",
            &manifest.gates.workspace_tests,
        ),
        ("audit", "audit", &manifest.gates.audit),
        ("deny", "deny", &manifest.gates.deny),
        ("sbom", "sbom", &manifest.gates.sbom),
        (
            "locked-release-build",
            "locked-release-build",
            &manifest.gates.locked_release_build,
        ),
        (
            "package-smoke",
            "package-smoke",
            &manifest.gates.package_smoke,
        ),
        ("scale-1000", "scale-1000", &manifest.gates.scale1000),
        ("scale-5000", "scale-5000", &manifest.gates.scale5000),
        ("performance", "performance", &manifest.gates.performance),
        (
            "clean-worktree",
            "clean-worktree",
            &manifest.gates.clean_worktree,
        ),
    ] {
        let result_path = format!("gates/{file_stem}.json");
        verify_subtree_ref(inventory, reference, "release", &result_path)?;
        let result: ReleaseGateResult = read_json(inventory, &format!("release/{result_path}"))?;
        ensure!(
            result.schema_version == "brainmap-m8-release-gate-result-v1",
            "unsupported release gate result schema: {key}"
        );
        ensure!(
            result.gate == key && result.command_id == key,
            "release gate identity mismatch: {key}"
        );
        ensure!(
            result.passed && result.exit_code == 0,
            "release gate failed: {key}"
        );
        ensure!(
            is_lower_hex(&result.log_sha256, 64),
            "release gate log hash is invalid: {key}"
        );
        let log_path = format!("release/gates/{file_stem}.log");
        ensure!(
            file(inventory, &log_path)?.sha256 == result.log_sha256,
            "release gate log hash mismatch: {key}"
        );
    }
    validate_release_observations(inventory, candidate)?;
    Ok(())
}

fn validate_release_observations(
    inventory: &Inventory,
    candidate: &QualificationCandidate,
) -> Result<()> {
    let qualification: ReleaseQualificationManifest = read_json(
        inventory,
        "release/qualification/qualification-manifest.json",
    )?;
    ensure!(
        qualification.schema_version == "brainmap-release-qualification-v1"
            && qualification.source_commit == candidate.commit
            && !qualification.source_tree_dirty
            && qualification.binaries.brainmap_sha256 == candidate.brainmap_sha256
            && qualification.binaries.brainmapd_sha256 == candidate.brainmapd_sha256
            && is_lower_hex(&qualification.portable_archive_sha256, 64)
            && !qualification.host.is_empty()
            && !qualification.toolchain.rustc.is_empty()
            && !qualification.toolchain.cargo.is_empty(),
        "release qualification observation does not match the candidate"
    );
    validate_interval(
        &qualification.started_at,
        &qualification.completed_at,
        "release qualification observation",
    )?;

    let eval: Value = read_json(inventory, "release/qualification/eval.json")?;
    let recall = eval.get("learnedRuleRecall").and_then(Value::as_object);
    let eval_qualifies = eval
        .get("cases")
        .and_then(Value::as_u64)
        .is_some_and(|v| v >= 100)
        && [
            "falseProceed",
            "falseAsk",
            "falseBlock",
            "wrongChoice",
            "wrongRule",
            "wrongMetadata",
        ]
        .into_iter()
        .all(|field| eval.get(field).and_then(Value::as_u64) == Some(0))
        && recall
            .and_then(|value| value.get("exact"))
            .and_then(Value::as_f64)
            .is_some_and(|value| value >= 1.0)
        && recall
            .and_then(|value| value.get("supportedParaphrase"))
            .and_then(Value::as_f64)
            .is_some_and(|value| value >= 0.95)
        && recall
            .and_then(|value| value.get("negativeExpected"))
            .and_then(Value::as_u64)
            .is_some_and(|value| value >= 100)
        && recall
            .and_then(|value| value.get("negativeSpecificity"))
            .and_then(Value::as_f64)
            .is_some_and(|value| value >= 1.0);
    ensure!(
        eval_qualifies,
        "release evaluation evidence does not satisfy correctness thresholds"
    );

    validate_release_benchmark(
        inventory,
        "release/qualification/bench-1000.json",
        1_000,
        10.0,
        None,
    )?;
    validate_release_benchmark(
        inventory,
        "release/qualification/bench-5000.json",
        5_000,
        25.0,
        Some(1_000.0),
    )?;

    for phase in [
        "verified",
        "staging-created",
        "files-written",
        "index-rebuilt",
        "links-checked",
        "gate-checked",
        "existing-backed-up",
        "staging-activated",
    ] {
        let path = format!("release/qualification/restore-fault-{phase}-state.json");
        let observation: RestoreFaultObservation = read_json(inventory, &path)?;
        let expected_tree = match observation.complete_state.as_str() {
            "old" => &observation.old_tree_hash,
            "new" => &observation.new_tree_hash,
            _ => bail!("release restore fault observation is noncanonical: {phase}"),
        };
        ensure!(
            observation.phase == phase
                && is_lower_hex(&observation.tree_hash, 64)
                && is_lower_hex(&observation.old_tree_hash, 64)
                && is_lower_hex(&observation.new_tree_hash, 64)
                && observation.old_tree_hash != observation.new_tree_hash
                && observation.tree_hash == *expected_tree,
            "release restore fault observation is noncanonical: {phase}"
        );
    }
    Ok(())
}

fn validate_release_benchmark(
    inventory: &Inventory,
    path: &str,
    expected_scale: u64,
    gate_p95_limit_ms: f64,
    rebuild_limit_ms: Option<f64>,
) -> Result<()> {
    let bench: Value = read_json(inventory, path)?;
    let qualifies = bench.get("scaleRequested").and_then(Value::as_u64) == Some(expected_scale)
        && bench
            .get("executableRules")
            .and_then(Value::as_u64)
            .is_some_and(|value| value >= expected_scale)
        && bench
            .get("gateP95Ms")
            .and_then(Value::as_f64)
            .is_some_and(|value| value < gate_p95_limit_ms)
        && bench
            .get("candidateBounds")
            .and_then(Value::as_object)
            .is_some_and(|value| !value.is_empty())
        && rebuild_limit_ms.is_none_or(|limit| {
            bench
                .get("indexRebuildMs")
                .and_then(Value::as_f64)
                .is_some_and(|value| value < limit)
        });
    ensure!(
        qualifies,
        "release {} benchmark exceeds its qualification envelope",
        if expected_scale == 1_000 { "1k" } else { "5k" }
    );
    Ok(())
}

fn validate_candidate(candidate: &QualificationCandidate, label: &str) -> Result<()> {
    ensure!(
        is_lower_hex(&candidate.commit, 40),
        "{label} commit must be 40 lowercase hexadecimal characters"
    );
    ensure!(
        is_lower_hex(&candidate.brainmap_sha256, 64),
        "{label} brainmap SHA-256 is invalid"
    );
    ensure!(
        is_lower_hex(&candidate.brainmapd_sha256, 64),
        "{label} brainmapd SHA-256 is invalid"
    );
    Ok(())
}

fn validate_privacy(privacy: &PrivacyClaims, label: &str) -> Result<()> {
    ensure!(
        !privacy.raw_prompts_retained
            && !privacy.secrets_retained
            && !privacy.private_paths_retained,
        "{label} claims retained private material"
    );
    Ok(())
}

fn validate_synthetic_privacy(privacy: &SyntheticPrivacyClaims, label: &str) -> Result<()> {
    ensure!(
        !privacy.raw_prompts_retained
            && !privacy.secrets_retained
            && !privacy.private_paths_retained
            && privacy.synthetic_inputs_only,
        "{label} does not satisfy the synthetic-only privacy contract"
    );
    Ok(())
}

fn validate_platform(platform: &HostPlatform, label: &str) -> Result<()> {
    ensure!(
        !platform.kernel_name.is_empty()
            && !platform.kernel_release.is_empty()
            && !platform.architecture.is_empty(),
        "{label} provenance is incomplete"
    );
    Ok(())
}

fn validate_interval(started_at: &str, completed_at: &str, label: &str) -> Result<()> {
    ensure!(
        started_at.ends_with('Z') && completed_at.ends_with('Z'),
        "{label} timestamps must use UTC Z notation"
    );
    let started = DateTime::parse_from_rfc3339(started_at)
        .with_context(|| format!("parse {label} startedAt"))?
        .with_timezone(&Utc);
    let completed = DateTime::parse_from_rfc3339(completed_at)
        .with_context(|| format!("parse {label} completedAt"))?
        .with_timezone(&Utc);
    ensure!(
        started <= completed,
        "{label} completedAt precedes startedAt"
    );
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(inventory: &Inventory, relative: &str) -> Result<T> {
    serde_json::from_str(&file(inventory, relative)?.text)
        .with_context(|| format!("parse strict qualification JSON {relative}"))
}

fn read_text<'a>(inventory: &'a Inventory, relative: &str) -> Result<&'a str> {
    Ok(&file(inventory, relative)?.text)
}

fn file<'a>(inventory: &'a Inventory, relative: &str) -> Result<&'a FileInfo> {
    inventory
        .files
        .get(relative)
        .with_context(|| format!("qualification artifact is missing: {relative}"))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_codex_cli_version(value: &str) -> bool {
    static VERSION: OnceLock<Regex> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            Regex::new(r"^codex-cli [0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$")
                .expect("valid Codex CLI version regex")
        })
        .is_match(value)
}

fn is_runtime_id(value: &str, prefix: &str) -> bool {
    let Some(rest) = value
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('_'))
    else {
        return false;
    };
    let Some((millis, digest)) = rest.split_once('_') else {
        return false;
    };
    (10..=20).contains(&millis.len())
        && millis.bytes().all(|byte| byte.is_ascii_digit())
        && is_lower_hex(digest, 12)
}
