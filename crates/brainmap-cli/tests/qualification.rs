mod support;

use std::fs;
use std::path::Path;
use std::process::Command;
use support::qualification::{COMMIT, ValidBundle, sha256_file, write_checksums, write_json};

#[test]
fn qualification_verify_rejects_a_semantic_bundle_for_a_nonqualifying_debug_binary() {
    let fixture = ValidBundle::new();
    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "candidate binary was not built by the clean locked two-root qualification workflow",
    );
}

#[test]
fn qualification_verify_accepts_runner_precheck_commands() {
    let fixture = ValidBundle::new();
    fixture.mutate_json("runner/commands.json", |commands| {
        let commands = commands.as_array_mut().expect("commands array");
        for command in commands.iter_mut() {
            command["sequence"] =
                serde_json::json!(command["sequence"].as_u64().expect("command sequence") + 1);
        }
        commands.insert(
            0,
            serde_json::json!({
                "sequence": 1,
                "fia": "PRECHECK",
                "id": "brainmap-version",
                "command": "<brainmap> --version",
                "expectedExit": 0,
                "exitCode": 0,
                "passed": true
            }),
        );
    });
    fixture.refresh_checksums();

    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "candidate binary was not built by the clean locked two-root qualification workflow",
    );
}

#[test]
fn qualification_verify_requires_the_exact_companion_brainmapd() {
    let fixture = ValidBundle::new_for(
        COMMIT,
        sha256_file(Path::new(env!("CARGO_BIN_EXE_brainmap"))),
        "0".repeat(64),
    );

    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "brainmapd hash does not match the companion brainmapd binary",
    );
}

#[test]
fn qualification_verify_rejects_legacy_all_true_self_attestation() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let bundle = tmp.path().join("qualification");
    fs::create_dir(&bundle).expect("create bundle");
    fs::write(
        bundle.join("qualification.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": "brainmap-m8-fia-v1",
            "candidateCommit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "brainmapSha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "fia1": true,
            "fia2": true,
            "fia3": true,
            "fia4": true,
            "fia5": true,
            "fia6": true,
            "fia7": true,
            "fia8": true
        }))
        .expect("serialize legacy manifest"),
    )
    .expect("write legacy manifest");
    write_checksums(&bundle);

    fails(
        &["qualification", "verify", "--bundle", path(&bundle)],
        "legacy flat FIA self-attestation is not accepted",
    );
}

#[test]
fn qualification_verify_rejects_checksum_tampering_and_unchecksummed_files() {
    let tampered = ValidBundle::new();
    fs::write(
        tampered.bundle.join("runner/reports/fia1.json"),
        b"{\"tampered\":true}\n",
    )
    .expect("tamper report");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&tampered.bundle),
        ],
        "checksum mismatch for runner/reports/fia1.json",
    );

    let extra = ValidBundle::new();
    fs::write(extra.bundle.join("host/unchecksummed.log"), "extra\n")
        .expect("write unchecksummed artifact");
    fails(
        &["qualification", "verify", "--bundle", path(&extra.bundle)],
        "SHA256SUMS does not exactly cover its artifact set",
    );
}

#[test]
fn qualification_verify_requires_recursive_sorted_newline_complete_checksums() {
    let nested_extra = ValidBundle::new();
    fs::write(
        nested_extra.bundle.join("runner/unlisted.log"),
        "signed only by the root\n",
    )
    .expect("write nested unlisted artifact");
    write_checksums(&nested_extra.bundle);
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&nested_extra.bundle),
        ],
        "runner/SHA256SUMS does not exactly cover its artifact set",
    );

    let unsorted = ValidBundle::new();
    let checksum_path = unsorted.bundle.join("SHA256SUMS");
    let checksum = fs::read_to_string(&checksum_path).expect("read root checksums");
    let mut lines = checksum.lines().map(str::to_owned).collect::<Vec<_>>();
    lines.swap(0, 1);
    fs::write(&checksum_path, format!("{}\n", lines.join("\n"))).expect("write unsorted checksums");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&unsorted.bundle),
        ],
        "checksum entries must be sorted and unique",
    );

    let incomplete = ValidBundle::new();
    let checksum_path = incomplete.bundle.join("runner/SHA256SUMS");
    let checksum = fs::read_to_string(&checksum_path).expect("read runner checksums");
    fs::write(&checksum_path, checksum.trim_end_matches('\n'))
        .expect("write newline-incomplete checksums");
    write_checksums(&incomplete.bundle);
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&incomplete.bundle),
        ],
        "checksum file is not newline-complete: runner/SHA256SUMS",
    );
}

#[test]
fn qualification_verify_rejects_traversal_and_case_collisions() {
    let traversal = ValidBundle::new();
    let checksum_path = traversal.bundle.join("SHA256SUMS");
    let checksum = fs::read_to_string(&checksum_path).expect("read root checksums");
    let mut lines = checksum.lines().map(str::to_owned).collect::<Vec<_>>();
    let digest = lines[0].split_once("  ").expect("checksum line").0;
    lines[0] = format!("{digest}  ../escape.json");
    fs::write(&checksum_path, format!("{}\n", lines.join("\n")))
        .expect("write traversing checksum path");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&traversal.bundle),
        ],
        "invalid qualification path component",
    );

    let collision = ValidBundle::new();
    let probe_lower = collision.bundle.join("case-probe");
    let probe_upper = collision.bundle.join("CASE-PROBE");
    fs::write(&probe_lower, "probe").expect("write case-sensitivity probe");
    let case_sensitive = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_upper)
        .is_ok();
    fs::remove_file(&probe_lower).expect("remove lower case-sensitivity probe");
    if case_sensitive {
        fs::remove_file(&probe_upper).expect("remove upper case-sensitivity probe");
    } else {
        return;
    }
    fs::copy(
        collision.bundle.join("runner/manifest.json"),
        collision.bundle.join("runner/Manifest.json"),
    )
    .expect("create case-colliding artifact");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&collision.bundle),
        ],
        "case-colliding paths",
    );
}

#[cfg(unix)]
#[test]
fn qualification_verify_rejects_symlinks() {
    use std::os::unix::fs::symlink;

    let fixture = ValidBundle::new();
    symlink(
        fixture.bundle.join("runner/manifest.json"),
        fixture.bundle.join("runner/manifest-link.json"),
    )
    .expect("create artifact symlink");
    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "qualification bundle contains a symlink",
    );
}

#[cfg(unix)]
#[test]
fn qualification_verify_rejects_hard_links() {
    let fixture = ValidBundle::new();
    fs::hard_link(
        fixture.bundle.join("runner/manifest.json"),
        fixture.bundle.join("runner/manifest-hardlink.json"),
    )
    .expect("create artifact hard link");
    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "qualification bundle contains a hard link",
    );
}

#[test]
fn qualification_verify_rejects_cross_candidate_evidence_even_when_resigned() {
    let fixture = ValidBundle::new();
    fixture.mutate_json("release/manifest.json", |manifest| {
        manifest["candidate"]["brainmapdSha256"] = serde_json::json!("c".repeat(64));
    });
    fixture.refresh_checksums();

    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "release candidate or binary hash mismatch",
    );
}

#[test]
fn qualification_verify_binds_commit_and_both_binary_hashes_to_reproducibility() {
    let invalid_commit = ValidBundle::new();
    invalid_commit.mutate_json("qualification.json", |manifest| {
        manifest["candidate"]["commit"] = serde_json::json!("a".repeat(39));
    });
    invalid_commit.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&invalid_commit.bundle),
        ],
        "root candidate commit must be 40 lowercase hexadecimal characters",
    );

    let brainmapd_mismatch = ValidBundle::new();
    brainmapd_mismatch.mutate_json("reproducibility/manifest.json", |manifest| {
        manifest["brainmapdSha256"] = serde_json::json!("c".repeat(64));
    });
    brainmapd_mismatch.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&brainmapd_mismatch.bundle),
        ],
        "reproducibility brainmapd hash mismatch",
    );

    let runner_repro_mismatch = ValidBundle::new();
    runner_repro_mismatch.mutate_json("runner/manifest.json", |manifest| {
        manifest["build"]["reproducibilityManifestSha256"] = serde_json::json!("c".repeat(64));
    });
    runner_repro_mismatch.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&runner_repro_mismatch.bundle),
        ],
        "runner reproducibility manifest hash mismatch",
    );

    let release_repro_mismatch = ValidBundle::new();
    release_repro_mismatch.mutate_json("release/manifest.json", |manifest| {
        manifest["reproducibilityManifestSha256"] = serde_json::json!("d".repeat(64));
    });
    release_repro_mismatch.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&release_repro_mismatch.bundle),
        ],
        "release reproducibility manifest hash mismatch",
    );

    let runner_retained_mismatch = ValidBundle::new();
    runner_retained_mismatch.mutate_json(
        "runner/release-reproducibility-manifest.json",
        |manifest| {
            manifest["cleanTree"] = serde_json::json!(false);
        },
    );
    runner_retained_mismatch.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&runner_retained_mismatch.bundle),
        ],
        "runner retained reproducibility manifest mismatch",
    );
}

#[test]
fn qualification_verify_binds_the_candidate_to_the_running_brainmap_binary() {
    let fixture = ValidBundle::new();
    fixture.retarget_brainmap("9".repeat(64));

    fails(
        &["qualification", "verify", "--bundle", path(&fixture.bundle)],
        "candidate brainmap hash does not match the running brainmap binary",
    );
}

#[test]
fn qualification_verify_rejects_non_qualifying_and_local_runners() {
    let non_qualifying = ValidBundle::new();
    non_qualifying.mutate_json("runner/manifest.json", |manifest| {
        manifest["qualificationEligible"] = serde_json::json!(false);
    });
    non_qualifying.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&non_qualifying.bundle),
        ],
        "runner evidence is non-qualifying",
    );

    let local = ValidBundle::new();
    local.mutate_json("runner/manifest.json", |manifest| {
        manifest["executionMode"] = serde_json::json!("local");
    });
    local.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&local.bundle)],
        "runner must use qualifying docker mode",
    );
}

#[test]
fn qualification_verify_derives_every_runner_fia_from_strict_report_evidence() {
    for (relative, pointer, replacement, expected) in [
        (
            "runner/reports/fia1.json",
            "/behaviorDerived",
            serde_json::json!(false),
            "FIA-1 report does not qualify",
        ),
        (
            "runner/reports/fia2.json",
            "/correctPredictions",
            serde_json::json!(5),
            "FIA-2 report does not qualify",
        ),
        (
            "runner/reports/fia3.json",
            "/actionRecorded",
            serde_json::json!(false),
            "FIA-3 report does not qualify",
        ),
        (
            "runner/reports/fia4.json",
            "/exactCausalPolicySet",
            serde_json::json!(false),
            "FIA-4 report does not qualify",
        ),
        (
            "runner/reports/fia6.json",
            "/uniqueLedgerIds",
            serde_json::json!(47),
            "FIA-6 event identity evidence does not qualify",
        ),
        (
            "runner/reports/fia7.json",
            "/faultPhases",
            serde_json::json!(7),
            "FIA-7 report does not qualify",
        ),
    ] {
        let fixture = ValidBundle::new();
        fixture.mutate_json(relative, |report| {
            *report
                .pointer_mut(pointer)
                .expect("fixture report field exists") = replacement;
        });
        fixture.refresh_checksums();
        fails(
            &["qualification", "verify", "--bundle", path(&fixture.bundle)],
            expected,
        );
    }
}

#[test]
fn qualification_verify_rejects_internally_inconsistent_fia_counts_and_ids() {
    let fia2 = ValidBundle::new();
    fia2.mutate_json("runner/reports/fia2.json", |report| {
        report["paraphrases"] = serde_json::json!(6);
        report["correctPredictions"] = serde_json::json!(6);
    });
    fia2.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&fia2.bundle)],
        "FIA-2 correct prediction count is inconsistent",
    );

    let fia3 = ValidBundle::new();
    fia3.mutate_json("runner/reports/fia3.json", |report| {
        report["moreRelevantRule"] = report["beforeRule"].clone();
    });
    fia3.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&fia3.bundle)],
        "FIA-3 rule identities are not distinct",
    );

    let fia6 = ValidBundle::new();
    fia6.mutate_json("runner/reports/fia6.json", |report| {
        report["operatingSystemProcesses"] = serde_json::json!(65);
        report["gateProcesses"] = serde_json::json!(17);
    });
    fia6.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&fia6.bundle)],
        "FIA-6 process and event counts are inconsistent",
    );

    let duplicate_command = ValidBundle::new();
    duplicate_command.mutate_json("runner/commands.json", |commands| {
        commands[1]["id"] = commands[0]["id"].clone();
    });
    duplicate_command.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&duplicate_command.bundle),
        ],
        "runner command ID is duplicated",
    );

    let unexpected_fia = ValidBundle::new();
    unexpected_fia.mutate_json("runner/commands.json", |commands| {
        commands
            .as_array_mut()
            .expect("commands array")
            .push(serde_json::json!({
                "sequence": 7,
                "fia": "FIA-5",
                "id": "fia-5-unexpected",
                "command": "<brainmap> qualification fixture",
                "expectedExit": 0,
                "exitCode": 0,
                "passed": true
            }));
    });
    unexpected_fia.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&unexpected_fia.bundle),
        ],
        "runner command claims an unsupported FIA",
    );
}

#[test]
fn qualification_verify_rejects_host_trust_bypass_and_reordered_events() {
    let bypassed = ValidBundle::new();
    bypassed.mutate_json("host/manifest.json", |manifest| {
        manifest["adapter"]["trustBypassUsed"] = serde_json::json!(true);
    });
    bypassed.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&bypassed.bundle),
        ],
        "FIA-5 host trust was bypassed or not accepted",
    );

    let reordered = ValidBundle::new();
    let events_path = reordered.bundle.join("host/events.jsonl");
    let text = fs::read_to_string(&events_path).expect("read host events");
    let mut lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    lines.swap(4, 5);
    fs::write(&events_path, format!("{}\n", lines.join("\n"))).expect("reorder events");
    reordered.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&reordered.bundle),
        ],
        "FIA-5 host event order is invalid",
    );
}

#[test]
fn qualification_verify_requires_host_v2_observation_and_active_second_record() {
    let legacy = ValidBundle::new();
    legacy.mutate_json("host/manifest.json", |manifest| {
        manifest["schemaVersion"] = serde_json::json!("brainmap-m8-host-v1");
    });
    legacy.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&legacy.bundle)],
        "unsupported host manifest schema",
    );

    let network_enabled = ValidBundle::new();
    network_enabled.mutate_json("host/host-observation.json", |observation| {
        observation["config"]["workspaceWriteNetworkAccess"] = serde_json::json!(true);
    });
    network_enabled.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&network_enabled.bundle),
        ],
        "FIA-5 host observation safe config does not qualify",
    );

    let unofficial = ValidBundle::new();
    unofficial.mutate_json("host/host-observation.json", |observation| {
        observation["officialCodex"]["binarySha256"] = serde_json::json!("f".repeat(64));
    });
    unofficial.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&unofficial.bundle),
        ],
        "FIA-5 official Codex provenance does not qualify",
    );

    let repeated_decision = ValidBundle::new();
    repeated_decision.mutate_json("host/host-observation.json", |observation| {
        observation["calls"]["second"]["decisionId"] =
            observation["calls"]["first"]["decisionId"].clone();
    });
    repeated_decision.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&repeated_decision.bundle),
        ],
        "FIA-5 host observation call lifecycle does not qualify",
    );

    let missing_second_record = ValidBundle::new();
    missing_second_record.mutate_host_events(|events| {
        events[11]["kind"] = serde_json::json!("changed-outcome-followed");
    });
    missing_second_record.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&missing_second_record.bundle),
        ],
        "FIA-5 host event order is invalid",
    );
}

#[test]
fn qualification_verify_requires_a_real_codex_identity_and_runtime_ids() {
    let fake_host = ValidBundle::new();
    fake_host.mutate_json("host/manifest.json", |manifest| {
        manifest["adapter"]["hostVersion"] = serde_json::json!("fixture");
    });
    fake_host.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&fake_host.bundle),
        ],
        "FIA-5 Codex host version is invalid",
    );

    let bad_decision_id = ValidBundle::new();
    bad_decision_id.mutate_host_events(|events| {
        for event in &mut events[4..] {
            event["decisionId"] = serde_json::json!("fabricated-decision");
        }
    });
    bad_decision_id.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&bad_decision_id.bundle),
        ],
        "FIA-5 decision ID is not a Brainmap runtime ID",
    );

    let bad_packet_id = ValidBundle::new();
    bad_packet_id.mutate_host_events(|events| {
        for event in &mut events[7..] {
            event["packetId"] = serde_json::json!("fabricated-packet");
        }
    });
    bad_packet_id.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&bad_packet_id.bundle),
        ],
        "FIA-5 packet ID is not a Brainmap runtime ID",
    );

    let changed_decision_id = ValidBundle::new();
    changed_decision_id.mutate_host_events(|events| {
        events[6]["decisionId"] = serde_json::json!("dec_1720000000002_cccccccccccc");
    });
    changed_decision_id.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&changed_decision_id.bundle),
        ],
        "FIA-5 decision correlation mismatch",
    );

    let changed_packet_id = ValidBundle::new();
    changed_packet_id.mutate_host_events(|events| {
        events[9]["packetId"] = serde_json::json!("upd_1720000000003_dddddddddddd");
    });
    changed_packet_id.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&changed_packet_id.bundle),
        ],
        "FIA-5 packet correlation mismatch",
    );

    let untrusted_project = ValidBundle::new();
    untrusted_project.mutate_json("host/manifest.json", |manifest| {
        manifest["adapter"]["projectTrusted"] = serde_json::json!(false);
    });
    untrusted_project.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&untrusted_project.bundle),
        ],
        "FIA-5 host trust was bypassed or not accepted",
    );
}

#[test]
fn qualification_verify_rejects_missing_and_failed_release_gates() {
    let missing = ValidBundle::new();
    fs::remove_file(missing.bundle.join("release/gates/format.json"))
        .expect("remove release gate result");
    missing.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&missing.bundle)],
        "qualification artifact is missing: release/gates/format.json",
    );

    let failed = ValidBundle::new();
    failed.mutate_json("release/gates/clippy.json", |gate| {
        gate["passed"] = serde_json::json!(false);
        gate["exitCode"] = serde_json::json!(1);
    });
    failed.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&failed.bundle)],
        "release gate failed: clippy",
    );
}

#[test]
fn qualification_verify_requires_every_fia8_gate_to_pass() {
    for gate in [
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
    ] {
        let fixture = ValidBundle::new();
        fixture.mutate_json(&format!("release/gates/{gate}.json"), |result| {
            result["passed"] = serde_json::json!(false);
            result["exitCode"] = serde_json::json!(1);
        });
        fixture.refresh_checksums();
        fails(
            &["qualification", "verify", "--bundle", path(&fixture.bundle)],
            &format!("release gate failed: {gate}"),
        );
    }
}

#[test]
fn qualification_verify_derives_release_thresholds_from_retained_observations() {
    let eval_failure = ValidBundle::new();
    eval_failure.mutate_json("release/qualification/eval.json", |eval| {
        eval["falseProceed"] = serde_json::json!(1);
    });
    eval_failure.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&eval_failure.bundle),
        ],
        "release evaluation evidence does not satisfy correctness thresholds",
    );

    let scale_failure = ValidBundle::new();
    scale_failure.mutate_json("release/qualification/bench-1000.json", |bench| {
        bench["gateP95Ms"] = serde_json::json!(10.0);
    });
    scale_failure.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&scale_failure.bundle),
        ],
        "release 1k benchmark exceeds its qualification envelope",
    );

    let rebuild_failure = ValidBundle::new();
    rebuild_failure.mutate_json("release/qualification/bench-5000.json", |bench| {
        bench["indexRebuildMs"] = serde_json::json!(1000);
    });
    rebuild_failure.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&rebuild_failure.bundle),
        ],
        "release 5k benchmark exceeds its qualification envelope",
    );

    let missing_fault = ValidBundle::new();
    fs::remove_file(
        missing_fault
            .bundle
            .join("release/qualification/restore-fault-verified-state.json"),
    )
    .expect("remove retained fault observation");
    missing_fault.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&missing_fault.bundle),
        ],
        "qualification artifact is missing: release/qualification/restore-fault-verified-state.json",
    );
}

#[test]
fn qualification_verify_rejects_unknown_and_duplicate_schema_fields() {
    let unknown_report_field = ValidBundle::new();
    unknown_report_field.mutate_json("runner/reports/fia1.json", |report| {
        report["selfAttested"] = serde_json::json!(true);
    });
    unknown_report_field.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&unknown_report_field.bundle),
        ],
        "unknown field `selfAttested`",
    );

    let duplicate_root_field = ValidBundle::new();
    let manifest_path = duplicate_root_field.bundle.join("qualification.json");
    let mut manifest = fs::read_to_string(&manifest_path).expect("read root manifest");
    let final_brace = manifest.rfind('}').expect("root manifest final brace");
    manifest.insert_str(
        final_brace,
        ",\n  \"schemaVersion\": \"brainmap-m8-qualification-bundle-v1\"\n",
    );
    fs::write(&manifest_path, manifest).expect("write duplicate root field");
    write_checksums(&duplicate_root_field.bundle);
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&duplicate_root_field.bundle),
        ],
        "duplicate field `schemaVersion`",
    );
}

#[test]
fn qualification_verify_rejects_every_unknown_evidence_schema() {
    for (relative, expected) in [
        (
            "qualification.json",
            "unsupported qualification bundle schema",
        ),
        (
            "reproducibility/manifest.json",
            "unsupported reproducibility manifest schema",
        ),
        ("runner/manifest.json", "unsupported runner manifest schema"),
        ("host/manifest.json", "unsupported host manifest schema"),
        (
            "host/install-dry-run.json",
            "FIA-5 installer dry-run evidence is invalid",
        ),
        ("host/doctor.json", "FIA-5 doctor evidence is unhealthy"),
        (
            "release/manifest.json",
            "unsupported release manifest schema",
        ),
        (
            "release/gates/format.json",
            "unsupported release gate result schema: format",
        ),
    ] {
        let fixture = ValidBundle::new();
        fixture.mutate_json(relative, |manifest| {
            manifest["schemaVersion"] = serde_json::json!("unsupported-v999");
        });
        fixture.refresh_checksums();
        fails(
            &["qualification", "verify", "--bundle", path(&fixture.bundle)],
            expected,
        );
    }
}

#[test]
fn qualification_verify_rejects_false_privacy_claims_even_when_resigned() {
    for (relative, pointer, expected) in [
        (
            "qualification.json",
            "/privacy/rawPromptsRetained",
            "root privacy claims retained private material",
        ),
        (
            "runner/manifest.json",
            "/privacy/syntheticInputsOnly",
            "runner privacy does not satisfy the synthetic-only privacy contract",
        ),
        (
            "host/manifest.json",
            "/privacy/secretsRetained",
            "host privacy does not satisfy the synthetic-only privacy contract",
        ),
        (
            "release/manifest.json",
            "/privacy/privatePathsRetained",
            "release privacy does not satisfy the synthetic-only privacy contract",
        ),
    ] {
        let fixture = ValidBundle::new();
        fixture.mutate_json(relative, |manifest| {
            let value = manifest
                .pointer_mut(pointer)
                .expect("fixture privacy field exists");
            *value = serde_json::json!(!value.as_bool().expect("privacy field is boolean"));
        });
        fixture.refresh_checksums();
        fails(
            &["qualification", "verify", "--bundle", path(&fixture.bundle)],
            expected,
        );
    }
}

#[test]
fn qualification_verify_scans_all_artifacts_for_prompts_paths_and_secrets() {
    let raw_prompt = ValidBundle::new();
    write_json(
        &raw_prompt.bundle.join("host/raw.json"),
        &serde_json::json!({"prompt": "choose a private tool"}),
    );
    raw_prompt.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&raw_prompt.bundle),
        ],
        "raw prompt or transcript field",
    );

    let private_path = ValidBundle::new();
    write_json(
        &private_path.bundle.join("host/private-path.json"),
        &serde_json::json!({"detail": "/home/developer/private-project"}),
    );
    private_path.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&private_path.bundle),
        ],
        "private absolute path",
    );

    let secret = ValidBundle::new();
    write_json(
        &secret.bundle.join("host/secret.json"),
        &serde_json::json!({"detail": "api_key=abcdefghijklmnop"}),
    );
    secret.refresh_checksums();
    fails(
        &["qualification", "verify", "--bundle", path(&secret.bundle)],
        "secret-like material",
    );

    let sbom_secret = ValidBundle::new();
    sbom_secret.mutate_json("release/sbom/brainmap.cdx.json", |sbom| {
        sbom["credential"] = serde_json::json!("api_key=abcdefghijklmnop");
    });
    sbom_secret.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&sbom_secret.bundle),
        ],
        "secret-like material",
    );

    let sbom_private_email = ValidBundle::new();
    sbom_private_email.mutate_json("release/sbom/brainmap.cdx.json", |sbom| {
        sbom["credentialContact"] = serde_json::json!("private@example.com");
    });
    sbom_private_email.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&sbom_private_email.bundle),
        ],
        "secret-like material: release/sbom/brainmap.cdx.json",
    );

    let sbom_author_email = ValidBundle::new();
    sbom_author_email.mutate_json("release/sbom/brainmap.cdx.json", |sbom| {
        sbom["components"][0]["author"] =
            serde_json::json!("Dependency Author <author@example.com>");
    });
    sbom_author_email.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&sbom_author_email.bundle),
        ],
        "secret-like material: release/sbom/brainmap.cdx.json",
    );

    let extra_sbom_email = ValidBundle::new();
    write_json(
        &extra_sbom_email.bundle.join("release/sbom/extra.json"),
        &serde_json::json!({"contact": "private@example.com"}),
    );
    extra_sbom_email.refresh_checksums();
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&extra_sbom_email.bundle),
        ],
        "secret-like material: release/sbom/extra.json",
    );
}

#[test]
fn qualification_verify_enforces_file_size_and_count_bounds() {
    let oversized = ValidBundle::new();
    let file = fs::File::create(oversized.bundle.join("runner/oversized.log"))
        .expect("create oversized artifact");
    file.set_len(8 * 1024 * 1024 + 1)
        .expect("extend oversized artifact");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&oversized.bundle),
        ],
        "per-file size limit",
    );

    let too_many = ValidBundle::new();
    let directory = too_many.bundle.join("runner/bounds");
    fs::create_dir(&directory).expect("create count-bound directory");
    for index in 0..520 {
        fs::write(directory.join(format!("artifact-{index:04}.log")), "x")
            .expect("write count-bound artifact");
    }
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&too_many.bundle),
        ],
        "file count limit",
    );

    let too_deep = ValidBundle::new();
    let mut directory = too_deep.bundle.join("runner/depth");
    for index in 0..9 {
        directory = directory.join(format!("d{index}"));
    }
    fs::create_dir_all(&directory).expect("create over-deep artifact directory");
    fs::write(directory.join("artifact.log"), "x").expect("write over-deep artifact");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&too_deep.bundle),
        ],
        "directory depth limit",
    );

    let path_too_long = ValidBundle::new();
    let component = "p".repeat(58);
    let directory = path_too_long
        .bundle
        .join("runner")
        .join(&component)
        .join(&component)
        .join(&component)
        .join(&component);
    fs::create_dir_all(&directory).expect("create long artifact path");
    fs::write(directory.join("x.log"), "x").expect("write long-path artifact");
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&path_too_long.bundle),
        ],
        "invalid qualification relative path length",
    );

    let too_large = ValidBundle::new();
    let directory = too_large.bundle.join("runner/total-size");
    fs::create_dir(&directory).expect("create total-size directory");
    for index in 0..8 {
        let file = fs::File::create(directory.join(format!("artifact-{index}.log")))
            .expect("create total-size artifact");
        file.set_len(8 * 1024 * 1024)
            .expect("extend total-size artifact");
    }
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&too_large.bundle),
        ],
        "qualification bundle exceeds total size limit",
    );

    let too_many_entries = ValidBundle::new();
    let directory = too_many_entries.bundle.join("runner/entry-count");
    fs::create_dir(&directory).expect("create entry-count directory");
    for index in 0..1_025 {
        fs::create_dir(directory.join(format!("entry-{index:04}")))
            .expect("create count-bound directory entry");
    }
    fails(
        &[
            "qualification",
            "verify",
            "--bundle",
            path(&too_many_entries.bundle),
        ],
        "qualification bundle exceeds entry count limit",
    );
}

fn fails(args: &[&str], expected_stderr: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .args(args)
        .output()
        .expect("run brainmap");
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
