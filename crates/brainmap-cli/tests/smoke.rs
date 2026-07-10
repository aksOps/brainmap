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
