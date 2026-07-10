use crate::cli::{LearnDecisionArgs, OnboardArgs};
use crate::{learning, markdown, privacy, vault};
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
    options: Vec<String>,
    chosen: String,
    #[serde(default)]
    rejected: Vec<String>,
    #[serde(default)]
    rationale: Option<String>,
}

pub fn run(args: OnboardArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let (answers, interactive) = if let Some(path) = args.answers {
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        let answers: OnboardingAnswers =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        (answers, false)
    } else {
        (interactive_answers()?, true)
    };
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

    for decision in &answers.decisions {
        learning::learn_decision(LearnDecisionArgs {
            situation: decision.situation.clone(),
            options: decision.options.join("|"),
            chosen: decision.chosen.clone(),
            rejected: (!decision.rejected.is_empty()).then(|| decision.rejected.join("|")),
            rationale: decision.rationale.clone(),
            decision_type: decision.decision_type.clone(),
            scope: decision.scope.clone(),
            vault: Some(root.clone()),
        })?;
    }
    learning::apply(crate::cli::ApplyArgs {
        pending: true,
        yes: true,
        dry_run: false,
        vault: Some(root),
    })?;
    println!("onboarding applied {} decision(s)", answers.decisions.len());
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
        let rule = markdown::DecisionRule {
            situation: decision.situation.clone(),
            decision_type: Some(decision.decision_type.clone()),
            scope: Some(decision.scope.clone()),
            options: decision.options.clone(),
            chosen: decision.chosen.clone(),
            rejected: decision.rejected.clone(),
        };
        markdown::decision_rule_marker(&rule)
            .with_context(|| format!("invalid onboarding decision {}", index + 1))?;
        let sensitive = format!(
            "{} {} {} {} {}",
            decision.situation,
            decision.options.join(" "),
            decision.chosen,
            decision.rejected.join(" "),
            decision.rationale.as_deref().unwrap_or_default()
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
        println!(
            "would learn {}: when {:?}, choose {:?}, type={}, scope={}",
            index + 1,
            decision.situation,
            decision.chosen,
            decision.decision_type,
            decision.scope
        );
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
        let chosen = prompt("Chosen option")?;
        let rejected = prompt("Rejected options, separated by | (optional)")?
            .split('|')
            .map(str::trim)
            .filter(|option| !option.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let decision_type = value_or(prompt("Decision type [general]")?, "general");
        let scope = value_or(prompt("Scope [global]")?, "global");
        let rationale = prompt("Rationale (optional)")?;
        decisions.push(OnboardingDecision {
            situation,
            decision_type,
            scope,
            options,
            chosen,
            rejected,
            rationale: (!rationale.is_empty()).then_some(rationale),
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
    "global".into()
}
