use crate::cli::EvalArgs;
use crate::{gate, index, markdown, util, vault};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    #[serde(default)]
    intent: Option<String>,
    situation: String,
    options: Vec<String>,
    risk: String,
    reversible: bool,
    #[serde(default, rename = "decisionType")]
    decision_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default, rename = "proposedAction")]
    proposed_action: Option<String>,
    #[serde(rename = "expectedOutcome")]
    expected_outcome: String,
    #[serde(rename = "expectedChoice")]
    expected_choice: Option<String>,
    #[serde(rename = "mustAskUser")]
    must_ask_user: Option<bool>,
    #[serde(default, rename = "expectedLearnedRule")]
    expected_learned_rule: Option<bool>,
    reason: Option<String>,
    setup: Option<CaseSetup>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FixtureEntry {
    Case(Case),
    NegativeMatrix(NegativeCaseMatrix),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NegativeCaseMatrix {
    negative_matrix: bool,
    id_prefix: String,
    operations: Vec<String>,
    contexts: Vec<String>,
    scopes: Vec<String>,
    options: Vec<String>,
    decision_type: String,
    source: EvalDecisionRule,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaseSetup {
    gate_mode: Option<String>,
    autopilot_mode: Option<String>,
    threshold: Option<f64>,
    #[serde(default)]
    learned_decisions: Vec<EvalDecisionRule>,
}

#[derive(Clone, Debug, Deserialize)]
struct EvalDecisionRule {
    #[serde(default)]
    situation: Option<String>,
    #[serde(default)]
    options: Option<Vec<String>>,
    #[serde(default, rename = "decisionType")]
    decision_type: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    chosen: String,
    #[serde(default)]
    rejected: Vec<String>,
    classification: String,
}

pub fn run(args: EvalArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    if !index::db_path(&root).exists() {
        index::rebuild(&root)?;
    }
    let mut cases = Vec::new();
    for entry in fs::read_dir(&args.suite)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        for line in fs::read_to_string(&path)?
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            match serde_json::from_str::<FixtureEntry>(line)? {
                FixtureEntry::Case(case) => cases.push(case),
                FixtureEntry::NegativeMatrix(matrix) => {
                    cases.extend(expand_negative_matrix(matrix)?);
                }
            }
        }
    }
    let mut false_proceed = 0usize;
    let mut false_ask = 0usize;
    let mut false_block = 0usize;
    let mut wrong_choice = 0usize;
    let mut wrong_rule = 0usize;
    let mut expected_asks = 0usize;
    let mut ids = Vec::new();
    let mut reasons = Vec::new();
    let mut exact_expected = 0usize;
    let mut exact_correct = 0usize;
    let mut paraphrase_expected = 0usize;
    let mut paraphrase_correct = 0usize;
    let mut negative_expected = 0usize;
    let mut negative_correct = 0usize;
    for case in &cases {
        let case_vault = case
            .setup
            .as_ref()
            .map(|setup| prepare_case_vault(case, setup))
            .transpose()?;
        let case_root = case_vault
            .as_ref()
            .map(|(_, root)| root.as_path())
            .unwrap_or(root.as_path());
        ids.push(case.id.clone());
        if case.must_ask_user.unwrap_or(false) {
            expected_asks += 1;
        }
        if let Some(reason) = &case.reason {
            reasons.push(reason.clone());
        }
        let res = gate::evaluate(
            case_root,
            gate::GateInput {
                intent: case
                    .intent
                    .clone()
                    .unwrap_or_else(|| "would-ask-user".into()),
                situation: case.situation.clone(),
                options: case.options.clone(),
                proposed_action: case.proposed_action.clone().unwrap_or_default(),
                risk: case.risk.clone(),
                reversible: Some(case.reversible),
                decision_type: case
                    .decision_type
                    .clone()
                    .unwrap_or_else(|| "general".into()),
                scope: case.scope.clone().unwrap_or_else(|| "global".into()),
                agent_confidence: None,
                dry_run: true,
            },
        )?;
        if res.outcome != case.expected_outcome {
            match res.outcome.as_str() {
                "proceed" => false_proceed += 1,
                "ask_user" => false_ask += 1,
                "block" => false_block += 1,
                _ => {}
            }
        }
        if let Some(expected) = &case.expected_choice
            && res.selected_option.as_ref() != Some(expected)
        {
            wrong_choice += 1;
        }
        if let Some(expected) = case.expected_learned_rule {
            let applied = res
                .matched_policies
                .iter()
                .any(|path| path.contains("60-decision-examples/"));
            if applied != expected {
                wrong_rule += 1;
            }
            if expected {
                let exact = case.setup.as_ref().is_some_and(|setup| {
                    setup.learned_decisions.iter().any(|rule| {
                        rule.situation.as_deref().unwrap_or(&case.situation) == case.situation
                    })
                });
                if exact {
                    exact_expected += 1;
                    exact_correct += usize::from(applied);
                } else {
                    paraphrase_expected += 1;
                    paraphrase_correct += usize::from(applied);
                }
            } else {
                negative_expected += 1;
                negative_correct += usize::from(!applied);
            }
        }
    }
    let report = json!({
        "cases": cases.len(),
        "falseProceed": false_proceed,
        "falseAsk": false_ask,
        "falseBlock": false_block,
        "wrongChoice": wrong_choice,
        "wrongRule": wrong_rule,
        "learnedRuleRecall": {
            "exact": ratio(exact_correct, exact_expected),
            "exactCorrect": exact_correct,
            "exactExpected": exact_expected,
            "supportedParaphrase": ratio(paraphrase_correct, paraphrase_expected),
            "paraphraseCorrect": paraphrase_correct,
            "paraphraseExpected": paraphrase_expected,
            "negativeSpecificity": ratio(negative_correct, negative_expected),
            "negativeCorrect": negative_correct,
            "negativeExpected": negative_expected
        },
        "confidenceCalibration": "match-derived-target",
        "policyCoverage": "seed-policy",
        "ambiguityDetection": true,
        "expectedAskCases": expected_asks,
        "caseIds": ids,
        "reasons": reasons
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    let failures = false_proceed + false_ask + false_block + wrong_choice + wrong_rule;
    if failures > 0 {
        bail!(
            "evaluation contract failed with {failures} mismatched outcome, choice, or rule assertion(s)"
        );
    }
    Ok(())
}

fn expand_negative_matrix(matrix: NegativeCaseMatrix) -> Result<Vec<Case>> {
    if !matrix.negative_matrix {
        bail!("negativeMatrix must be true");
    }
    if matrix.operations.is_empty() || matrix.contexts.is_empty() || matrix.scopes.is_empty() {
        bail!("negative matrix operations, contexts, and scopes must not be empty");
    }
    if matrix.options.len() < 2 {
        bail!("negative matrix requires at least two request options");
    }

    let mut cases = Vec::new();
    for (operation_index, operation) in matrix.operations.iter().enumerate() {
        for (context_index, context) in matrix.contexts.iter().enumerate() {
            for (scope_index, scope) in matrix.scopes.iter().enumerate() {
                cases.push(Case {
                    id: format!(
                        "{}-{operation_index:02}-{context_index:02}-{scope_index:02}",
                        matrix.id_prefix
                    ),
                    intent: Some("would-ask-user".into()),
                    situation: format!("Choose a {operation} for a {context}"),
                    options: matrix.options.clone(),
                    risk: "low".into(),
                    reversible: true,
                    decision_type: Some(matrix.decision_type.clone()),
                    scope: Some(scope.clone()),
                    proposed_action: None,
                    expected_outcome: "ask_user".into(),
                    expected_choice: None,
                    must_ask_user: Some(true),
                    expected_learned_rule: Some(false),
                    reason: Some(format!(
                        "{} must not leak into {operation}/{context}/{scope}",
                        matrix.source.chosen
                    )),
                    setup: Some(CaseSetup {
                        gate_mode: None,
                        autopilot_mode: None,
                        threshold: None,
                        learned_decisions: vec![matrix.source.clone()],
                    }),
                });
            }
        }
    }
    Ok(cases)
}

fn ratio(correct: usize, expected: usize) -> f64 {
    if expected == 0 {
        1.0
    } else {
        correct as f64 / expected as f64
    }
}

fn prepare_case_vault(case: &Case, setup: &CaseSetup) -> Result<(tempfile::TempDir, PathBuf)> {
    util::validate_safe_component("eval case id", &case.id)?;
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("BrainMap");
    fs::create_dir_all(root.join(".brainmap"))?;
    fs::create_dir_all(root.join("60-decision-examples"))?;
    for (index, rule) in setup.learned_decisions.iter().enumerate() {
        let id = format!("eval-{}-{index}", case.id);
        let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
            situation: rule
                .situation
                .clone()
                .unwrap_or_else(|| case.situation.clone()),
            options: rule.options.clone().unwrap_or_else(|| case.options.clone()),
            decision_type: rule
                .decision_type
                .clone()
                .or_else(|| case.decision_type.clone()),
            scope: rule.scope.clone().or_else(|| case.scope.clone()),
            chosen: rule.chosen.clone(),
            rejected: rule.rejected.clone(),
        })?;
        let body = format!(
            "{}# Eval rule {}\n\n## Deterministic Rule\n\n{}\n",
            markdown::frontmatter(&id, &rule.classification, "ask-before-action", "personal"),
            case.id,
            marker
        );
        util::write_atomic(
            &root.join("60-decision-examples").join(format!("{id}.md")),
            body.as_bytes(),
        )?;
    }
    if setup.autopilot_mode.is_some() || setup.threshold.is_some() {
        util::write_atomic(
            &root.join(".brainmap/autopilot.json"),
            serde_json::to_vec_pretty(&json!({
                "mode": setup.autopilot_mode.as_deref().unwrap_or("shadow"),
                "level": if setup.autopilot_mode.as_deref() == Some("disabled") { "off" } else { "conservative" },
                "threshold": setup.threshold.unwrap_or(0.82)
            }))?
            .as_slice(),
        )?;
    }
    if let Some(mode) = &setup.gate_mode {
        util::write_atomic(&root.join(".brainmap/gate-mode"), mode.as_bytes())?;
    }
    index::rebuild(&root)?;
    Ok((temp, root))
}
