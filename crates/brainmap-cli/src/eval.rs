use crate::cli::EvalArgs;
use crate::{gate, index, markdown, util, vault};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, PartialEq, Eq)]
enum ExpectedChoice {
    #[default]
    Unspecified,
    NoSelection,
    Selection(String),
}

fn deserialize_expected_choice<'de, D>(deserializer: D) -> Result<ExpectedChoice, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(choice) => ExpectedChoice::Selection(choice),
        None => ExpectedChoice::NoSelection,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(
        default,
        rename = "expectedChoice",
        deserialize_with = "deserialize_expected_choice"
    )]
    expected_choice: ExpectedChoice,
    #[serde(default, rename = "expectedPredictedOutcome")]
    expected_predicted_outcome: Option<String>,
    #[serde(
        default,
        rename = "expectedPredictedChoice",
        deserialize_with = "deserialize_expected_choice"
    )]
    expected_predicted_choice: ExpectedChoice,
    #[serde(rename = "mustAskUser")]
    must_ask_user: Option<bool>,
    #[serde(default, rename = "expectedLearnedRule")]
    expected_learned_rule: Option<bool>,
    #[serde(default, rename = "expectedCandidateCollision")]
    expected_candidate_collision: Option<bool>,
    #[serde(default, rename = "expectedMatchKind")]
    expected_match_kind: Option<String>,
    #[serde(default, rename = "minimumMatchMargin")]
    minimum_match_margin: Option<f64>,
    #[serde(default, rename = "maximumMatchMargin")]
    maximum_match_margin: Option<f64>,
    #[serde(default, rename = "maximumConfidence")]
    maximum_confidence: Option<f64>,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaseSetup {
    gate_mode: Option<String>,
    autopilot_mode: Option<String>,
    threshold: Option<f64>,
    #[serde(default)]
    learned_decisions: Vec<EvalDecisionRule>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalDecisionRule {
    #[serde(default = "one_copy")]
    copies: usize,
    #[serde(default = "seed_status")]
    status: String,
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

fn one_copy() -> usize {
    1
}

fn seed_status() -> String {
    "seed".into()
}

pub fn run(args: EvalArgs) -> Result<()> {
    let _requested_root = vault::resolve_vault(args.vault);
    let (_baseline_temp, baseline_root) = prepare_baseline_vault()?;
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
    let mut wrong_metadata = 0usize;
    let mut wrong_prediction = 0usize;
    let mut outcome_mismatches = Vec::new();
    let mut choice_mismatches = Vec::new();
    let mut rule_mismatches = Vec::new();
    let mut metadata_mismatches = Vec::new();
    let mut prediction_mismatches = Vec::new();
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
            .unwrap_or(baseline_root.as_path());
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
            outcome_mismatches.push(format!(
                "{}: outcome expected {:?}, got {:?}",
                case.id, case.expected_outcome, res.outcome
            ));
            match res.outcome.as_str() {
                "proceed" => false_proceed += 1,
                "ask_user" => false_ask += 1,
                "block" => false_block += 1,
                _ => {}
            }
        }
        match &case.expected_choice {
            ExpectedChoice::Selection(expected)
                if res.selected_option.as_ref() != Some(expected) =>
            {
                wrong_choice += 1;
                choice_mismatches.push(format!(
                    "{}: selectedOption expected {:?}, got {:?}",
                    case.id, expected, res.selected_option
                ));
            }
            ExpectedChoice::NoSelection if res.selected_option.is_some() => {
                wrong_choice += 1;
                choice_mismatches.push(format!(
                    "{}: selectedOption expected null, got {:?}",
                    case.id, res.selected_option
                ));
            }
            _ => {}
        }
        if let Some(expected) = &case.expected_predicted_outcome
            && &res.predicted_outcome != expected
        {
            wrong_prediction += 1;
            prediction_mismatches.push(format!(
                "{}: predictedOutcome expected {:?}, got {:?}",
                case.id, expected, res.predicted_outcome
            ));
        }
        match &case.expected_predicted_choice {
            ExpectedChoice::Selection(expected)
                if res.predicted_selected_option.as_ref() != Some(expected) =>
            {
                wrong_prediction += 1;
                prediction_mismatches.push(format!(
                    "{}: predictedSelectedOption expected {:?}, got {:?}",
                    case.id, expected, res.predicted_selected_option
                ));
            }
            ExpectedChoice::NoSelection if res.predicted_selected_option.is_some() => {
                wrong_prediction += 1;
                prediction_mismatches.push(format!(
                    "{}: predictedSelectedOption expected null, got {:?}",
                    case.id, res.predicted_selected_option
                ));
            }
            _ => {}
        }
        if let Some(expected) = case.expected_learned_rule {
            let applied = res
                .matched_policies
                .iter()
                .any(|path| path.contains("60-decision-examples/"));
            if applied != expected {
                wrong_rule += 1;
                rule_mismatches.push(format!(
                    "{}: learned rule expected {expected}, got {applied}",
                    case.id
                ));
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
        if let Some(expected) = case.expected_candidate_collision
            && res.candidate_collision != expected
        {
            wrong_metadata += 1;
            metadata_mismatches.push(format!(
                "{}: candidateCollision expected {expected}, got {}",
                case.id, res.candidate_collision
            ));
        }
        if let Some(expected) = &case.expected_match_kind
            && res.match_kind.as_ref() != Some(expected)
        {
            wrong_metadata += 1;
            metadata_mismatches.push(format!(
                "{}: matchKind expected {expected:?}, got {:?}",
                case.id, res.match_kind
            ));
        }
        if let Some(minimum) = case.minimum_match_margin
            && res.match_margin.is_none_or(|margin| margin < minimum)
        {
            wrong_metadata += 1;
            metadata_mismatches.push(format!(
                "{}: matchMargin expected >= {minimum}, got {:?}",
                case.id, res.match_margin
            ));
        }
        if let Some(maximum) = case.maximum_match_margin
            && res.match_margin.is_none_or(|margin| margin > maximum)
        {
            wrong_metadata += 1;
            metadata_mismatches.push(format!(
                "{}: matchMargin expected <= {maximum}, got {:?}",
                case.id, res.match_margin
            ));
        }
        if let Some(maximum) = case.maximum_confidence
            && res.confidence > maximum
        {
            wrong_metadata += 1;
            metadata_mismatches.push(format!(
                "{}: confidence expected <= {maximum}, got {}",
                case.id, res.confidence
            ));
        }
    }
    let report = json!({
        "cases": cases.len(),
        "falseProceed": false_proceed,
        "falseAsk": false_ask,
        "falseBlock": false_block,
        "wrongChoice": wrong_choice,
        "wrongRule": wrong_rule,
        "wrongMetadata": wrong_metadata,
        "wrongPrediction": wrong_prediction,
        "outcomeMismatches": outcome_mismatches,
        "choiceMismatches": choice_mismatches,
        "ruleMismatches": rule_mismatches,
        "metadataMismatches": metadata_mismatches,
        "predictionMismatches": prediction_mismatches,
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
    let failures = false_proceed
        + false_ask
        + false_block
        + wrong_choice
        + wrong_rule
        + wrong_metadata
        + wrong_prediction;
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
                    expected_choice: ExpectedChoice::NoSelection,
                    expected_predicted_outcome: None,
                    expected_predicted_choice: ExpectedChoice::Unspecified,
                    must_ask_user: Some(true),
                    expected_learned_rule: Some(false),
                    expected_candidate_collision: None,
                    expected_match_kind: None,
                    minimum_match_margin: None,
                    maximum_match_margin: None,
                    maximum_confidence: None,
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

fn prepare_baseline_vault() -> Result<(tempfile::TempDir, PathBuf)> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("BrainMap");
    vault::init_vault_quiet(Some(root.clone()), true)?;
    util::write_atomic(
        &root.join(".brainmap/autopilot.json"),
        serde_json::to_vec_pretty(&json!({
            "mode": "conservative",
            "level": "conservative",
            "threshold": 0.82
        }))?
        .as_slice(),
    )?;
    util::write_atomic(&root.join(".brainmap/gate-mode"), b"active")?;
    index::rebuild(&root)?;
    Ok((temp, root))
}

fn prepare_case_vault(case: &Case, setup: &CaseSetup) -> Result<(tempfile::TempDir, PathBuf)> {
    util::validate_safe_component("eval case id", &case.id)?;
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("BrainMap");
    fs::create_dir_all(root.join(".brainmap"))?;
    fs::create_dir_all(root.join("60-decision-examples"))?;
    let mut compiled_index = 0usize;
    for rule in &setup.learned_decisions {
        if !(1..=256).contains(&rule.copies) {
            bail!("eval learned decision copies must be between 1 and 256");
        }
        for _ in 0..rule.copies {
            let id = format!("eval-{}-{compiled_index}", case.id);
            compiled_index += 1;
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
            let frontmatter =
                markdown::frontmatter(&id, &rule.classification, "ask-before-action", "personal")
                    .replacen("status: seed", &format!("status: {}", rule.status), 1);
            let body = format!(
                "{}# Eval rule {}\n\n## Deterministic Rule\n\n{}\n",
                frontmatter, case.id, marker
            );
            util::write_atomic(
                &root.join("60-decision-examples").join(format!("{id}.md")),
                body.as_bytes(),
            )?;
        }
    }
    let autopilot_mode = setup.autopilot_mode.as_deref().unwrap_or("conservative");
    let autopilot_level = match autopilot_mode {
        "disabled" => "off",
        "balanced" => "balanced",
        _ => "conservative",
    };
    util::write_atomic(
        &root.join(".brainmap/autopilot.json"),
        serde_json::to_vec_pretty(&json!({
            "mode": autopilot_mode,
            "level": autopilot_level,
            "threshold": setup.threshold.unwrap_or(0.82)
        }))?
        .as_slice(),
    )?;
    util::write_atomic(
        &root.join(".brainmap/gate-mode"),
        setup.gate_mode.as_deref().unwrap_or("active").as_bytes(),
    )?;
    index::rebuild(&root)?;
    Ok((temp, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_case(expected_choice: &str) -> Case {
        serde_json::from_str(&format!(
            r#"{{"id":"choice-expectation","situation":"Choose safely","options":["A","B"],"risk":"low","reversible":true,"expectedOutcome":"ask_user","mustAskUser":true{expected_choice}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn expected_choice_distinguishes_omitted_null_and_selected_values() {
        assert_eq!(parse_case("").expected_choice, ExpectedChoice::Unspecified);
        assert_eq!(
            parse_case(r#", "expectedChoice":null"#).expected_choice,
            ExpectedChoice::NoSelection
        );
        assert_eq!(
            parse_case(r#", "expectedChoice":"A""#).expected_choice,
            ExpectedChoice::Selection("A".into())
        );
    }

    #[test]
    fn prepared_eval_case_defaults_to_active_enforcement() {
        let case = parse_case("");
        let setup = CaseSetup {
            gate_mode: None,
            autopilot_mode: None,
            threshold: None,
            learned_decisions: Vec::new(),
        };

        let (_temp, root) = prepare_case_vault(&case, &setup).unwrap();

        assert_eq!(crate::learning::gate_mode_config(&root), "active");
        assert_eq!(
            crate::learning::autopilot_config(&root).mode,
            "conservative"
        );
    }

    #[test]
    fn baseline_eval_vault_runs_seed_policies_in_active_mode() {
        let (_temp, root) = prepare_baseline_vault().unwrap();

        let result = gate::evaluate(
            &root,
            gate::GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose v1 storage".into(),
                options: vec!["Markdown+JSONL".into(), "SQLite".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(result.outcome, "proceed");
        assert_eq!(result.selected_option.as_deref(), Some("Markdown+JSONL"));
        assert_eq!(result.gate_mode, "active");
        assert_eq!(result.autopilot_mode, "conservative");
    }
}
