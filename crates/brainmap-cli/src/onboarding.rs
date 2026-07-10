use crate::cli::{LearnDecisionArgs, OnboardArgs};
use crate::{learning, markdown, privacy, util, vault};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};

const SCHEMA_VERSION: &str = "brainmap-onboarding-v1";

struct CalibrationTemplate {
    question: &'static str,
    situation: &'static str,
    decision_type: &'static str,
    options: &'static [&'static str],
}

const CALIBRATIONS: &[CalibrationTemplate] = &[
    CalibrationTemplate {
        question: "When this project declares a formatter, what should Brainmap prefer?",
        situation: "When a project declares a formatter, choose the formatter policy",
        decision_type: "tooling",
        options: &["follow project configuration", "ask user"],
    },
    CalibrationTemplate {
        question: "When this project declares a test command, what should Brainmap prefer?",
        situation: "When a project declares a test command, choose the test policy",
        decision_type: "workflow",
        options: &["follow project configuration", "ask user"],
    },
    CalibrationTemplate {
        question: "Without an existing preference, how should Brainmap handle a dependency change?",
        situation: "Choose how to handle a dependency change without an existing preference",
        decision_type: "dependencies",
        options: &["ask user", "make the smallest reversible change"],
    },
];

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

enum PreparedOnboardingDecision {
    Executable(Box<learning::PreparedDecisionUpdate>),
    Pending(serde_json::Value),
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
    let prepared = prepare_answers(&answers)?;
    print_preview(&prepared)?;

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
    for decision in &prepared {
        match decision {
            PreparedOnboardingDecision::Executable(update) => {
                update.write(&root)?;
                executable += 1;
            }
            PreparedOnboardingDecision::Pending(event) => {
                util::append_jsonl(&root.join("90-calibration/pending-onboarding.jsonl"), event)?;
                pending += 1;
            }
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
        let sensitive = format!(
            "{} {} {} {} {} {} {} {}",
            decision.situation,
            decision.decision_type,
            decision.scope,
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
        markdown::validate_executable_rule_metadata(
            "decision-example",
            "seed",
            &decision.decision_type,
            &decision.scope,
        )
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid onboarding decision {}", index + 1))?;
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
    }
    Ok(())
}

fn prepare_answers(answers: &OnboardingAnswers) -> Result<Vec<PreparedOnboardingDecision>> {
    answers
        .decisions
        .iter()
        .map(|decision| {
            if let Some(chosen) = &decision.chosen {
                let update = learning::prepare_decision_update(LearnDecisionArgs {
                    situation: decision.situation.clone(),
                    options: decision.options.join("|"),
                    chosen: chosen.clone(),
                    rejected: (!decision.rejected.is_empty()).then(|| decision.rejected.join("|")),
                    rationale: decision.rationale.clone(),
                    decision_type: decision.decision_type.clone(),
                    scope: decision.scope.clone(),
                    vault: None,
                })?
                .context("validated onboarding decision unexpectedly became secret")?;
                Ok(PreparedOnboardingDecision::Executable(Box::new(update)))
            } else {
                Ok(PreparedOnboardingDecision::Pending(serde_json::json!({
                    "id": util::id("onboarding", &decision.situation),
                    "createdAt": util::now_iso(),
                    "schemaVersion": SCHEMA_VERSION,
                    "status": "pending-clarification",
                    "situation": privacy::redact(&decision.situation),
                    "decisionType": privacy::redact(&decision.decision_type),
                    "scope": privacy::redact(&decision.scope),
                    "freeText": privacy::redact(decision.free_text.as_deref().unwrap_or_default())
                })))
            }
        })
        .collect()
}

fn print_preview(prepared: &[PreparedOnboardingDecision]) -> Result<()> {
    println!("onboarding schema: {SCHEMA_VERSION}");
    for decision in prepared {
        match decision {
            PreparedOnboardingDecision::Executable(update) => println!(
                "onboarding exact executable update preview: {}",
                serde_json::to_string(&update.preview_value())?
            ),
            PreparedOnboardingDecision::Pending(event) => println!(
                "onboarding exact pending preview: {}",
                serde_json::to_string(&serde_json::json!({
                    "path": "90-calibration/pending-onboarding.jsonl",
                    "event": event
                }))?
            ),
        }
    }
    Ok(())
}

fn interactive_answers() -> Result<OnboardingAnswers> {
    println!("Brainmap local onboarding. Answer three predefined calibration questions.");
    println!("Use a number or exact choice; any other answer stays pending clarification.");
    let mut decisions = Vec::new();
    for (index, template) in CALIBRATIONS.iter().enumerate() {
        println!(
            "Calibration {}/{}: {}",
            index + 1,
            CALIBRATIONS.len(),
            template.question
        );
        for (choice_index, choice) in template.options.iter().enumerate() {
            println!("  {}) {choice}", choice_index + 1);
        }
        let answer = prompt_required("Answer")?;
        let chosen = calibration_choice(template, &answer);
        let rejected = chosen
            .as_ref()
            .map(|selected| {
                template
                    .options
                    .iter()
                    .filter(|choice| !choice.eq_ignore_ascii_case(selected))
                    .map(|choice| (*choice).to_string())
                    .collect()
            })
            .unwrap_or_default();
        decisions.push(OnboardingDecision {
            situation: template.situation.into(),
            decision_type: template.decision_type.into(),
            scope: "project:auto".into(),
            options: template
                .options
                .iter()
                .map(|choice| (*choice).to_string())
                .collect(),
            chosen,
            rejected,
            rationale: None,
            free_text: calibration_choice(template, &answer)
                .is_none()
                .then_some(answer),
        });
    }
    println!("Optionally add more concrete recurring decisions.");
    loop {
        let situation = prompt("Additional situation (leave empty to finish)")?;
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

fn calibration_choice(template: &CalibrationTemplate, answer: &str) -> Option<String> {
    if let Ok(index) = answer.parse::<usize>()
        && (1..=template.options.len()).contains(&index)
    {
        return Some(template.options[index - 1].into());
    }
    template
        .options
        .iter()
        .find(|choice| choice.eq_ignore_ascii_case(answer))
        .map(|choice| (*choice).to_string())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut value = String::new();
    if io::stdin().read_line(&mut value)? == 0 {
        bail!("onboarding input ended before all required prompts were answered");
    }
    Ok(value.trim().to_string())
}

fn prompt_required(label: &str) -> Result<String> {
    loop {
        let value = prompt(label)?;
        if !value.is_empty() {
            return Ok(value);
        }
        println!("An answer is required for each calibration question.");
    }
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
