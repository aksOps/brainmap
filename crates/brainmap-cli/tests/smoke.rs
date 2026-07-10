use std::process::Command;

#[test]
fn brainmap_help_binary_builds() {
    let output = Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .arg("--help")
        .output()
        .expect("run brainmap --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Local deterministic personal decision engine"));
}

#[test]
fn skill_command_prints_dynamic_build_decision_engine_skill() {
    let output = ok(&["skill", "build-decision-engine", "--host", "codex"]);
    assert!(output.contains("Use Brainmap to learn decisions, not knowledge."));
    assert!(output.contains("Local hooks are installed by default."));
    assert!(output.contains("brainmap record-decision"));
    assert!(output.contains("brainmap learn-feedback"));
    assert!(output.contains("brainmap apply --pending --yes"));
    assert!(output.contains("Host: codex."));
}

#[test]
fn production_smoke_cli_flow() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let archive = tmp.path().join("portable.brainmap.tar.zst");
    let tampered = tmp.path().join("tampered.brainmap.tar.zst");
    let imported = tmp.path().join("Imported");
    let restored = tmp.path().join("Restored");
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let suite = workspace.join("fixtures/decision-bench");

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);
    ok(&["index", "verify", "--vault", path(&root)]);
    ok(&["link-check", "--vault", path(&root)]);
    let gate = ok(&[
        "gate",
        "--json",
        "--intent",
        "would-ask-user",
        "--situation",
        "Choose v1 storage",
        "--options",
        "Markdown+JSONL|SQLite|External Vector DB",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "architecture",
        "--vault",
        path(&root),
    ]);
    assert!(gate.contains("\"outcome\""));
    let context = ok(&["context", "--fast", "--json", "--vault", path(&root)]);
    assert!(context.contains("\"hot_path\""));

    ok(&["models", "materialize", "--vault", path(&root)]);
    ok(&["models", "verify", "--vault", path(&root)]);
    let embedded = ok(&["embed", "rebuild", "--vault", path(&root)]);
    assert!(embedded.contains("embedded "));
    let vector = ok(&[
        "search",
        "--vector",
        "local first decisions",
        "--vault",
        path(&root),
    ]);
    assert!(vector.contains("\"score\""));
    let hybrid = ok(&[
        "search",
        "--hybrid",
        "privacy approval",
        "--vault",
        path(&root),
    ]);
    assert!(hybrid.contains("\"graph\""));

    ok(&[
        "export",
        "--mode",
        "portable",
        "--vault",
        path(&root),
        "--out",
        path(&archive),
    ]);
    ok(&["verify-export", path(&archive)]);
    std::fs::copy(&archive, &tampered).expect("copy archive");
    {
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&tampered)
            .expect("open tampered archive")
            .write_all(b"x")
            .expect("append tamper byte");
    }
    fails(&["verify-export", path(&tampered)], "trailing data");
    ok(&[
        "import",
        "--file",
        path(&archive),
        "--to",
        path(&imported),
        "--dry-run",
    ]);
    ok(&["restore", "--file", path(&archive), "--to", path(&restored)]);

    ok(&["snapshot", "create", "--vault", path(&root)]);
    let snapshots = ok(&["snapshot", "list", "--vault", path(&root)]);
    assert!(snapshots.contains(".brainmap.tar.zst"));
    let eval = ok(&["eval", "--vault", path(&root), "--suite", path(&suite)]);
    assert!(eval.contains("\"falseProceed\": 0"));
    assert!(eval.contains("\"falseAsk\": 0"));
    assert!(eval.contains("\"falseBlock\": 0"));
    assert!(eval.contains("\"wrongChoice\": 0"));
}

#[test]
fn bench_scale_cli_reports_envelope_fields() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let output = ok(&["bench", "--vault", path(&root), "--scale", "12"]);
    assert!(output.contains("\"scaleRequested\": 12"));
    assert!(output.contains("\"notes\": 12"));
    assert!(output.contains("\"indexRebuildMs\""));
    assert!(output.contains("\"gateP50Ms\""));
    assert!(output.contains("\"gateP95Ms\""));
    assert!(output.contains("\"candidateBounds\""));
    assert!(output.contains("\"host\""));
    assert!(output.contains("\"contextFastMs\""));
}

#[test]
fn onboarding_answer_file_changes_a_scoped_decision() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let answers = tmp.path().join("answers.json");
    std::fs::write(
        &answers,
        r#"{
  "schemaVersion": "brainmap-onboarding-v1",
  "decisions": [{
    "situation": "Choose package manager for a JavaScript project",
    "decisionType": "tooling",
    "scope": "project:alpha",
    "options": ["npm", "pnpm"],
    "chosen": "pnpm",
    "rejected": ["npm"],
    "rationale": "Use one fast deterministic package manager"
  }]
}"#,
    )
    .expect("write onboarding answers");

    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    let preview = ok(&[
        "onboard",
        "--answers",
        path(&answers),
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(preview.contains("would learn"));
    let before_apply = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose package manager for a JavaScript project",
        "--options",
        "npm|pnpm",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:alpha",
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(before_apply.contains("\"outcome\": \"ask_user\""));
    assert!(before_apply.contains("\"selectedOption\": null"));
    let applied = ok(&[
        "onboard",
        "--answers",
        path(&answers),
        "--vault",
        path(&root),
        "--yes",
    ]);
    assert!(applied.contains("onboarding applied 1 decision"));

    let gate = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose package manager for a JavaScript project",
        "--options",
        "npm|pnpm",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:alpha",
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(gate.contains("\"selectedOption\": \"pnpm\""));
    assert!(gate.contains("\"ruleScope\": \"project:alpha\""));
}

#[test]
fn interactive_onboarding_completes_on_a_clean_vault() {
    use std::io::Write;
    use std::process::Stdio;

    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .args(["onboard", "--vault", path(&root)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn interactive onboarding");
    child
        .stdin
        .take()
        .expect("onboarding stdin")
        .write_all(
            b"Choose formatter for interactive project\nbiome|prettier\nbiome\nprettier\ntooling\nproject:interactive\nFast local formatter\n\ny\n",
        )
        .expect("answer onboarding prompts");
    let output = child.wait_with_output().expect("wait for onboarding");
    assert!(
        output.status.success(),
        "interactive onboarding failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("onboarding applied 1 decision"));

    let result = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose formatter for interactive project",
        "--options",
        "prettier|biome",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:interactive",
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(result.contains("\"selectedOption\": \"biome\""));
}

#[test]
fn codex_integration_doctor_verifies_the_learning_contract() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let project = tmp.path().join("Project");
    std::fs::create_dir_all(&project).expect("create project");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);
    ok(&[
        "install",
        "harness",
        "--target",
        "codex",
        "--project",
        path(&project),
    ]);

    let doctor = ok(&[
        "integration",
        "doctor",
        "--target",
        "codex",
        "--project",
        path(&project),
        "--vault",
        path(&root),
    ]);
    assert!(doctor.contains("\"healthy\": true"));
    assert!(doctor.contains("\"gateReachable\": true"));
    assert!(doctor.contains("\"recordingSupported\": true"));
    assert!(doctor.contains("\"feedbackSupported\": true"));
    assert!(doctor.contains("\"activationRequiresApproval\": true"));
}

#[test]
fn concurrent_processes_preserve_ledgers_ids_capture_and_feedback() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);

    let mut children = Vec::new();
    for _ in 0..16 {
        children.push(
            Command::new(env!("CARGO_BIN_EXE_brainmap"))
                .args([
                    "gate",
                    "--json",
                    "--situation",
                    "Choose formatter for the concurrent project",
                    "--options",
                    "biome|prettier",
                    "--risk",
                    "low",
                    "--reversible",
                    "true",
                    "--decision-type",
                    "tooling",
                    "--scope",
                    "project:concurrency",
                    "--vault",
                    path(&root),
                ])
                .spawn()
                .expect("spawn concurrent gate"),
        );
        children.push(
            Command::new(env!("CARGO_BIN_EXE_brainmap"))
                .args([
                    "record-decision",
                    "--chosen",
                    "biome",
                    "--vault",
                    path(&root),
                ])
                .spawn()
                .expect("spawn concurrent recording"),
        );
    }
    for child in children {
        let output = child
            .wait_with_output()
            .expect("wait for concurrent process");
        assert!(
            output.status.success(),
            "concurrent process failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let ledger_path = root.join("90-calibration/decision-ledger.jsonl");
    let ledger = std::fs::read_to_string(&ledger_path).expect("read decision ledger");
    let events = ledger
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<serde_json::Result<Vec<_>>>()
        .expect("every ledger line is complete JSON");
    assert_eq!(events.len(), 32);
    let ids = events
        .iter()
        .filter_map(|event| event["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), events.len());

    let gate_ids = events
        .iter()
        .filter(|event| event["kind"] == "decision-gate")
        .take(8)
        .filter_map(|event| event["id"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(gate_ids.len(), 8);

    let mut learning_children = Vec::new();
    for (index, decision_id) in gate_ids.iter().enumerate() {
        learning_children.push(
            Command::new(env!("CARGO_BIN_EXE_brainmap"))
                .args([
                    "learn-feedback",
                    "--decision-id",
                    decision_id,
                    "--chosen",
                    "biome",
                    "--rejected",
                    "prettier",
                    "--vault",
                    path(&root),
                ])
                .spawn()
                .expect("spawn concurrent feedback"),
        );
        learning_children.push(
            Command::new(env!("CARGO_BIN_EXE_brainmap"))
                .args([
                    "capture",
                    "--text",
                    &format!("When formatting concurrent project {index}, choose biome"),
                    "--source",
                    "concurrency-test",
                    "--vault",
                    path(&root),
                ])
                .spawn()
                .expect("spawn concurrent capture"),
        );
    }
    for child in learning_children {
        let output = child.wait_with_output().expect("wait for learning process");
        assert!(
            output.status.success(),
            "learning process failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let capture = std::fs::read_to_string(root.join(".brainmap/capture-queue.jsonl"))
        .expect("read capture queue");
    let captures = capture
        .lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<serde_json::Result<Vec<_>>>()
        .expect("every capture line is complete JSON");
    assert_eq!(captures.len(), 8);
    let capture_ids = captures
        .iter()
        .filter_map(|event| event["id"].as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(capture_ids.len(), captures.len());

    let pending = std::fs::read_dir(root.join("99-meta/pending-update-packets"))
        .expect("read pending packets")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert_eq!(pending, 8);
}

#[test]
fn concurrent_update_processes_apply_a_packet_at_most_once() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);
    ok(&[
        "learn-decision",
        "--situation",
        "Choose formatter for serialized updates",
        "--options",
        "biome|prettier",
        "--chosen",
        "biome",
        "--rejected",
        "prettier",
        "--decision-type",
        "tooling",
        "--scope",
        "project:serialization",
        "--vault",
        path(&root),
    ]);

    let children = (0..2)
        .map(|_| {
            Command::new(env!("CARGO_BIN_EXE_brainmap"))
                .args(["apply", "--pending", "--yes", "--vault", path(&root)])
                .spawn()
                .expect("spawn concurrent update application")
        })
        .collect::<Vec<_>>();
    let outputs = children
        .into_iter()
        .map(|child| child.wait_with_output().expect("wait for update process"))
        .collect::<Vec<_>>();
    assert!(outputs.iter().any(|output| output.status.success()));

    let packet_dir = root.join("99-meta/pending-update-packets");
    let applied_packets = std::fs::read_dir(&packet_dir)
        .expect("read packet directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".applied.json"))
        })
        .count();
    let pending_packets = std::fs::read_dir(&packet_dir)
        .expect("read packet directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .filter(|entry| {
            !entry
                .path()
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".applied.json"))
        })
        .count();
    assert_eq!(applied_packets, 1);
    assert_eq!(pending_packets, 0);

    let gate = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose formatter for serialized updates",
        "--options",
        "biome|prettier",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:serialization",
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(gate.contains("\"selectedOption\": \"biome\""));
}

#[test]
fn export_restore_preserves_learned_corrected_and_policy_decisions() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let restored = tmp.path().join("Restored");
    let archive = tmp.path().join("behavior.brainmap.tar.zst");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);

    ok(&[
        "learn-decision",
        "--situation",
        "Choose formatter for restore project",
        "--options",
        "biome|prettier",
        "--chosen",
        "biome",
        "--rejected",
        "prettier",
        "--decision-type",
        "tooling",
        "--scope",
        "project:restore",
        "--vault",
        path(&root),
    ]);
    ok(&["apply", "--pending", "--yes", "--vault", path(&root)]);

    let correction_source = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose package manager for restore project",
        "--options",
        "npm|pnpm",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:restore",
        "--vault",
        path(&root),
    ]);
    let correction_source: serde_json::Value =
        serde_json::from_str(&correction_source).expect("parse correction source");
    let decision_id = correction_source["decisionId"]
        .as_str()
        .expect("decision id");
    ok(&[
        "learn-feedback",
        "--decision-id",
        decision_id,
        "--chosen",
        "pnpm",
        "--rejected",
        "npm",
        "--vault",
        path(&root),
    ]);
    ok(&["apply", "--pending", "--yes", "--vault", path(&root)]);

    std::fs::write(
        root.join("20-decision-frames/restore-policy.md"),
        r#"---
id: restore-policy
type: decision-policy
status: active
confidence: high
risk_tier: reversible-auto
sensitivity: personal
---
# Restore policy

<!-- brainmap-decision-rule:v1 {"situation":"Choose test runner for restore project","decision_type":"tooling","scope":"project:restore","options":["cargo nextest","cargo test"],"chosen":"cargo nextest","rejected":["cargo test"]} -->
"#,
    )
    .expect("write executable policy");
    ok(&["index", "rebuild", "--vault", path(&root)]);

    let requests = [
        (
            "Choose formatter for restore project",
            "biome|prettier",
            "biome",
        ),
        (
            "Choose package manager for restore project",
            "npm|pnpm",
            "pnpm",
        ),
        (
            "Choose test runner for restore project",
            "cargo nextest|cargo test",
            "cargo nextest",
        ),
    ];
    let evaluate = |vault: &std::path::Path, situation: &str, options: &str| {
        let output = ok(&[
            "gate",
            "--json",
            "--situation",
            situation,
            "--options",
            options,
            "--risk",
            "low",
            "--reversible",
            "true",
            "--decision-type",
            "tooling",
            "--scope",
            "project:restore",
            "--vault",
            path(vault),
            "--dry-run",
        ]);
        serde_json::from_str::<serde_json::Value>(&output).expect("parse gate output")
    };
    let before = requests
        .iter()
        .map(|(situation, options, expected)| {
            let result = evaluate(&root, situation, options);
            assert_eq!(result["selectedOption"], *expected);
            result
        })
        .collect::<Vec<_>>();

    ok(&[
        "export",
        "--mode",
        "portable",
        "--vault",
        path(&root),
        "--out",
        path(&archive),
    ]);
    ok(&["verify-export", path(&archive)]);
    ok(&["restore", "--file", path(&archive), "--to", path(&restored)]);

    for ((situation, options, expected), baseline) in requests.iter().zip(before) {
        let result = evaluate(&restored, situation, options);
        assert_eq!(result["selectedOption"], *expected);
        for field in [
            "outcome",
            "selectedOption",
            "rejectedOptions",
            "confidence",
            "ruleId",
            "ruleScope",
            "matchScore",
            "matchKind",
            "appliedPolicies",
            "restrictionsApplied",
        ] {
            assert_eq!(
                result[field], baseline[field],
                "changed {field} after restore"
            );
        }
    }
}

fn ok(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .args(args)
        .output()
        .expect("run brainmap");
    assert!(
        output.status.success(),
        "brainmap {args:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
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

fn path(path: &std::path::Path) -> &str {
    path.to_str().expect("test path is utf-8")
}
