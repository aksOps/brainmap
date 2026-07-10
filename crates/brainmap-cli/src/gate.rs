use crate::cli::{DecideArgs, GateArgs, ShouldAskArgs};
use crate::decision_engine::DecisionEngine;
use crate::{index, learning, privacy, util, vault};
use anyhow::Result;
use serde_json::json;
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
        scope: args.scope,
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
            scope: "global".into(),
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
            scope: "global".into(),
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
    if let Some(q) = &response.ask_user_question {
        println!("ask: {q}");
    }
}

pub fn evaluate(root: &Path, input: GateInput) -> Result<GateResponse> {
    DecisionEngine::new(root).evaluate(input)
}

pub(crate) fn evaluate_internal(root: &Path, input: GateInput) -> Result<GateResponse> {
    let combined = privacy::redact(&input.combined());
    let lower = combined.to_lowercase();
    let decision_id = util::id("dec", &combined);
    let autopilot = learning::autopilot_config(root);
    let configured_gate_mode = learning::gate_mode_config(root);
    let threshold = autopilot.threshold;
    let learned_resolution = index::resolve_decision_rule(
        root,
        &input.situation,
        &input.decision_type,
        &input.scope,
        &input.options,
    )?;
    let mut matched = index::policy_paths_for(root, &["local", "privacy", "approval", "question"])
        .unwrap_or_default();
    matched.truncate(6);
    let mut restrictions = Vec::new();
    let mut outcome = "ask_user".to_string();
    let mut recommendation = "Ask a focused question before proceeding.".to_string();
    let mut selected = None;
    let mut confidence = input.agent_confidence.unwrap_or(0.56).clamp(0.0, 1.0);
    let mut rule_id = None;
    let mut rule_scope = None;
    let mut match_score = None;
    let mut match_kind = None;
    let mut risk_tier = "ask_before_action".to_string();
    let mut summary = vec!["Applied deterministic policy precedence.".to_string()];
    let mut learned_question = None;
    let reversible = input.reversible.unwrap_or(false);
    let options = input.options.clone();

    if combined.trim().is_empty()
        || (input.situation.trim().is_empty() && input.proposed_action.trim().is_empty())
    {
        outcome = "needs_more_context".into();
        recommendation = "Provide situation, proposed action, or options.".into();
        confidence = 0.2;
        summary.push("Situation was under-specified.".into());
    } else if privacy::contains_secret(&input.combined()) {
        outcome = "block".into();
        recommendation = "Blocked because the request contains secret-like material.".into();
        confidence = 0.99;
        risk_tier = "never_auto".into();
        restrictions.push("[[40-restrictions/hard-no-rules.md]]".into());
        summary.push("Secrets and safety rules outrank all policies.".into());
    } else if hard_no(&lower) {
        outcome = "block".into();
        recommendation = "Blocked by hard-no or never-auto policy.".into();
        confidence = 0.96;
        risk_tier = "never_auto".into();
        restrictions.push("[[40-restrictions/never-auto.md]]".into());
        summary.push("Request matches never-auto safety class.".into());
    } else if privacy_boundary(&lower) {
        outcome = "ask_user".into();
        recommendation =
            "Ask for explicit approval before crossing privacy or remote-use boundary.".into();
        confidence = 0.9;
        risk_tier = "approval_required".into();
        restrictions.push("[[40-restrictions/privacy-boundaries.md]]".into());
        summary.push("Privacy boundary may apply.".into());
    } else if input.risk.eq_ignore_ascii_case("critical")
        || (input.risk.eq_ignore_ascii_case("high") && !reversible)
    {
        outcome = "ask_user".into();
        recommendation = "Ask before high-risk or irreversible action.".into();
        confidence = 0.86;
        risk_tier = "approval_required".into();
        restrictions.push("[[40-restrictions/approval-required.md]]".into());
        summary.push("Irreversible/high-risk action requires approval.".into());
    } else if let index::DecisionRuleResolution::Applicable(rule) = &learned_resolution {
        let rule_link = format!("[[{}]]", rule.path);
        if !matched.contains(&rule_link) {
            matched.insert(0, rule_link);
        }
        selected = choose_learned_option(&options, &rule.chosen, &rule.rejected);
        confidence = rule.calibrated_confidence();
        rule_id = Some(rule.rule_id.clone());
        rule_scope = Some(rule.scope.clone());
        match_score = Some(rule.score);
        match_kind = Some(rule.match_kind.into());
        if learned_rule_requires_ask(&rule.chosen) {
            outcome = "ask_user".into();
            recommendation = format!("Follow the learned preference to {}.", rule.chosen);
            risk_tier = "ask_before_action".into();
            learned_question = Some(format!(
                "Should I follow your learned preference to {}?",
                rule.chosen
            ));
        } else if let Some(choice) = &selected {
            outcome = "proceed".into();
            recommendation = format!("Proceed with {choice}; it matches a learned decision.");
            risk_tier = "reversible_auto".into();
        } else {
            outcome = "ask_user".into();
            recommendation = format!(
                "The learned choice '{}' is not among the current options.",
                rule.chosen
            );
            learned_question = Some(format!(
                "Use the learned choice '{}', or select one of the current options?",
                rule.chosen
            ));
        }
        summary.push(format!(
            "Applied compiled learned decision rule with score {:.2}.",
            rule.score
        ));
    } else if let index::DecisionRuleResolution::OptionMismatch(rule) = &learned_resolution {
        let rule_link = format!("[[{}]]", rule.path);
        if !matched.contains(&rule_link) {
            matched.insert(0, rule_link);
        }
        confidence = rule.calibrated_confidence();
        rule_id = Some(rule.rule_id.clone());
        rule_scope = Some(rule.scope.clone());
        match_score = Some(rule.score);
        match_kind = Some(rule.match_kind.into());
        outcome = "ask_user".into();
        recommendation = format!(
            "The learned choice '{}' is not among the current options.",
            rule.chosen
        );
        learned_question = Some(format!(
            "The available options changed. Which current option should replace '{}'?",
            rule.chosen
        ));
        summary.push(format!(
            "An applicable {} learned rule scored {:.2}, but its choice is unavailable.",
            rule.match_kind, rule.score
        ));
    } else if let index::DecisionRuleResolution::Ambiguous { best, alternative } =
        &learned_resolution
    {
        confidence = best.calibrated_confidence().min(0.65);
        rule_scope = Some(best.scope.clone());
        match_score = Some(best.score);
        match_kind = Some("ambiguous".into());
        outcome = "ask_user".into();
        recommendation = "Conflicting learned decisions are equally relevant.".into();
        learned_question = Some(format!(
            "Should I choose '{}' or '{}' for this situation?",
            best.chosen, alternative.chosen
        ));
        summary.push(format!(
            "Learned rule conflict: {} and {} both scored {:.2}.",
            best.rule_id, alternative.rule_id, best.score
        ));
    } else if ambiguous(&lower, options.len()) {
        outcome = "ask_user".into();
        recommendation = "Ask one narrower question; current context is ambiguous.".into();
        confidence = 0.58;
        summary.push("Ambiguity detected.".into());
    } else if local_first_storage(&lower, &options) {
        selected = choose_local_first(&options);
        outcome = "proceed".into();
        confidence = 0.9;
        risk_tier = "reversible_auto".into();
        recommendation = selected
            .as_ref()
            .map(|s| format!("Proceed with {s}; it matches local-first v1 policy."))
            .unwrap_or_else(|| "Proceed with local Markdown/JSONL plus embedded SQLite.".into());
        summary.push("Local-first v1 storage policy matched.".into());
    } else if input.intent == "would-ask-user" && lower.contains("redundant") {
        outcome = "no_action".into();
        recommendation = "Suppress redundant question.".into();
        confidence = 0.84;
        risk_tier = "suggest_only".into();
        summary.push("Question appears redundant.".into());
    } else if input.risk.eq_ignore_ascii_case("low") && reversible {
        outcome = "ask_user".into();
        recommendation =
            "No learned or executable policy selects among the current options.".into();
        summary.push(
            "Option order is non-authoritative; the confidence threshold cannot select a default."
                .into(),
        );
    } else if confidence >= threshold && reversible {
        outcome = "ask_user".into();
        recommendation =
            "Confidence alone cannot select an option without an applicable policy.".into();
        summary.push(
            "Confidence exceeded threshold, but no executable policy selected an option.".into(),
        );
    }

    if outcome == "proceed" && confidence < threshold {
        outcome = "ask_user".into();
        recommendation = format!(
            "Ask before proceeding; confidence {:.2} is below the configured {:.2} threshold.",
            confidence, threshold
        );
        risk_tier = "ask_before_action".into();
        summary.push("Configured confidence threshold forced ask_user.".into());
    }

    if (std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() == Some("1")
        || matches!(
            std::env::var("BRAINMAP_GATE_MODE").ok().as_deref(),
            Some("ask-always" | "suggest-only")
        )
        || autopilot.mode == "disabled"
        || matches!(configured_gate_mode.as_str(), "ask-always" | "suggest-only"))
        && outcome == "proceed"
    {
        outcome = "ask_user".into();
        recommendation = "Autopilot/gate enforcement disabled; ask user.".into();
        risk_tier = "ask_before_action".into();
        summary.push("Kill switch forced ask_user.".into());
    }

    let rejected = options
        .iter()
        .filter(|o| Some(*o) != selected.as_ref())
        .cloned()
        .collect::<Vec<_>>();
    let question = match outcome.as_str() {
        "ask_user" => {
            learned_question.or_else(|| Some(focused_question(&input, selected.as_deref())))
        }
        "needs_more_context" => {
            Some("What situation, options, and risk should I use for this decision?".into())
        }
        _ => None,
    };
    let default_if_no_answer = if outcome == "ask_user" {
        Some("defer or take the cheapest reversible step".into())
    } else {
        None
    };
    let response = GateResponse {
        decision_id: decision_id.clone(),
        outcome: outcome.clone(),
        recommendation,
        selected_option: selected,
        rejected_options: rejected,
        confidence,
        rule_id,
        rule_scope,
        match_score,
        match_kind,
        risk_tier,
        reasoning_summary: summary,
        matched_policies: matched,
        restrictions_applied: restrictions,
        ask_user_question: question,
        default_if_no_answer,
        learning_event: json!({
            "shouldRecord": !input.dry_run,
            "kind": "decision-gate",
            "situation": privacy::redact(&input.situation),
            "chosen": null,
            "confidence": confidence
        }),
    };
    if !input.dry_run {
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        util::append_jsonl(
            &ledger,
            &json!({
                "id": decision_id,
                "createdAt": util::now_iso(),
                "kind": "decision-gate",
                "outcome": outcome,
                "confidence": confidence,
                "situation": privacy::redact(&input.situation)
            }),
        )?;
    }
    Ok(response)
}

fn split_options(options: &str) -> Vec<String> {
    options
        .split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn hard_no(lower: &str) -> bool {
    let destructive =
        lower.contains("delete") || lower.contains("rm -rf") || lower.contains("destroy");
    let disable_privacy = lower.contains("disable privacy") || lower.contains("ignore privacy");
    let money =
        lower.contains("spend money") || lower.contains("payment") || lower.contains("credit card");
    let remote_private = lower.contains("private memory")
        && (lower.contains("remote") || lower.contains("external"));
    destructive && lower.contains("irreversible") || disable_privacy || money || remote_private
}

fn privacy_boundary(lower: &str) -> bool {
    let remote =
        lower.contains("remote") || lower.contains("external") || lower.contains("model-call");
    let sensitive = lower.contains("private")
        || lower.contains("memory")
        || lower.contains("credential")
        || lower.contains("secret")
        || lower.contains("personal data");
    remote && sensitive
}

fn ambiguous(lower: &str, option_count: usize) -> bool {
    lower.contains("ambiguous")
        || lower.contains("unclear")
        || lower.contains("not sure")
        || (option_count == 0 && lower.len() < 24)
}

fn local_first_storage(lower: &str, options: &[String]) -> bool {
    let has_storage_context = lower.contains("storage")
        || lower.contains("v1")
        || lower.contains("local")
        || lower.contains("personal")
        || lower.contains("brainmap");
    let has_good_option = options.iter().any(|o| {
        let o = o.to_lowercase();
        o.contains("markdown") || o.contains("jsonl") || o.contains("sqlite")
    });
    let has_bad_option = options.iter().any(|o| {
        let o = o.to_lowercase();
        o.contains("external") || o.contains("cloud") || o.contains("vector db")
    });
    has_storage_context && has_good_option && has_bad_option
}

fn choose_local_first(options: &[String]) -> Option<String> {
    options
        .iter()
        .find(|o| {
            let o = o.to_lowercase();
            o.contains("markdown") || o.contains("jsonl")
        })
        .or_else(|| options.iter().find(|o| o.to_lowercase().contains("sqlite")))
        .cloned()
}

fn choose_learned_option(options: &[String], chosen: &str, rejected: &[String]) -> Option<String> {
    let chosen_lower = chosen.to_lowercase();
    options
        .iter()
        .filter(|option| {
            !rejected
                .iter()
                .any(|rejected| option.eq_ignore_ascii_case(rejected))
        })
        .find(|option| {
            let option_lower = option.to_lowercase();
            option_lower == chosen_lower
                || chosen_lower.contains(&option_lower)
                || option_lower.contains(&chosen_lower)
        })
        .cloned()
}

fn learned_rule_requires_ask(chosen: &str) -> bool {
    let lower = chosen.to_lowercase();
    lower.contains("ask user")
        || lower.contains("ask me")
        || lower.contains("ask before")
        || lower.contains("approval")
}

fn focused_question(input: &GateInput, selected: Option<&str>) -> String {
    if input.risk.eq_ignore_ascii_case("high") || input.risk.eq_ignore_ascii_case("critical") {
        return "Do you explicitly approve this high-risk or irreversible action?".into();
    }
    if let Some(selected) = selected {
        format!("Proceed with {selected}, or choose another option?")
    } else if input.options.is_empty() {
        "What option should I compare against the default reversible path?".into()
    } else {
        format!(
            "Which option should be chosen: {}?",
            input.options.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();
        (tmp, root)
    }

    #[test]
    fn low_risk_storage_proceeds() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose v1 storage".into(),
                options: vec![
                    "Markdown+JSONL".into(),
                    "SQLite".into(),
                    "External Vector DB".into(),
                ],
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
        assert_eq!(res.outcome, "proceed");
        assert_eq!(res.selected_option.as_deref(), Some("Markdown+JSONL"));
    }

    #[test]
    fn unlearned_low_risk_decision_does_not_select_the_first_option() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Choose a package manager for a new JavaScript project".into(),
                options: vec!["npm".into(), "pnpm".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:example".into(),
                agent_confidence: Some(0.9),
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option, None);
    }

    #[test]
    fn secrets_block() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "privacy".into(),
                situation: "store api_key=abcdef1234567890".into(),
                options: vec![],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "privacy".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "block");
    }

    #[test]
    fn ambiguous_asks() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "unknown".into(),
                situation: "unclear thing".into(),
                options: vec![],
                proposed_action: String::new(),
                risk: "medium".into(),
                reversible: Some(false),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "ask_user");
        assert!(!res.ask_user_question.unwrap().contains("Brainmap"));
    }

    #[test]
    fn empty_options_question_hides_policy_layer() {
        let (_tmp, root) = temp_vault();
        let res = evaluate(
            &root,
            GateInput {
                intent: "would-ask-user".into(),
                situation: "Need choose publishing flow for finished work".into(),
                options: vec![],
                proposed_action: String::new(),
                risk: "medium".into(),
                reversible: Some(false),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "ask_user");
        assert!(!res.ask_user_question.unwrap().contains("Brainmap"));
    }

    #[test]
    fn learned_ask_decision_overrides_reversible_default() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "renaming local temporary notes".into(),
            options: "rename automatically|ask user".into(),
            chosen: "ask user".into(),
            rejected: Some("rename automatically".into()),
            rationale: Some("explicit user preference".into()),
            decision_type: "workflow".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "renaming local temporary notes".into(),
                options: vec!["rename automatically".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option.as_deref(), Some("ask user"));
    }

    #[test]
    fn learned_rule_does_not_leak_to_a_nearby_decision() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("repository-specific formatting rules".into()),
            decision_type: "general".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose a database for a Rust repository".into(),
                options: vec!["SQLite".into(), "PostgreSQL".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert!(
            res.matched_policies
                .iter()
                .all(|policy| !policy.contains("60-decision-examples")),
            "an unrelated learned formatter rule was reported as applied: {res:#?}"
        );
    }

    #[test]
    fn learned_rule_applies_to_a_supported_paraphrase() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("repository-specific formatting rules".into()),
            decision_type: "general".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "What formatting tool should this Rust codebase use?".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "general".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "proceed");
        assert_eq!(res.selected_option.as_deref(), Some("a custom formatter"));
    }

    #[test]
    fn fuzzy_learned_result_exposes_match_evidence_and_calibrated_confidence() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("project formatting rules".into()),
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose primary formatter for a Rust repository".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:alpha".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.match_kind.as_deref(), Some("fuzzy"));
        assert_eq!(res.match_score, Some(0.8));
        assert_eq!(res.rule_scope.as_deref(), Some("project:alpha"));
        assert!(res.rule_id.is_some());
        assert!(res.confidence < 0.9, "{res:#?}");
    }

    #[test]
    fn equally_relevant_conflicting_rules_return_ambiguity() {
        let (_tmp, root) = temp_vault();
        for (situation, chosen, rejected) in [
            (
                "Choose primary formatter for a Rust repository",
                "rustfmt",
                "a custom formatter",
            ),
            (
                "Choose preferred formatter for a Rust repository",
                "a custom formatter",
                "rustfmt",
            ),
        ] {
            crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
                situation: situation.into(),
                options: "rustfmt|a custom formatter".into(),
                chosen: chosen.into(),
                rejected: Some(rejected.into()),
                rationale: Some("conflicting learned evidence".into()),
                decision_type: "tooling".into(),
                scope: "project:alpha".into(),
                vault: Some(root.clone()),
            })
            .unwrap();
        }
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose formatter for a Rust repository".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:alpha".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert_eq!(res.selected_option, None);
        assert_eq!(res.match_kind.as_deref(), Some("ambiguous"));
        assert!(
            res.reasoning_summary
                .iter()
                .any(|line| line.contains("conflict"))
        );
    }

    #[test]
    fn project_scoped_rule_does_not_leak_to_another_project() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("project alpha formatting rules".into()),
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "Choose formatter for a Rust repository".into(),
                options: vec!["rustfmt".into(), "a custom formatter".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "tooling".into(),
                scope: "project:beta".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert!(
            res.matched_policies
                .iter()
                .all(|policy| !policy.contains("60-decision-examples")),
            "a project-scoped rule leaked across projects: {res:#?}"
        );
    }

    #[test]
    fn hundred_nearby_decisions_do_not_apply_a_formatter_rule() {
        let (_tmp, root) = temp_vault();
        crate::learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose formatter for a Rust repository".into(),
            options: "rustfmt|a custom formatter".into(),
            chosen: "a custom formatter".into(),
            rejected: Some("rustfmt".into()),
            rationale: Some("project formatting rules".into()),
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let operations = [
            "database",
            "logging library",
            "package manager",
            "test runner",
            "build system",
            "linter",
            "deployment target",
            "cache",
            "message queue",
            "serializer",
        ];
        let contexts = [
            "Rust repository",
            "Python repository",
            "JavaScript repository",
            "Go repository",
            "Java repository",
            "mobile application",
            "web application",
            "command line project",
            "backend project",
            "frontend project",
        ];

        let mut evaluated = 0usize;
        for operation in operations {
            for context in contexts {
                let res = evaluate(
                    &root,
                    GateInput {
                        intent: "plan".into(),
                        situation: format!("Choose a {operation} for a {context}"),
                        options: vec!["option A".into(), "option B".into()],
                        proposed_action: String::new(),
                        risk: "low".into(),
                        reversible: Some(true),
                        decision_type: "tooling".into(),
                        scope: "project:alpha".into(),
                        agent_confidence: None,
                        dry_run: true,
                    },
                )
                .unwrap();
                assert!(
                    res.rule_id.is_none(),
                    "formatter rule leaked into {operation}/{context}: {res:#?}"
                );
                assert_eq!(res.outcome, "ask_user");
                assert_eq!(res.selected_option, None);
                evaluated += 1;
            }
        }
        assert_eq!(evaluated, 100);
    }

    #[test]
    fn corrected_feedback_compiles_explicit_choice_and_rejection() {
        let (_tmp, root) = temp_vault();
        let input = || GateInput {
            intent: "plan".into(),
            situation: "renaming local temporary notes".into(),
            options: vec!["rename automatically".into(), "ask user".into()],
            proposed_action: String::new(),
            risk: "low".into(),
            reversible: Some(true),
            decision_type: "workflow".into(),
            scope: "global".into(),
            agent_confidence: None,
            dry_run: false,
        };
        let original = evaluate(&root, input()).unwrap();
        crate::learning::learn_feedback(crate::cli::LearnFeedbackArgs {
            decision_id: original.decision_id,
            correction: "never rename automatically; always ask user".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        crate::learning::apply(crate::cli::ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        let corrected = evaluate(
            &root,
            GateInput {
                dry_run: true,
                ..input()
            },
        )
        .unwrap();

        assert_eq!(corrected.outcome, "ask_user");
        assert_eq!(corrected.selected_option.as_deref(), Some("ask user"));
        assert!(
            corrected
                .rejected_options
                .contains(&"rename automatically".into())
        );
    }

    #[test]
    fn persisted_autopilot_disable_forces_ask() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "disabled", "off", None).unwrap();
        crate::learning::autopilot_set_threshold(Some(root.clone()), 0.95).unwrap();
        let config = crate::learning::autopilot_config(&root);
        assert_eq!(config.mode, "disabled");
        assert_eq!(config.level, "off");
        assert_eq!(config.threshold, 0.95);

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "rename a local temporary note".into(),
                options: vec!["rename now".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
    }

    #[test]
    fn persisted_ask_always_gate_mode_forces_ask() {
        let (_tmp, root) = temp_vault();
        crate::learning::gate_mode(Some(root.clone()), "ask-always").unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "rename a local temporary note".into(),
                options: vec!["rename now".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
    }

    #[test]
    fn configured_threshold_applies_to_all_proceed_outcomes() {
        let (_tmp, root) = temp_vault();
        crate::learning::autopilot_set(Some(root.clone()), "shadow", "conservative", Some(0.95))
            .unwrap();

        let res = evaluate(
            &root,
            GateInput {
                intent: "plan".into(),
                situation: "rename a local temporary note".into(),
                options: vec!["rename now".into(), "ask user".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "workflow".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();

        assert_eq!(res.outcome, "ask_user");
        assert!(
            res.reasoning_summary
                .iter()
                .any(|reason| reason.contains("threshold"))
        );
    }
}
