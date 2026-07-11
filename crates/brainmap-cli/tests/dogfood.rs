use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

mod support;
use support::qualification::ValidBundle;

const CANDIDATE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn debug_binary_rejects_dogfood_start_before_mutation() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let qualification = valid_bundle();
    let now = Utc::now();
    let started_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    fails(
        &[
            "dogfood",
            "start",
            "--candidate-commit",
            CANDIDATE_COMMIT,
            "--adapter",
            "codex",
            "--started-at",
            &started_at,
            "--qualification-bundle",
            path(&qualification.bundle),
            "--vault",
            path(&root),
        ],
        "candidate binary was not built by the clean locked two-root qualification workflow",
    );
    assert!(!root.join(".brainmap/dogfood.json").exists());
    assert_eq!(
        fs::read_dir(root.join("99-meta/backups"))
            .expect("read backups")
            .count(),
        0
    );
}

#[test]
#[ignore = "requires the eligible installed release candidate exercised by M8"]
fn status_fails_closed_when_a_nested_copied_qualification_artifact_is_tampered() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let qualification = valid_bundle();
    let now = Utc::now();
    let started_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&[
        "dogfood",
        "start",
        "--candidate-commit",
        CANDIDATE_COMMIT,
        "--adapter",
        "codex",
        "--started-at",
        &started_at,
        "--qualification-bundle",
        path(&qualification.bundle),
        "--vault",
        path(&root),
    ]);
    let status: Value = serde_json::from_str(&ok(&["dogfood", "status", "--vault", path(&root)]))
        .expect("status JSON");
    let copied = root.join(
        status["qualificationBundleRelativePath"]
            .as_str()
            .expect("qualification relative path"),
    );
    let state_path = root.join(".brainmap/dogfood.json");
    let original_state = fs::read(&state_path).expect("read dogfood state");
    let mut redirected_state: Value =
        serde_json::from_slice(&original_state).expect("parse dogfood state");
    redirected_state["runs"][0]["qualificationBundleRelativePath"] =
        json!(qualification.bundle.to_string_lossy());
    fs::write(
        &state_path,
        serde_json::to_vec_pretty(&redirected_state).unwrap(),
    )
    .expect("redirect persisted qualification path");
    fails(
        &["dogfood", "status", "--vault", path(&root)],
        "qualification bundle path is not scoped to its run ID",
    );
    assert!(qualification.bundle.join("qualification.json").is_file());
    fs::write(&state_path, original_state).expect("restore dogfood state");

    fs::write(
        copied.join("runner/reports/fia1.json"),
        b"{\"tampered\":true}\n",
    )
    .expect("tamper copied qualification artifact");

    let unhealthy = run(&["dogfood", "status", "--vault", path(&root)]);
    assert!(!unhealthy.status.success());
    let unhealthy_status: Value =
        serde_json::from_slice(&unhealthy.stdout).expect("unhealthy status JSON");
    assert_eq!(
        unhealthy_status["health"]["qualificationBundleMatches"],
        false
    );
    assert_eq!(unhealthy_status["health"]["healthy"], false);
}

#[test]
#[ignore = "requires the eligible installed release candidate exercised by M8"]
fn review_persists_a_prompt_free_self_bound_receipt_and_updates_status() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap-private-path");
    let qualification = valid_bundle();
    let now = Utc::now();
    let started_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&[
        "dogfood",
        "start",
        "--candidate-commit",
        CANDIDATE_COMMIT,
        "--adapter",
        "codex",
        "--started-at",
        &started_at,
        "--qualification-bundle",
        path(&qualification.bundle),
        "--vault",
        path(&root),
    ]);

    let receipt_text = ok(&[
        "dogfood",
        "review",
        "--incident-status",
        "clear",
        "--vault",
        path(&root),
    ]);
    let receipt: Value = serde_json::from_str(&receipt_text).expect("review receipt JSON");
    assert_eq!(receipt["schemaVersion"], "brainmap-dogfood-review-v1");
    assert_eq!(receipt["kind"], "dogfood-review");
    assert_eq!(receipt["incidentStatus"], "clear");
    assert_eq!(receipt["ledgerPrefixBytes"], 0);
    assert_eq!(receipt["ledgerPrefixLines"], 0);
    assert_eq!(
        receipt["ledgerPrefixSha256"],
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        receipt["shadowMetricsSha256"]
            .as_str()
            .expect("metrics checksum")
            .len(),
        64
    );
    let created_at = receipt["createdAt"].as_str().expect("createdAt");
    let milliseconds = created_at
        .split_once('.')
        .and_then(|(_, value)| value.strip_suffix('Z'))
        .expect("canonical UTC milliseconds");
    assert_eq!(milliseconds.len(), 3);
    assert!(!receipt_text.contains(root.to_string_lossy().as_ref()));
    let receipt_keys = receipt
        .as_object()
        .expect("receipt object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert!(!receipt_keys.iter().any(|key| {
        let key = key.to_ascii_lowercase();
        key.contains("prompt") || key.contains("path") || key.contains("note")
    }));

    let status: Value = serde_json::from_str(&ok(&["dogfood", "status", "--vault", path(&root)]))
        .expect("status JSON");
    assert_eq!(status["reviewSummary"]["reviewCount"], 1);
    assert_eq!(status["reviewSummary"]["currentIncidentStatus"], "clear");
    assert_eq!(status["reviewSummary"]["finalReviewCoversLedger"], true);
    assert_eq!(status["health"]["reviewIntegrityValid"], true);
    assert_eq!(status["health"]["reviewReady"], true);
    assert_eq!(status["health"]["healthy"], true);

    fails(
        &[
            "dogfood",
            "review",
            "--incident-status",
            "clear",
            "--created-at",
            "2026-01-01T00:00:00.000Z",
            "--vault",
            path(&root),
        ],
        "unexpected argument '--created-at'",
    );
}

#[test]
#[ignore = "requires the eligible installed release candidate exercised by M8"]
fn abort_records_a_sanitized_reason_and_closes_the_active_run() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let qualification = valid_bundle();
    let now = Utc::now();
    let started_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&[
        "dogfood",
        "start",
        "--candidate-commit",
        CANDIDATE_COMMIT,
        "--adapter",
        "codex",
        "--started-at",
        &started_at,
        "--qualification-bundle",
        path(&qualification.bundle),
        "--vault",
        path(&root),
    ]);

    let output = ok(&[
        "dogfood",
        "abort",
        "--reason",
        "binary drift; api_key=abcdef1234567890",
        "--vault",
        path(&root),
    ]);
    let aborted: Value = serde_json::from_str(&output).expect("abort JSON");
    assert_eq!(aborted["status"], "aborted");
    assert_eq!(aborted["abortReason"], "binary drift; [REDACTED]");
    assert!(aborted["abortedAt"].as_str().is_some());
    assert!(!output.contains("abcdef1234567890"));

    let status: Value = serde_json::from_str(&ok(&["dogfood", "status", "--vault", path(&root)]))
        .expect("status JSON");
    assert_eq!(status["status"], "aborted");
    fails(
        &[
            "dogfood",
            "abort",
            "--reason",
            "already closed",
            "--vault",
            path(&root),
        ],
        "no active dogfood run",
    );
}

#[test]
#[ignore = "requires the eligible installed release candidate exercised by M8"]
fn finalize_refuses_incomplete_intensive_coverage_without_changing_the_run() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let qualification = valid_bundle();
    let report_dir = tmp.path().join("report");
    let now = Utc::now();
    let started_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&[
        "dogfood",
        "start",
        "--candidate-commit",
        CANDIDATE_COMMIT,
        "--adapter",
        "codex",
        "--started-at",
        &started_at,
        "--qualification-bundle",
        path(&qualification.bundle),
        "--vault",
        path(&root),
    ]);

    fails(
        &[
            "dogfood",
            "finalize",
            "--out",
            path(&report_dir),
            "--signer",
            "Local developer",
            "--incident-disposition",
            "No incidents observed",
            "--vault",
            path(&root),
        ],
        "action coverage",
    );
    assert!(!report_dir.exists());
    let status: Value = serde_json::from_str(&ok(&["dogfood", "status", "--vault", path(&root)]))
        .expect("status JSON");
    assert_eq!(status["status"], "active");
    assert!(status["finalizedAt"].is_null());
}

#[test]
fn start_rejects_legacy_tampered_or_candidate_mismatched_qualification() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let now = Utc::now();
    let started_at = now.to_rfc3339_opts(SecondsFormat::Secs, true);
    ok(&["init-vault", "--vault", path(&root), "--yes"]);

    let legacy = tmp.path().join("legacy-qualification");
    fs::create_dir(&legacy).expect("create legacy qualification");
    fs::write(
        legacy.join("qualification.json"),
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": "brainmap-m8-fia-v1",
            "candidateCommit": CANDIDATE_COMMIT,
            "brainmapSha256": binary_sha256(),
            "fia1": true, "fia2": true, "fia3": true, "fia4": true,
            "fia5": true, "fia6": true, "fia7": true, "fia8": true
        }))
        .unwrap(),
    )
    .unwrap();
    write_test_checksums(&legacy);
    fails_start(
        &root,
        &legacy,
        &started_at,
        "codex",
        "legacy flat FIA self-attestation is not accepted",
    );

    let tampered = valid_bundle();
    fs::write(
        tampered.bundle.join("runner/reports/fia1.json"),
        b"{\"tampered\":true}\n",
    )
    .unwrap();
    fails_start(
        &root,
        &tampered.bundle,
        &started_at,
        "codex",
        "checksum mismatch",
    );

    let wrong_commit = ValidBundle::new_for(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        binary_sha256(),
        brainmapd_sha256(),
    );
    fails_start(
        &root,
        &wrong_commit.bundle,
        &started_at,
        "codex",
        "candidate commit does not match",
    );

    let wrong_brainmap = ValidBundle::new_for(CANDIDATE_COMMIT, "0".repeat(64), brainmapd_sha256());
    fails_start(
        &root,
        &wrong_brainmap.bundle,
        &started_at,
        "codex",
        "brainmap hash does not match the running brainmap binary",
    );

    let wrong_brainmapd = ValidBundle::new_for(CANDIDATE_COMMIT, binary_sha256(), "0".repeat(64));
    fails_start(
        &root,
        &wrong_brainmapd.bundle,
        &started_at,
        "codex",
        "brainmapd hash does not match the companion brainmapd binary",
    );

    let valid = valid_bundle();
    fails_start(
        &root,
        &valid.bundle,
        &started_at,
        "claude",
        "requires --adapter codex",
    );

    assert!(!root.join(".brainmap/dogfood.json").exists());
    assert_eq!(
        fs::read_dir(root.join("99-meta/backups"))
            .expect("read backups")
            .count(),
        0
    );
}

#[test]
fn removed_planned_end_argument_is_rejected() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let qualification = valid_bundle();
    let started_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    ok(&["init-vault", "--vault", path(&root), "--yes"]);

    fails(
        &[
            "dogfood",
            "start",
            "--candidate-commit",
            CANDIDATE_COMMIT,
            "--adapter",
            "codex",
            "--started-at",
            &started_at,
            "--planned-end",
            "2026-07-17T00:00:00Z",
            "--qualification-bundle",
            path(&qualification.bundle),
            "--vault",
            path(&root),
        ],
        "unexpected argument '--planned-end'",
    );

    assert!(!root.join(".brainmap/dogfood.json").exists());
    let help = ok(&["dogfood", "start", "--help"]);
    assert!(!help.contains("planned-end"));
}

fn valid_bundle() -> ValidBundle {
    ValidBundle::new_for(CANDIDATE_COMMIT, binary_sha256(), brainmapd_sha256())
}

fn fails_start(
    root: &Path,
    qualification: &Path,
    started_at: &str,
    adapter: &str,
    expected_stderr: &str,
) {
    fails(
        &[
            "dogfood",
            "start",
            "--candidate-commit",
            CANDIDATE_COMMIT,
            "--adapter",
            adapter,
            "--started-at",
            started_at,
            "--qualification-bundle",
            path(qualification),
            "--vault",
            path(root),
        ],
        expected_stderr,
    );
}

fn write_test_checksums(root: &Path) {
    let name = "qualification.json";
    fs::write(
        root.join("SHA256SUMS"),
        format!("{}  {name}\n", file_sha256(&root.join(name))),
    )
    .expect("write test checksums");
}

fn binary_sha256() -> String {
    file_sha256(Path::new(env!("CARGO_BIN_EXE_brainmap")))
}

fn brainmapd_sha256() -> String {
    file_sha256(Path::new(env!("CARGO_BIN_EXE_brainmapd")))
}

fn file_sha256(path: &Path) -> String {
    let bytes = fs::read(path).expect("read file for checksum");
    hex::encode(Sha256::digest(bytes))
}

fn ok(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "brainmap {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .args(args)
        .output()
        .expect("run brainmap")
}

fn fails(args: &[&str], expected_stderr: &str) {
    let output = run(args);
    assert!(!output.status.success(), "brainmap {args:?} should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected_stderr),
        "stderr did not contain {expected_stderr:?}:\n{stderr}"
    );
}

fn path(path: &Path) -> &str {
    path.to_str().expect("test path is utf-8")
}
