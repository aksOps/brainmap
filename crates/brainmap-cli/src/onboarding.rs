use crate::cli::{LearnDecisionArgs, OnboardArgs};
use crate::{learning, markdown, privacy, util, vault};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};

const SCHEMA_VERSION: &str = "brainmap-onboarding-v1";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OnboardingAnswers {
    schema_version: String,
    decisions: Vec<OnboardingDecision>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OnboardingDecision {
    situation: String,
    #[serde(default = "default_decision_type")]
    decision_type: String,
    #[serde(default = "default_scope")]
    scope: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    chosen: Option<String>,
    #[serde(default)]
    rejected: Vec<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    free_text: Option<String>,
}

pub fn run(args: OnboardArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let (mut answers, interactive) = if let Some(path) = args.answers {
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let answers: OnboardingAnswers =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        (answers, false)
    } else {
        (interactive_answers()?, true)
    };
    for decision in &mut answers.decisions {
        decision.scope = util::resolve_learning_scope(&decision.scope);
    }
    validate_answers(&answers)?;
    print_preview(&answers);

    let approved = if args.dry_run {
        false
    } else if args.yes {
        true
    } else if interactive {
        prompt_confirmation()?
    } else {
        false
    };
    if !approved {
        println!("onboarding preview only; pass --yes to activate these decisions");
        return Ok(());
    }

    let mut executable = 0usize;
    let mut pending = 0usize;
    for decision in &answers.decisions {
        if let Some(chosen) = &decision.chosen {
            learning::learn_decision(LearnDecisionArgs {
                situation: decision.situation.clone(),
                options: decision.options.join("|"),
                chosen: chosen.clone(),
                rejected: (!decision.rejected.is_empty()).then(|| decision.rejected.join("|")),
                rationale: decision.rationale.clone(),
                decision_type: decision.decision_type.clone(),
                scope: decision.scope.clone(),
                vault: Some(root.clone()),
            })?;
            executable += 1;
        } else {
            util::append_jsonl(
                &root.join("90-calibration/pending-onboarding.jsonl"),
                &serde_json::json!({
                    "id": util::id("onboarding", &decision.situation),
                    "createdAt": util::now_iso(),
                    "schemaVersion": SCHEMA_VERSION,
                    "status": "pending-clarification",
                    "situation": privacy::redact(&decision.situation),
                    "decisionType": decision.decision_type,
                    "scope": decision.scope,
                    "freeText": privacy::redact(decision.free_text.as_deref().unwrap_or_default())
                }),
            )?;
            pending += 1;
        }
    }
    if executable > 0 {
        learning::apply(crate::cli::ApplyArgs {
            pending: true,
            yes: true,
            dry_run: false,
            vault: Some(root),
        })?;
    }
    println!(
        "onboarding applied {executable} decision(s); kept {pending} answer(s) pending clarification"
    );
    Ok(())
}

fn validate_answers(answers: &OnboardingAnswers) -> Result<()> {
    if answers.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported onboarding schema {}; expected {SCHEMA_VERSION}",
            answers.schema_version
        );
    }
    if answers.decisions.is_empty() {
        bail!("onboarding answers contain no decisions");
    }
    for (index, decision) in answers.decisions.iter().enumerate() {
        if let Some(chosen) = &decision.chosen {
            let rule = markdown::DecisionRule {
                situation: decision.situation.clone(),
                decision_type: Some(decision.decision_type.clone()),
                scope: Some(decision.scope.clone()),
                options: decision.options.clone(),
                chosen: chosen.clone(),
                rejected: decision.rejected.clone(),
            };
            markdown::decision_rule_marker(&rule)
                .with_context(|| format!("invalid onboarding decision {}", index + 1))?;
        } else if decision
            .free_text
            .as_deref()
            .is_none_or(|answer| answer.trim().is_empty())
        {
            bail!(
                "onboarding decision {} needs either chosen or freeText",
                index + 1
            );
        }
        let sensitive = format!(
            "{} {} {} {} {} {}",
            decision.situation,
            decision.options.join(" "),
            decision.chosen.as_deref().unwrap_or_default(),
            decision.rejected.join(" "),
            decision.rationale.as_deref().unwrap_or_default(),
            decision.free_text.as_deref().unwrap_or_default()
        );
        if privacy::contains_secret(&sensitive) {
            bail!(
                "onboarding decision {} contains secret-like material",
                index + 1
            );
        }
    }
    Ok(())
}

fn print_preview(answers: &OnboardingAnswers) {
    println!("onboarding schema: {}", answers.schema_version);
    for (index, decision) in answers.decisions.iter().enumerate() {
        if let Some(chosen) = &decision.chosen {
            println!(
                "would learn {}: when {:?}, choose {:?}, type={}, scope={}",
                index + 1,
                decision.situation,
                chosen,
                decision.decision_type,
                decision.scope
            );
        } else {
            println!(
                "would keep {} pending clarification: situation {:?}, type={}, scope={}",
                index + 1,
                decision.situation,
                decision.decision_type,
                decision.scope
            );
        }
    }
}

fn interactive_answers() -> Result<OnboardingAnswers> {
    println!("Brainmap local onboarding. Enter concrete recurring decisions.");
    println!("Leave the situation empty when finished.");
    let mut decisions = Vec::new();
    loop {
        let situation = prompt("Situation")?;
        if situation.is_empty() {
            break;
        }
        let options = prompt("Options, separated by |")?
            .split('|')
            .map(str::trim)
            .filter(|option| !option.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let chosen = prompt("Chosen option (leave empty for pending free text)")?;
        let rejected = prompt("Rejected options, separated by | (optional)")?
            .split('|')
            .map(str::trim)
            .filter(|option| !option.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let decision_type = value_or(prompt("Decision type [general]")?, "general");
        let scope = value_or(prompt("Scope [project:auto]")?, "project:auto");
        let rationale = prompt("Rationale or free-text decision")?;
        decisions.push(OnboardingDecision {
            situation,
            decision_type,
            scope,
            options,
            chosen: (!chosen.is_empty()).then_some(chosen),
            rejected,
            rationale: (!rationale.is_empty()).then_some(rationale.clone()),
            free_text: (!rationale.is_empty()).then_some(rationale),
        });
    }
    Ok(OnboardingAnswers {
        schema_version: SCHEMA_VERSION.into(),
        decisions,
    })
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(value.trim().to_string())
}

fn prompt_confirmation() -> Result<bool> {
    Ok(prompt("Apply these decisions? [y/N]")?.eq_ignore_ascii_case("y"))
}

fn value_or(value: String, default: &str) -> String {
    if value.is_empty() {
        default.into()
    } else {
        value
    }
}

fn default_decision_type() -> String {
    "general".into()
}

fn default_scope() -> String {
    "project:auto".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{gate, index};

    #[test]
    fn ambiguous_free_text_stays_pending_and_does_not_change_gate_behavior() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();
        let answers = tmp.path().join("answers.json");
        fs::write(
            &answers,
            r#"{
  "schemaVersion": "brainmap-onboarding-v1",
  "decisions": [{
    "situation": "Choose deployment rhythm for an experimental project",
    "decisionType": "workflow",
    "scope": "project:experimental",
    "freeText": "It depends on how risky the current change feels"
  }]
}"#,
        )
        .unwrap();

        run(OnboardArgs {
            vault: Some(root.clone()),
            answers: Some(answers),
            dry_run: false,
            yes: true,
        })
        .unwrap();

        let pending =
            fs::read_to_string(root.join("90-calibration/pending-onboarding.jsonl")).unwrap();
        assert!(pending.contains("pending-clarification"));
        let result = gate::evaluate(
            &root,
            gate::GateInput {
                intent: "plan".into(),
                situation: "Choose deployment rhythm for an experimental project".into(),
                options: vec!["daily".into(), "weekly".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "project:experimental".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(result.outcome, "ask_user");
        assert_eq!(result.selected_option, None);
        assert!(result.rule_id.is_none());
    }
}
