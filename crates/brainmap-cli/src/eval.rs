use crate::cli::EvalArgs;
use crate::{gate, index, markdown, util, vault};
use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
struct Case {
    id: String,
    situation: String,
    options: Vec<String>,
    risk: String,
    reversible: bool,
    #[serde(rename = "expectedOutcome")]
    expected_outcome: String,
    #[serde(rename = "expectedChoice")]
    expected_choice: Option<String>,
    #[serde(rename = "mustAskUser")]
    must_ask_user: Option<bool>,
    reason: Option<String>,
    setup: Option<CaseSetup>,
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

#[derive(Debug, Deserialize)]
struct EvalDecisionRule {
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
            cases.push(serde_json::from_str::<Case>(line)?);
        }
    }
    let mut false_proceed = 0usize;
    let mut false_ask = 0usize;
    let mut false_block = 0usize;
    let mut wrong_choice = 0usize;
    let mut expected_asks = 0usize;
    let mut ids = Vec::new();
    let mut reasons = Vec::new();
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
                intent: "would-ask-user".into(),
                situation: case.situation.clone(),
                options: case.options.clone(),
                proposed_action: String::new(),
                risk: case.risk.clone(),
                reversible: Some(case.reversible),
                decision_type: "general".into(),
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
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "cases": cases.len(),
            "falseProceed": false_proceed,
            "falseAsk": false_ask,
            "falseBlock": false_block,
            "wrongChoice": wrong_choice,
            "confidenceCalibration": "deterministic-mvp",
            "policyCoverage": "seed-policy",
            "ambiguityDetection": true,
            "expectedAskCases": expected_asks,
            "caseIds": ids,
            "reasons": reasons
        }))?
    );
    Ok(())
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
            situation: case.situation.clone(),
            options: case.options.clone(),
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
