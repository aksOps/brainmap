#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

pub(crate) const COMMIT: &str = env!("BRAINMAP_BUILD_CANDIDATE_COMMIT");
pub(crate) const BRAINMAPD_SHA: &str =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const STARTED_AT: &str = "2026-07-10T00:00:00Z";
const COMPLETED_AT: &str = "2026-07-10T01:00:00Z";
const CODEX_VERSION: &str = "codex-cli 0.144.0";
const CODEX_TARGET: &str = "x86_64-unknown-linux-musl";
const CODEX_ARCHIVE_SHA: &str = "6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd";
const CODEX_BINARY_SHA: &str = "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429";

pub(crate) struct ValidBundle {
    _tmp: tempfile::TempDir,
    pub(crate) bundle: std::path::PathBuf,
    pub(crate) brainmap_sha: String,
    pub(crate) brainmapd_sha: String,
}

impl ValidBundle {
    pub(crate) fn new() -> Self {
        Self::new_for(COMMIT, binary_sha256(), brainmapd_binary_sha256())
    }

    pub(crate) fn new_for(
        commit: impl Into<String>,
        brainmap_sha: impl Into<String>,
        brainmapd_sha: impl Into<String>,
    ) -> Self {
        let commit = commit.into();
        let brainmap_sha = brainmap_sha.into();
        let brainmapd_sha = brainmapd_sha.into();
        let tmp = tempfile::tempdir().expect("temp dir");
        let bundle = tmp.path().join("qualification");
        for directory in [
            "reproducibility",
            "runner/reports",
            "host",
            "release/gates",
            "release/qualification",
            "release/sbom",
        ] {
            fs::create_dir_all(bundle.join(directory)).expect("create bundle directory");
        }
        let candidate = candidate(&commit, &brainmap_sha, &brainmapd_sha);
        let build_info = serde_json::json!({
            "schemaVersion": "brainmap-build-info-v1",
            "candidateCommit": env!("BRAINMAP_BUILD_CANDIDATE_COMMIT"),
            "cargoProfile": env!("BRAINMAP_BUILD_CARGO_PROFILE"),
            "qualification": {
                "eligible": env!("BRAINMAP_BUILD_QUALIFICATION_ELIGIBLE") == "true",
                "marker": env!("BRAINMAP_BUILD_QUALIFICATION_MARKER"),
                "release": env!("BRAINMAP_BUILD_QUALIFICATION_RELEASE") == "true",
                "locked": env!("BRAINMAP_BUILD_QUALIFICATION_LOCKED") == "true",
                "twoRootCandidate": env!("BRAINMAP_BUILD_TWO_ROOT_CANDIDATE") == "true"
            },
            "producerDigests": {
                "integratedQualificationSha256": env!("BRAINMAP_M8_INTEGRATED_QUALIFICATION_SHA256"),
                "codexFia5Sha256": env!("BRAINMAP_M8_CODEX_FIA5_SHA256"),
                "releaseQualificationSha256": env!("BRAINMAP_M8_RELEASE_QUALIFICATION_SHA256"),
                "assembleQualificationSha256": env!("BRAINMAP_M8_ASSEMBLE_QUALIFICATION_SHA256")
            }
        });
        let build_info_json =
            serde_json::to_string(&build_info).expect("serialize embedded test build provenance");
        let build_info_sha = format!("{:x}", Sha256::digest(build_info_json.as_bytes()));

        write_json(
            &bundle.join("reproducibility/manifest.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-release-reproducibility-v2",
                "candidateCommit": commit,
                "profile": "release",
                "locked": true,
                "twoRootByteIdentical": true,
                "cleanTree": true,
                "brainmapSha256": brainmap_sha,
                "brainmapdSha256": brainmapd_sha,
                "buildInfoSha256": build_info_sha,
                "producerDigests": build_info["producerDigests"]
            }),
        );
        let repro_sha = sha256_file(&bundle.join("reproducibility/manifest.json"));
        fs::copy(
            bundle.join("reproducibility/manifest.json"),
            bundle.join("release/reproducibility-manifest.json"),
        )
        .expect("retain release reproducibility manifest");
        fs::copy(
            bundle.join("reproducibility/manifest.json"),
            bundle.join("runner/release-reproducibility-manifest.json"),
        )
        .expect("retain runner reproducibility manifest");
        write_json(
            &bundle.join("release/sbom/brainmap.cdx.json"),
            &serde_json::json!({
                "bomFormat": "CycloneDX",
                "metadata": {},
                "components": [{"author": "Dependency Author"}]
            }),
        );

        write_json(
            &bundle.join("release/qualification/qualification-manifest.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-release-qualification-v1",
                "sourceCommit": commit,
                "sourceTreeDirty": false,
                "startedAt": STARTED_AT,
                "completedAt": COMPLETED_AT,
                "host": "Linux fixture x86_64",
                "toolchain": {"rustc": "rustc fixture", "cargo": "cargo fixture"},
                "binaries": {
                    "brainmapSha256": brainmap_sha,
                    "brainmapdSha256": brainmapd_sha
                },
                "portableArchiveSha256": "c".repeat(64)
            }),
        );
        write_json(
            &bundle.join("release/qualification/eval.json"),
            &serde_json::json!({
                "cases": 238,
                "falseProceed": 0,
                "falseAsk": 0,
                "falseBlock": 0,
                "wrongChoice": 0,
                "wrongRule": 0,
                "wrongMetadata": 0,
                "learnedRuleRecall": {
                    "exact": 1.0,
                    "supportedParaphrase": 1.0,
                    "negativeExpected": 207,
                    "negativeSpecificity": 1.0
                }
            }),
        );
        for (scale, gate_p95, index_ms) in [(1000, 2.5, 150), (5000, 11.0, 500)] {
            write_json(
                &bundle.join(format!("release/qualification/bench-{scale}.json")),
                &serde_json::json!({
                    "scaleRequested": scale,
                    "executableRules": scale,
                    "gateP95Ms": gate_p95,
                    "indexRebuildMs": index_ms,
                    "candidateBounds": {"retrieval": "actual-rule-term-postings"}
                }),
            );
        }
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
            write_json(
                &bundle.join(format!(
                    "release/qualification/restore-fault-{phase}-state.json"
                )),
                &serde_json::json!({
                    "phase": phase,
                    "completeState": "old",
                    "treeHash": "d".repeat(64),
                    "oldTreeHash": "d".repeat(64),
                    "newTreeHash": "e".repeat(64)
                }),
            );
        }

        write_json(
            &bundle.join("runner/commands.json"),
            &serde_json::Value::Array(
                ["FIA-1", "FIA-2", "FIA-3", "FIA-4", "FIA-6", "FIA-7"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, fia)| {
                        serde_json::json!({
                            "sequence": index + 1,
                            "fia": fia,
                            "id": format!("{}-fixture", fia.to_ascii_lowercase()),
                            "command": "<brainmap> qualification fixture",
                            "expectedExit": 0,
                            "exitCode": 0,
                            "passed": true
                        })
                    })
                    .collect(),
            ),
        );
        let reports = [
            (
                "fia1",
                serde_json::json!({
                    "answers": 3, "previews": 3, "approvedPackets": 3,
                    "automaticRebuild": true, "behaviorDerived": true
                }),
            ),
            (
                "fia2",
                serde_json::json!({
                    "exact": 1, "paraphrases": 5, "negatives": 4,
                    "correctPredictions": 6, "nonLeaks": 4,
                    "negativesRetainedCompatibleLearnedOptions": true,
                    "ruleId": "rule-fia2"
                }),
            ),
            (
                "fia3",
                serde_json::json!({
                    "nonDryDecision": true, "actionRecorded": true,
                    "previewed": true, "approved": true,
                    "beforeChoice": "npm", "afterChoice": "pnpm",
                    "scopeIsolation": true, "relevanceIsolation": true,
                    "moreRelevantCompetingChoice": "npm",
                    "moreRelevantCompetingRuleWins": true,
                    "decisionId": "decision-fia3", "beforeRule": "rule-before",
                    "afterRule": "rule-after", "moreRelevantRule": "rule-more-relevant"
                }),
            ),
            (
                "fia4",
                serde_json::json!({
                    "added": true, "rebuiltActive": true,
                    "activePrediction": "cargo nextest", "activeDecoyPolicy": true,
                    "exactCausalPolicySet": true, "causallyNamed": true,
                    "unrelatedNotNamed": true, "retired": true,
                    "rebuiltRetired": true, "retiredNotApplied": true
                }),
            ),
            (
                "fia6",
                serde_json::json!({
                    "operatingSystemProcesses": 64, "gateProcesses": 16,
                    "recordProcesses": 16, "captureProcesses": 16,
                    "feedbackProcesses": 16, "ledgerEvents": 48,
                    "uniqueLedgerIds": 48, "captureEvents": 16,
                    "uniqueCaptureIds": 16, "appliedPackets": 16,
                    "canonicalNotes": 16, "pendingPackets": 0,
                    "gateRecordOverlapBarrier": true,
                    "captureFeedbackOverlapBarrier": true,
                    "simultaneousGateRecordWorkers": 32,
                    "simultaneousCaptureFeedbackWorkers": 32,
                    "jsonlComplete": true, "notesComplete": true,
                    "ledgerSha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "captureSha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                }),
            ),
            (
                "fia7",
                serde_json::json!({
                    "exportVerified": true,
                    "archiveSha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "oldTreeHash": "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    "newTreeHash": "1111111111111111111111111111111111111111111111111111111111111111",
                    "behaviorPairs": 3, "faultPhases": 8,
                    "canonicalFaultStates": 8, "behaviorEquivalent": true,
                    "learnedEquivalent": true, "correctedEquivalent": true,
                    "policyEquivalent": true
                }),
            ),
        ];
        for (fia, report) in reports {
            write_json(&bundle.join(format!("runner/reports/{fia}.json")), &report);
        }
        let report_refs = serde_json::Map::from_iter(
            ["fia1", "fia2", "fia3", "fia4", "fia6", "fia7"].map(|fia| {
                (
                    fia.to_string(),
                    file_ref(&bundle, &format!("runner/reports/{fia}.json"), "reports/"),
                )
            }),
        );
        write_json(
            &bundle.join("runner/manifest.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-runner-v2",
                "qualificationEligible": true,
                "result": "passed",
                "candidate": candidate,
                "startedAt": STARTED_AT,
                "completedAt": COMPLETED_AT,
                "executionMode": "docker",
                "provenance": {
                    "host": {"kernelName": "Linux", "kernelRelease": "fixture", "architecture": "x86_64"},
                    "qualificationEnvironment": {"kernelName": "Linux", "kernelRelease": "fixture", "architecture": "x86_64"},
                    "container": {
                        "image": "ubuntu:24.04", "imageId": format!("sha256:{}", "2".repeat(64)),
                        "network": "none", "rootFilesystem": "read-only",
                        "capabilities": "dropped", "noNewPrivileges": true
                    }
                },
                "build": {
                    "profile": "release", "locked": true, "twoRootByteIdentical": true,
                    "reproducibilityManifestSha256": repro_sha
                },
                "commands": file_ref(&bundle, "runner/commands.json", ""),
                "reports": serde_json::Value::Object(report_refs),
                "privacy": {
                    "rawPromptsRetained": false, "secretsRetained": false,
                    "privatePathsRetained": false, "syntheticInputsOnly": true
                }
            }),
        );
        write_checksums(&bundle.join("runner"));

        write_json(
            &bundle.join("host/install-dry-run.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-host-install-dry-run-v1",
                "target": "codex", "dryRun": true, "candidate": candidate
            }),
        );
        write_json(
            &bundle.join("host/doctor.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-host-doctor-v1",
                "target": "codex", "healthy": true,
                "healthScope": "local-adapter-files-and-contract",
                "hostHookTrustVerified": false, "hostProbeRequired": true,
                "candidate": candidate
            }),
        );
        let first_decision_id = "dec_1720000000000_aaaaaaaaaaaa";
        let packet_id = "upd_1720000000001_bbbbbbbbbbbb";
        let second_decision_id = "dec_1720000000002_cccccccccccc";
        let host_events = [
            ("installer-dry-run", None, None, None, None, None),
            ("installed", None, None, None, None, None),
            ("doctor-healthy", None, None, None, None, None),
            ("host-launched", None, None, None, None, None),
            (
                "initial-gate",
                Some(first_decision_id),
                None,
                None,
                None,
                None,
            ),
            (
                "initial-outcome-followed",
                Some(first_decision_id),
                None,
                None,
                None,
                None,
            ),
            (
                "initial-action-recorded",
                Some(first_decision_id),
                None,
                None,
                None,
                None,
            ),
            (
                "feedback-created",
                Some(first_decision_id),
                Some(packet_id),
                None,
                None,
                None,
            ),
            (
                "preview-observed",
                Some(first_decision_id),
                Some(packet_id),
                None,
                None,
                None,
            ),
            (
                "update-approved",
                Some(first_decision_id),
                Some(packet_id),
                None,
                None,
                None,
            ),
            (
                "changed-outcome-followed",
                Some(second_decision_id),
                Some(packet_id),
                Some(true),
                Some("proceed"),
                Some("prettier"),
            ),
            (
                "changed-action-recorded",
                Some(second_decision_id),
                Some(packet_id),
                None,
                None,
                None,
            ),
        ];
        let mut events = String::new();
        for (index, (kind, decision_id, packet_id, changed, outcome, selected_option)) in
            host_events.into_iter().enumerate()
        {
            let mut event =
                serde_json::json!({"sequence": index + 1, "kind": kind, "success": true});
            if let Some(value) = decision_id {
                event["decisionId"] = serde_json::json!(value);
            }
            if let Some(value) = packet_id {
                event["packetId"] = serde_json::json!(value);
            }
            if let Some(value) = changed {
                event["changed"] = serde_json::json!(value);
            }
            if let Some(value) = outcome {
                event["outcome"] = serde_json::json!(value);
            }
            if let Some(value) = selected_option {
                event["selectedOption"] = serde_json::json!(value);
            }
            events.push_str(&serde_json::to_string(&event).expect("serialize host event"));
            events.push('\n');
        }
        fs::write(bundle.join("host/events.jsonl"), events).expect("write host events");
        write_json(
            &bundle.join("host/host-observation.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-host-observation-v2",
                "qualificationEligible": true,
                "mode": "qualification",
                "candidate": candidate,
                "officialCodex": {
                    "version": CODEX_VERSION,
                    "target": CODEX_TARGET,
                    "archiveSha256": CODEX_ARCHIVE_SHA,
                    "binarySha256": CODEX_BINARY_SHA,
                    "observedBinarySha256": CODEX_BINARY_SHA,
                    "archiveVerified": true,
                    "binaryVerified": true
                },
                "config": {
                    "approvalPolicy": "on-request",
                    "approvalsReviewer": "user",
                    "sandboxMode": "workspace-write",
                    "workspaceWriteNetworkAccess": false,
                    "bypassHookTrust": false,
                    "bypassApprovalsAndSandbox": false,
                    "feedbackApprovalMode": "prompt",
                    "applyApprovalMode": "prompt",
                    "codexHomeSha256": "8".repeat(64),
                    "gateMode": "active",
                    "autopilotMode": "conservative"
                },
                "launch": {
                    "launcherSha256": "3".repeat(64),
                    "argvSha256": "4".repeat(64),
                    "argv": [
                        {"position":0,"kind":"codex-executable","sha256":CODEX_BINARY_SHA},
                        {"position":1,"literal":"--ask-for-approval"},
                        {"position":2,"literal":"on-request"},
                        {"position":3,"literal":"--sandbox"},
                        {"position":4,"literal":"workspace-write"},
                        {"position":5,"literal":"-c"},
                        {"position":6,"literal":"approvals_reviewer=\"user\""},
                        {"position":7,"literal":"-c"},
                        {"position":8,"literal":"sandbox_workspace_write.network_access=false"},
                        {"position":9,"literal":"--cd"},
                        {"position":10,"kind":"synthetic-project","sha256":"a".repeat(64)},
                        {"position":11,"literal":"--no-alt-screen"},
                        {"position":12,"kind":"fixed-workflow-directive","sha256":"b".repeat(64)}
                    ],
                    "appServerArgvSha256": "5".repeat(64),
                    "appServerArgv": [
                        {"position":0,"kind":"codex-executable","sha256":CODEX_BINARY_SHA},
                        {"position":1,"literal":"-c"},
                        {"position":2,"literal":"approval_policy=\"on-request\""},
                        {"position":3,"literal":"-c"},
                        {"position":4,"literal":"approvals_reviewer=\"user\""},
                        {"position":5,"literal":"-c"},
                        {"position":6,"literal":"sandbox_mode=\"workspace-write\""},
                        {"position":7,"literal":"-c"},
                        {"position":8,"literal":"sandbox_workspace_write.network_access=false"},
                        {"position":9,"literal":"app-server"},
                        {"position":10,"literal":"--stdio"}
                    ],
                    "codexHomeBound": true,
                    "projectInventoryBound": true,
                    "session": {
                        "source": "cli", "idSha256": "9".repeat(64),
                        "createdAt": 1720000000
                    }
                },
                "hooks": {
                    "trustedHookCount": 2,
                    "entries": [
                        {"eventName":"preToolUse","currentHash":format!("sha256:{}", "6".repeat(64)),"trustStatus":"trusted"},
                        {"eventName":"userPromptSubmit","currentHash":format!("sha256:{}", "7".repeat(64)),"trustStatus":"trusted"}
                    ],
                    "executedHookGateCount": 1
                },
                "calls": {
                    "count": 7,
                    "order": [
                        "brainmap_decision_gate", "brainmap_record_decision",
                        "brainmap_learn_feedback", "brainmap_preview_update",
                        "brainmap_apply_update", "brainmap_decision_gate",
                        "brainmap_record_decision"
                    ],
                    "first": {
                        "decisionId": first_decision_id, "outcome": "ask_user",
                        "selectedOption": null,
                        "action": {"chosen":"biome","wasAsked":true}
                    },
                    "feedback": {"packetId":packet_id,"previewed":true,"approved":true},
                    "second": {
                        "decisionId": second_decision_id, "outcome":"proceed",
                        "selectedOption":"prettier", "changed":true,
                        "action":{"chosen":"prettier","wasAsked":false}
                    }
                },
                "ledger": {
                    "correlation":"complete", "correlatedEventCount":5,
                    "postBoundaryEventCount":6
                },
                "project": {
                    "inventorySha256":"a".repeat(64),
                    "workflowSha256":"b".repeat(64), "unchanged":true
                }
            }),
        );
        write_json(
            &bundle.join("host/manifest.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-host-v2",
                "qualificationEligible": true, "mode": "qualification",
                "candidate": candidate,
                "startedAt": STARTED_AT, "completedAt": COMPLETED_AT,
                "adapter": {
                    "target": "codex", "hostVersion": CODEX_VERSION,
                    "launchMode": "normal", "trustBypassUsed": false,
                    "persistedHookAccepted": true, "projectTrusted": true
                },
                "provenance": {
                    "kernelName": "Linux", "kernelRelease": "fixture", "architecture": "x86_64",
                    "configuredBrainmapSha256": brainmap_sha,
                    "configuredBrainmapdSha256": brainmapd_sha,
                    "codexTarget": CODEX_TARGET,
                    "officialCodexArchiveSha256": CODEX_ARCHIVE_SHA,
                    "officialCodexBinarySha256": CODEX_BINARY_SHA,
                    "observedCodexBinarySha256": CODEX_BINARY_SHA,
                    "officialCodexVerified": true
                },
                "artifacts": {
                    "events": file_ref(&bundle, "host/events.jsonl", ""),
                    "installDryRun": file_ref(&bundle, "host/install-dry-run.json", ""),
                    "doctor": file_ref(&bundle, "host/doctor.json", ""),
                    "hostObservation": file_ref(&bundle, "host/host-observation.json", "")
                },
                "privacy": {
                    "rawPromptsRetained": false, "secretsRetained": false,
                    "privatePathsRetained": false, "syntheticInputsOnly": true
                }
            }),
        );
        write_checksums(&bundle.join("host"));

        let gate_names = [
            "format",
            "clippy",
            "workspace-tests",
            "audit",
            "deny",
            "sbom",
            "locked-release-build",
            "package-smoke",
            "scale-1000",
            "scale-5000",
            "performance",
            "clean-worktree",
        ];
        for gate in gate_names {
            let log_path = bundle.join(format!("release/gates/{gate}.log"));
            fs::write(&log_path, format!("{gate} passed\n")).expect("write release gate log");
            write_json(
                &bundle.join(format!("release/gates/{gate}.json")),
                &serde_json::json!({
                    "schemaVersion": "brainmap-m8-release-gate-result-v1",
                    "gate": gate, "commandId": gate,
                    "passed": true, "exitCode": 0,
                    "logSha256": sha256_file(&log_path)
                }),
            );
        }
        let release_gate_refs = serde_json::Map::from_iter(
            [
                ("format", "format"),
                ("clippy", "clippy"),
                ("workspaceTests", "workspace-tests"),
                ("audit", "audit"),
                ("deny", "deny"),
                ("sbom", "sbom"),
                ("lockedReleaseBuild", "locked-release-build"),
                ("packageSmoke", "package-smoke"),
                ("scale1000", "scale-1000"),
                ("scale5000", "scale-5000"),
                ("performance", "performance"),
                ("cleanWorktree", "clean-worktree"),
            ]
            .map(|(key, file)| {
                (
                    key.to_string(),
                    file_ref(&bundle, &format!("release/gates/{file}.json"), "gates/"),
                )
            }),
        );
        write_json(
            &bundle.join("release/manifest.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-release-v1",
                "qualificationEligible": true, "candidate": candidate,
                "sourceTreeDirtyBefore": false, "sourceTreeDirtyAfter": false,
                "startedAt": STARTED_AT, "completedAt": COMPLETED_AT,
                "host": {"kernelName": "Linux", "kernelRelease": "fixture", "architecture": "x86_64"},
                "toolchain": {"rustc": "rustc fixture", "cargo": "cargo fixture"},
                "reproducibilityManifestSha256": repro_sha,
                "gates": serde_json::Value::Object(release_gate_refs),
                "privacy": {
                    "rawPromptsRetained": false, "secretsRetained": false,
                    "privatePathsRetained": false, "syntheticInputsOnly": true
                }
            }),
        );
        write_checksums(&bundle.join("release"));

        write_json(
            &bundle.join("qualification.json"),
            &serde_json::json!({
                "schemaVersion": "brainmap-m8-qualification-bundle-v1",
                "candidate": candidate,
                "evidence": {
                    "reproducibilityManifest": root_ref(&bundle, "reproducibility/manifest.json"),
                    "runnerManifest": root_ref(&bundle, "runner/manifest.json"),
                    "runnerChecksums": root_ref(&bundle, "runner/SHA256SUMS"),
                    "hostManifest": root_ref(&bundle, "host/manifest.json"),
                    "hostChecksums": root_ref(&bundle, "host/SHA256SUMS"),
                    "releaseManifest": root_ref(&bundle, "release/manifest.json"),
                    "releaseChecksums": root_ref(&bundle, "release/SHA256SUMS")
                },
                "privacy": {
                    "rawPromptsRetained": false,
                    "secretsRetained": false,
                    "privatePathsRetained": false
                }
            }),
        );
        write_checksums(&bundle);

        Self {
            _tmp: tmp,
            bundle,
            brainmap_sha,
            brainmapd_sha,
        }
    }

    pub(crate) fn mutate_json(&self, relative: &str, mutate: impl FnOnce(&mut serde_json::Value)) {
        let path = self.bundle.join(relative);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read JSON to mutate"))
                .expect("parse JSON to mutate");
        mutate(&mut value);
        write_json(&path, &value);
    }

    pub(crate) fn mutate_host_events(&self, mutate: impl FnOnce(&mut [serde_json::Value])) {
        let path = self.bundle.join("host/events.jsonl");
        let text = fs::read_to_string(&path).expect("read host events to mutate");
        let mut events = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("parse host event to mutate"))
            .collect::<Vec<_>>();
        mutate(&mut events);
        let mut output = String::new();
        for event in events {
            output.push_str(&serde_json::to_string(&event).expect("serialize mutated host event"));
            output.push('\n');
        }
        fs::write(path, output).expect("write mutated host events");
    }

    pub(crate) fn refresh_checksums(&self) {
        self.mutate_json("runner/manifest.json", |manifest| {
            manifest["commands"] = file_ref(&self.bundle, "runner/commands.json", "");
            for fia in ["fia1", "fia2", "fia3", "fia4", "fia6", "fia7"] {
                manifest["reports"][fia] = file_ref(
                    &self.bundle,
                    &format!("runner/reports/{fia}.json"),
                    "reports/",
                );
            }
        });
        self.mutate_json("host/manifest.json", |manifest| {
            for (key, relative) in [
                ("events", "host/events.jsonl"),
                ("installDryRun", "host/install-dry-run.json"),
                ("doctor", "host/doctor.json"),
                ("hostObservation", "host/host-observation.json"),
            ] {
                manifest["artifacts"][key] = file_ref(&self.bundle, relative, "");
            }
        });
        self.mutate_json("release/manifest.json", |manifest| {
            for (key, file) in [
                ("format", "format"),
                ("clippy", "clippy"),
                ("workspaceTests", "workspace-tests"),
                ("audit", "audit"),
                ("deny", "deny"),
                ("sbom", "sbom"),
                ("lockedReleaseBuild", "locked-release-build"),
                ("packageSmoke", "package-smoke"),
                ("scale1000", "scale-1000"),
                ("scale5000", "scale-5000"),
                ("performance", "performance"),
                ("cleanWorktree", "clean-worktree"),
            ] {
                let relative = format!("release/gates/{file}.json");
                if self.bundle.join(&relative).is_file() {
                    manifest["gates"][key] = file_ref(&self.bundle, &relative, "gates/");
                }
            }
        });
        for subtree in ["runner", "host", "release"] {
            write_checksums(&self.bundle.join(subtree));
        }
        self.mutate_json("qualification.json", |manifest| {
            for (key, relative) in [
                ("reproducibilityManifest", "reproducibility/manifest.json"),
                ("runnerManifest", "runner/manifest.json"),
                ("runnerChecksums", "runner/SHA256SUMS"),
                ("hostManifest", "host/manifest.json"),
                ("hostChecksums", "host/SHA256SUMS"),
                ("releaseManifest", "release/manifest.json"),
                ("releaseChecksums", "release/SHA256SUMS"),
            ] {
                manifest["evidence"][key] = root_ref(&self.bundle, relative);
            }
        });
        write_checksums(&self.bundle);
    }

    pub(crate) fn retarget_brainmap(&self, brainmap_sha: String) {
        self.mutate_json("reproducibility/manifest.json", |manifest| {
            manifest["brainmapSha256"] = serde_json::json!(brainmap_sha);
        });
        fs::copy(
            self.bundle.join("reproducibility/manifest.json"),
            self.bundle.join("release/reproducibility-manifest.json"),
        )
        .expect("refresh retained reproducibility manifest");
        fs::copy(
            self.bundle.join("reproducibility/manifest.json"),
            self.bundle
                .join("runner/release-reproducibility-manifest.json"),
        )
        .expect("refresh runner reproducibility manifest");
        let repro_sha = sha256_file(&self.bundle.join("reproducibility/manifest.json"));

        self.mutate_json("runner/manifest.json", |manifest| {
            manifest["candidate"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
            manifest["build"]["reproducibilityManifestSha256"] = serde_json::json!(repro_sha);
        });
        self.mutate_json("host/manifest.json", |manifest| {
            manifest["candidate"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
            manifest["provenance"]["configuredBrainmapSha256"] = serde_json::json!(brainmap_sha);
        });
        self.mutate_json("host/host-observation.json", |observation| {
            observation["candidate"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
        });
        for relative in ["host/install-dry-run.json", "host/doctor.json"] {
            self.mutate_json(relative, |manifest| {
                manifest["candidate"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
            });
        }
        self.mutate_json("release/manifest.json", |manifest| {
            manifest["candidate"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
            manifest["reproducibilityManifestSha256"] = serde_json::json!(repro_sha);
        });
        self.mutate_json(
            "release/qualification/qualification-manifest.json",
            |manifest| {
                manifest["binaries"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
            },
        );
        self.mutate_json("qualification.json", |manifest| {
            manifest["candidate"]["brainmapSha256"] = serde_json::json!(brainmap_sha);
        });
        self.refresh_checksums();
    }
}

fn candidate(commit: &str, brainmap_sha: &str, brainmapd_sha: &str) -> serde_json::Value {
    serde_json::json!({
        "commit": commit,
        "brainmapSha256": brainmap_sha,
        "brainmapdSha256": brainmapd_sha
    })
}

pub(crate) fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize JSON"),
    )
    .expect("write JSON");
}

fn file_ref(bundle: &Path, root_relative: &str, strip_prefix: &str) -> serde_json::Value {
    let path = root_relative
        .split_once('/')
        .map(|(_, relative)| relative)
        .unwrap_or(root_relative);
    let path = path.strip_prefix(strip_prefix).unwrap_or(path);
    serde_json::json!({
        "path": if strip_prefix.is_empty() { path.to_string() } else { format!("{strip_prefix}{path}") },
        "sha256": sha256_file(&bundle.join(root_relative))
    })
}

fn root_ref(bundle: &Path, relative: &str) -> serde_json::Value {
    serde_json::json!({"path": relative, "sha256": sha256_file(&bundle.join(relative))})
}

pub(crate) fn sha256_file(path: &Path) -> String {
    format!(
        "{:x}",
        Sha256::digest(fs::read(path).expect("read file to hash"))
    )
}

fn binary_sha256() -> &'static str {
    static SHA256: OnceLock<String> = OnceLock::new();
    SHA256
        .get_or_init(|| {
            let Some(binary) = option_env!("CARGO_BIN_EXE_brainmap") else {
                panic!("CARGO_BIN_EXE_brainmap is required by ValidBundle::new");
            };
            sha256_file(Path::new(binary))
        })
        .as_str()
}

fn brainmapd_binary_sha256() -> &'static str {
    static SHA256: OnceLock<String> = OnceLock::new();
    SHA256
        .get_or_init(|| {
            let Some(binary) = option_env!("CARGO_BIN_EXE_brainmapd") else {
                panic!("CARGO_BIN_EXE_brainmapd is required by ValidBundle::new");
            };
            sha256_file(Path::new(binary))
        })
        .as_str()
}

pub(crate) fn write_checksums(root: &Path) {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut checksums = String::new();
    for relative in files {
        if relative == "SHA256SUMS" {
            continue;
        }
        let bytes = fs::read(root.join(&relative)).expect("read checksummed file");
        checksums.push_str(&format!("{:x}  {relative}\n", Sha256::digest(bytes)));
    }
    fs::write(root.join("SHA256SUMS"), checksums).expect("write checksums");
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(directory).expect("read directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(
                path.strip_prefix(root)
                    .expect("path under root")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
