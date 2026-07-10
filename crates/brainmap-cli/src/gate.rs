use crate::cli::{DecideArgs, GateArgs, ShouldAskArgs};
use crate::decision_engine::DecisionEngine;
use crate::vault;
use anyhow::Result;
use std::path::Path;

pub use crate::decision_engine::{DecisionRequest as GateInput, DecisionResult as GateResponse};

pub fn cmd_gate(args: GateArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let input = GateInput {
        intent: args.intent,
        situation: args.situation,
        options: split_options(&args.options),
        proposed_action: args.proposed_action,
        risk: args.risk,
        reversible: args.reversible,
        decision_type: args.decision_type,
        scope: crate::util::resolve_learning_scope(&args.scope),
        agent_confidence: args.agent_confidence,
        dry_run: args.dry_run,
    };
    let response = evaluate(&root, input)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_human(&response);
    }
    Ok(())
}

pub fn cmd_should_ask(args: ShouldAskArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let situation = if args.situation.is_empty() {
        args.question.clone()
    } else {
        args.situation.clone()
    };
    let response = evaluate(
        &root,
        GateInput {
            intent: "would-ask-user".into(),
            situation,
            options: Vec::new(),
            proposed_action: args.question,
            risk: "medium".into(),
            reversible: Some(true),
            decision_type: "general".into(),
            scope: crate::util::default_project_scope(),
            agent_confidence: None,
            dry_run: false,
        },
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else if response.outcome == "proceed" || response.outcome == "no_action" {
        println!("no");
    } else {
        println!(
            "yes: {}",
            response
                .ask_user_question
                .unwrap_or(response.recommendation)
        );
    }
    Ok(())
}

pub fn cmd_decide(args: DecideArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let response = evaluate(
        &root,
        GateInput {
            intent: "plan".into(),
            situation: args.situation.unwrap_or_default(),
            options: split_options(&args.options),
            proposed_action: String::new(),
            risk: args.risk,
            reversible: args.reversible,
            decision_type: "general".into(),
            scope: crate::util::default_project_scope(),
            agent_confidence: None,
            dry_run: false,
        },
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&response)?);
    } else {
        print_human(&response);
    }
    Ok(())
}

fn print_human(response: &GateResponse) {
    println!("outcome: {}", response.outcome);
    if let Some(selected) = &response.selected_option {
        println!("selected: {selected}");
    }
    println!("confidence: {:.2}", response.confidence);
    println!("recommendation: {}", response.recommendation);
    if let Some(question) = &response.ask_user_question {
        println!("ask: {question}");
    }
}

pub fn evaluate(root: &Path, input: GateInput) -> Result<GateResponse> {
    DecisionEngine::new(root).evaluate(input)
}

fn split_options(options: &str) -> Vec<String> {
    options
        .split('|')
        .map(str::trim)
        .filter(|option| !option.is_empty())
        .map(str::to_string)
        .collect()
}
