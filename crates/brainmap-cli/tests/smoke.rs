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
    assert!(eval.contains("\"wrongMetadata\": 0"));
    let report: serde_json::Value = serde_json::from_str(&eval).expect("parse eval report");
    let recall = &report["learnedRuleRecall"];
    let exact_expected = recall["exactExpected"].as_u64().expect("exact denominator");
    let exact_correct = recall["exactCorrect"].as_u64().expect("exact numerator");
    let paraphrase_expected = recall["paraphraseExpected"]
        .as_u64()
        .expect("paraphrase denominator");
    let paraphrase_correct = recall["paraphraseCorrect"]
        .as_u64()
        .expect("paraphrase numerator");
    let negative_expected = recall["negativeExpected"]
        .as_u64()
        .expect("negative denominator");
    let negative_correct = recall["negativeCorrect"]
        .as_u64()
        .expect("negative numerator");
    assert!(exact_expected > 0);
    assert_eq!(exact_correct, exact_expected, "exact recall must be 100%");
    assert!(paraphrase_expected >= 5);
    assert!(
        paraphrase_correct * 100 >= paraphrase_expected * 95,
        "supported paraphrase recall must be at least 95%"
    );
    assert!(
        negative_expected >= 100,
        "the executable eval suite must contain at least 100 negative cases"
    );
    assert_eq!(
        negative_correct, negative_expected,
        "negative cases must have zero learned-rule applications"
    );
}

#[test]
fn bench_scale_cli_reports_envelope_fields() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let output = ok(&["bench", "--vault", path(&root), "--scale", "140"]);
    assert!(output.contains("\"scaleRequested\": 140"));
    assert!(output.contains("\"notes\": 140"));
    assert!(output.contains("\"indexRebuildMs\""));
    assert!(output.contains("\"gateP50Ms\""));
    assert!(output.contains("\"gateP95Ms\""));
    assert!(output.contains("\"candidateBounds\""));
    assert!(output.contains("\"host\""));
    assert!(output.contains("\"contextFastMs\""));
    let report: serde_json::Value = serde_json::from_str(&output).expect("benchmark JSON");
    assert_eq!(report["candidateBounds"]["maximumFuzzyRowsScored"], 40);
    assert_eq!(report["candidateBounds"]["rowsPerTerm"], 5_000);
    assert_eq!(report["candidateBounds"]["executableRules"], 5_000);
    assert_eq!(
        report["candidateBounds"]["retrieval"],
        "actual-rule-term-postings"
    );
    assert_eq!(report["unavailableChoiceProbe"]["outcome"], "ask_user");
    assert_eq!(report["unavailableChoiceProbe"]["matchKind"], "fuzzy");
    assert_eq!(
        report["unavailableChoiceProbe"]["candidateCollision"],
        false
    );
    assert!(
        report["unavailableChoiceProbe"]["matchedPolicies"]
            .as_array()
            .is_some_and(|policies| policies.iter().any(|policy| {
                policy
                    .as_str()
                    .is_some_and(|path| path.contains("bench-decision-00000"))
            }))
    );
}

#[test]
fn eval_exits_nonzero_when_an_expected_result_is_wrong() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let suite = tmp.path().join("suite");
    std::fs::create_dir_all(&suite).expect("create eval suite");
    std::fs::write(
        suite.join("failing.jsonl"),
        r#"{"id":"expected-mismatch","situation":"Choose an unlearned tool","options":["A","B"],"risk":"low","reversible":true,"expectedOutcome":"proceed","expectedChoice":null}
"#,
    )
    .expect("write failing eval case");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);

    fails(
        &["eval", "--vault", path(&root), "--suite", path(&suite)],
        "evaluation contract failed",
    );
}

#[test]
fn legacy_decision_commands_record_project_narrow_defaults() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);

    ok(&[
        "decide",
        "Choose a local test runner",
        "--options",
        "cargo-nextest|cargo-test",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--json",
        "--vault",
        path(&root),
    ]);
    ok(&[
        "should-ask-user",
        "--question",
        "Which local test runner should I use?",
        "--json",
        "--vault",
        path(&root),
    ]);

    let ledger = std::fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl"))
        .expect("read decision ledger");
    let records = ledger
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("parse ledger row"))
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| {
        record["scope"]
            .as_str()
            .is_some_and(|scope| scope.starts_with("project:"))
    }));
    assert!(records.iter().all(|record| record["scope"] != "global"));
}

#[test]
fn gate_blocks_secret_in_risk_without_persisting_it() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    let secret = "api_key=abcdef1234567890";
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);

    let output = ok(&[
        "gate",
        "--json",
        "--intent",
        "plan",
        "--situation",
        "Choose a local formatter",
        "--options",
        "biome|prettier",
        "--risk",
        secret,
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:secret-regression",
        "--vault",
        path(&root),
    ]);
    let result: serde_json::Value = serde_json::from_str(&output).expect("parse gate result");
    assert_eq!(result["outcome"], "block");

    let ledger = std::fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl"))
        .expect("read decision ledger");
    assert!(!ledger.contains(secret));
    let event: serde_json::Value =
        serde_json::from_str(ledger.lines().last().expect("ledger event")).expect("parse event");
    assert_eq!(event["risk"], "[REDACTED]");
}

#[test]
fn structured_feedback_incidents_drive_prompt_free_shadow_metrics() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);
    ok(&[
        "learn-decision",
        "--situation",
        "Choose formatter for incident project",
        "--options",
        "biome|prettier",
        "--chosen",
        "biome",
        "--rejected",
        "prettier",
        "--decision-type",
        "tooling",
        "--scope",
        "project:incident",
        "--vault",
        path(&root),
    ]);
    ok(&["apply", "--pending", "--yes", "--vault", path(&root)]);

    let learned = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose formatter for incident project",
        "--options",
        "biome|prettier",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:incident",
        "--vault",
        path(&root),
    ]);
    let learned: serde_json::Value = serde_json::from_str(&learned).expect("parse learned gate");
    assert_eq!(learned["outcome"], "proceed");
    ok(&[
        "learn-feedback",
        "--decision-id",
        learned["decisionId"].as_str().expect("learned decision id"),
        "--chosen",
        "prettier",
        "--rejected",
        "biome",
        "--incident",
        "cross-domain-application",
        "--vault",
        path(&root),
    ]);

    let policy = ok(&[
        "gate",
        "--json",
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
        "--scope",
        "global",
        "--vault",
        path(&root),
    ]);
    let policy: serde_json::Value = serde_json::from_str(&policy).expect("parse policy gate");
    assert_eq!(policy["outcome"], "proceed");
    fails(
        &[
            "learn-feedback",
            "--decision-id",
            policy["decisionId"].as_str().expect("policy decision id"),
            "--chosen",
            "SQLite",
            "--incident",
            "cross-domain-application",
            "--vault",
            path(&root),
        ],
        "requires an applied learned decision rule",
    );
    ok(&[
        "learn-feedback",
        "--decision-id",
        policy["decisionId"].as_str().expect("policy decision id"),
        "--chosen",
        "SQLite",
        "--rejected",
        "Markdown+JSONL",
        "--incident",
        "false-proceed",
        "--vault",
        path(&root),
    ]);

    let unlearned = ok(&[
        "gate",
        "--json",
        "--situation",
        "Choose an unlearned deployment tool",
        "--options",
        "A|B",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--scope",
        "project:incident",
        "--vault",
        path(&root),
    ]);
    let unlearned: serde_json::Value =
        serde_json::from_str(&unlearned).expect("parse unlearned gate");
    assert_eq!(unlearned["outcome"], "ask_user");
    fails(
        &[
            "learn-feedback",
            "--decision-id",
            unlearned["decisionId"].as_str().expect("unlearned id"),
            "--chosen",
            "A",
            "--incident",
            "false-proceed",
            "--vault",
            path(&root),
        ],
        "requires an original proceed outcome",
    );

    let status = ok(&["autopilot", "status", "--vault", path(&root)]);
    let status: serde_json::Value = serde_json::from_str(&status).expect("parse autopilot status");
    let metrics = &status["shadowMetrics"];
    assert_eq!(metrics["confirmedCrossDomainApplications"], 1);
    assert_eq!(metrics["falseProceeds"], 1);
    assert_eq!(metrics["privacyViolations"], 0);
    assert_eq!(metrics["rawPromptsRetained"], false);
    assert!(
        !status
            .to_string()
            .contains("Choose formatter for incident project")
    );
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
    let before_preview = tree_snapshot(&root);
    let preview = ok(&[
        "onboard",
        "--answers",
        path(&answers),
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(preview.contains("onboarding exact executable update preview"));
    assert_eq!(tree_snapshot(&root), before_preview);
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
            b"follow project configuration\nask user\nmake the smallest reversible change\n\ny\n",
        )
        .expect("answer onboarding prompts");
    let output = child.wait_with_output().expect("wait for onboarding");
    assert!(
        output.status.success(),
        "interactive onboarding failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Calibration 1/3"));
    assert!(stdout.contains("Calibration 2/3"));
    assert!(stdout.contains("Calibration 3/3"));
    assert!(stdout.contains("onboarding applied 3 decision"));
    let preview: serde_json::Value = stdout
        .lines()
        .find_map(|line| {
            line.strip_prefix("onboarding exact executable update preview: ")
                .map(|json| serde_json::from_str(json).expect("parse exact onboarding preview"))
        })
        .expect("exact executable onboarding preview");
    assert_eq!(
        preview["packet"]["decisionRule"]["options"],
        serde_json::json!(["follow project configuration", "ask user"])
    );
    assert_eq!(
        preview["packet"]["decisionRule"]["rejected"],
        serde_json::json!(["ask user"])
    );
    let packet_id = preview["packet"]["id"].as_str().expect("preview packet id");
    let applied_packet: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            root.join("99-meta/pending-update-packets")
                .join(format!("manual-decision-{packet_id}.applied.json")),
        )
        .expect("read exact applied onboarding packet"),
    )
    .expect("parse exact applied onboarding packet");
    assert_eq!(preview["packet"], applied_packet);

    let result = ok(&[
        "gate",
        "--json",
        "--situation",
        "When a project declares a formatter, choose the formatter policy",
        "--options",
        "ask user|follow project configuration",
        "--risk",
        "low",
        "--reversible",
        "true",
        "--decision-type",
        "tooling",
        "--vault",
        path(&root),
        "--dry-run",
    ]);
    assert!(result.contains("\"selectedOption\": \"follow project configuration\""));
    assert!(result.contains("\"ruleScope\": \"project:"));
    assert!(!result.contains("\"ruleScope\": \"global\""));
}

#[test]
fn onboarding_rejects_secrets_in_pending_metadata() {
    for (field, secret) in [
        ("decisionType", "api_key=abcdef1234567890"),
        ("scope", "project:api_key=abcdef1234567890"),
    ] {
        let tmp = tempfile::tempdir().expect("temp dir");
        let root = tmp.path().join("BrainMap");
        let answers = tmp.path().join("answers.json");
        let mut decision = serde_json::json!({
            "situation": "Choose an ambiguous local workflow",
            "decisionType": "workflow",
            "scope": "project:metadata-secret",
            "freeText": "It depends on the current change"
        });
        decision[field] = serde_json::json!(secret);
        std::fs::write(
            &answers,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schemaVersion": "brainmap-onboarding-v1",
                "decisions": [decision]
            }))
            .expect("serialize answers"),
        )
        .expect("write answers");
        ok(&["init-vault", "--vault", path(&root), "--yes"]);

        fails(
            &[
                "onboard",
                "--answers",
                path(&answers),
                "--vault",
                path(&root),
                "--yes",
            ],
            "secret-like material",
        );
        let pending = root.join("90-calibration/pending-onboarding.jsonl");
        assert!(!pending.exists());
    }
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
        "--vault",
        path(&root),
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
fn codex_integration_doctor_rejects_invalid_toml_configuration() {
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
        "--vault",
        path(&root),
    ]);
    let config = project.join(".codex/config.toml");
    let mut invalid = std::fs::read_to_string(&config).expect("read Codex config");
    invalid.push_str("\ninvalid = [\n");
    std::fs::write(&config, invalid).expect("corrupt Codex config");

    fails(
        &[
            "integration",
            "doctor",
            "--target",
            "codex",
            "--project",
            path(&project),
            "--vault",
            path(&root),
        ],
        "invalid host configuration",
    );
}

#[test]
fn codex_mcp_adapter_completes_the_learning_lifecycle() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::process::Stdio;

    let tmp = tempfile::tempdir().expect("temp dir");
    let root = tmp.path().join("BrainMap");
    ok(&["init-vault", "--vault", path(&root), "--yes"]);
    ok(&["index", "rebuild", "--vault", path(&root)]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .args(["mcp", "serve", "--vault", path(&root)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Brainmap MCP adapter");
    let mut stdin = child.stdin.take().expect("MCP stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
    let mut stderr = child.stderr.take().expect("MCP stderr");
    {
        let mut request_id = 0u64;
        let mut call = |name: &str, arguments: serde_json::Value| {
            request_id += 1;
            let request = serde_json::json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            });
            writeln!(stdin, "{}", serde_json::to_string(&request).unwrap()).unwrap();
            stdin.flush().unwrap();
            let mut line = String::new();
            let read = stdout.read_line(&mut line).unwrap();
            if read == 0 {
                let mut error = String::new();
                stderr.read_to_string(&mut error).unwrap();
                panic!("MCP server closed stdout: {error}");
            }
            let response: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert!(response.get("error").is_none(), "MCP error: {response}");
            let text = response["result"]["content"][0]["text"]
                .as_str()
                .expect("MCP text result");
            serde_json::from_str::<serde_json::Value>(text).expect("parse MCP tool payload")
        };

        let gate_arguments = serde_json::json!({
            "intent": "would-ask-user",
            "situation": "Choose package manager through Codex MCP",
            "options": ["npm", "pnpm"],
            "risk": "low",
            "reversible": true,
            "decisionType": "tooling",
            "scope": "project:codex-mcp"
        });
        let first = call("brainmap_decision_gate", gate_arguments.clone());
        assert_eq!(first["outcome"], "ask_user");
        let decision_id = first["decisionId"].as_str().unwrap();

        let recorded = call(
            "brainmap_record_decision",
            serde_json::json!({
                "decisionId": decision_id,
                "chosen": "pnpm",
                "wasAsked": true
            }),
        );
        assert_eq!(recorded["recorded"], true);
        let feedback = call(
            "brainmap_learn_feedback",
            serde_json::json!({
                "decisionId": decision_id,
                "chosen": "pnpm",
                "rejected": "npm"
            }),
        );
        assert_eq!(feedback["packetCreated"], true);

        let pending = call("brainmap_list_pending", serde_json::json!({}));
        let packet_id = pending[0]["id"].as_str().unwrap();
        let preview = call(
            "brainmap_preview_update",
            serde_json::json!({"packetId": packet_id}),
        );
        assert_eq!(preview[0]["id"], packet_id);
        let applied = call(
            "brainmap_apply_update",
            serde_json::json!({"packetId": packet_id, "approved": true}),
        );
        assert_eq!(applied["applied"], true);

        let changed = call("brainmap_decision_gate", gate_arguments);
        assert_eq!(changed["selectedOption"], "pnpm");
        assert_eq!(changed["ruleScope"], "project:codex-mcp");
    }

    drop(stdin);
    let status = child.wait().expect("wait for MCP adapter");
    assert!(status.success());
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

    let packet_dir = root.join("99-meta/pending-update-packets");
    let pending = std::fs::read_dir(&packet_dir)
        .expect("read pending packets")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert_eq!(pending, 8);

    ok(&["apply", "--pending", "--yes", "--vault", path(&root)]);
    let applied = std::fs::read_dir(&packet_dir)
        .expect("read applied packets")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".applied.json"))
        })
        .map(|entry| {
            serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(entry.path()).expect("read applied packet"),
            )
            .expect("parse applied packet")
        })
        .collect::<Vec<_>>();
    assert_eq!(applied.len(), 8);
    for packet in applied {
        let packet_id = packet["id"].as_str().expect("applied packet id");
        let note = root
            .join("60-decision-examples")
            .join(format!("{packet_id}.md"));
        let body = std::fs::read_to_string(&note).expect("read applied canonical note");
        assert!(body.contains("brainmap-decision-rule:v1"));
    }
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
status: tested
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

fn tree_snapshot(root: &std::path::Path) -> Vec<(String, Option<Vec<u8>>)> {
    fn visit(
        root: &std::path::Path,
        directory: &std::path::Path,
        entries: &mut Vec<(String, Option<Vec<u8>>)>,
    ) {
        for entry in std::fs::read_dir(directory).expect("read snapshot directory") {
            let path = entry.expect("snapshot entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("snapshot path under root")
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                entries.push((relative, None));
                visit(root, &path, entries);
            } else {
                entries.push((
                    relative,
                    Some(std::fs::read(path).expect("read snapshot file")),
                ));
            }
        }
    }

    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
