use crate::cli::{DecideArgs, GateArgs, ShouldAskArgs};
use crate::{index, privacy, util, vault};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateResponse {
    #[serde(rename = "decisionId")]
    pub decision_id: String,
    pub outcome: String,
    pub recommendation: String,
    #[serde(rename = "selectedOption")]
    pub selected_option: Option<String>,
    #[serde(rename = "rejectedOptions")]
    pub rejected_options: Vec<String>,
    pub confidence: f64,
    #[serde(rename = "riskTier")]
    pub risk_tier: String,
    #[serde(rename = "reasoningSummary")]
    pub reasoning_summary: Vec<String>,
    #[serde(rename = "matchedPolicies")]
    pub matched_policies: Vec<String>,
    #[serde(rename = "restrictionsApplied")]
    pub restrictions_applied: Vec<String>,
    #[serde(rename = "askUserQuestion")]
    pub ask_user_question: Option<String>,
    #[serde(rename = "defaultIfNoAnswer")]
    pub default_if_no_answer: Option<String>,
    #[serde(rename = "learningEvent")]
    pub learning_event: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct GateInput {
    pub intent: String,
    pub situation: String,
    pub options: Vec<String>,
    pub proposed_action: String,
    pub risk: String,
    pub reversible: Option<bool>,
    pub decision_type: String,
    pub agent_confidence: Option<f64>,
    pub dry_run: bool,
}

impl GateInput {
    fn combined(&self) -> String {
        format!(
            "{} {} {} {} {}",
            self.intent,
            self.situation,
            self.options.join(" "),
            self.proposed_action,
            self.decision_type
        )
    }
}

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
    let combined = privacy::redact(&input.combined());
    let lower = combined.to_lowercase();
    let decision_id = util::id("dec", &combined);
    let mut matched = index::policy_paths_for(root, &["local", "privacy", "approval", "question"])
        .unwrap_or_default();
    matched.truncate(6);
    let mut restrictions = Vec::new();
    let mut outcome = "ask_user".to_string();
    let mut recommendation = "Ask a focused question before proceeding.".to_string();
    let mut selected = None;
    let mut confidence = input.agent_confidence.unwrap_or(0.56).clamp(0.0, 1.0);
    let mut risk_tier = "ask_before_action".to_string();
    let mut summary = vec!["Applied deterministic policy precedence.".to_string()];
    let threshold = 0.82;
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
        selected = options.first().cloned();
        outcome = "proceed".into();
        confidence = confidence.max(0.84);
        risk_tier = "reversible_auto".into();
        recommendation = selected
            .as_ref()
            .map(|s| format!("Proceed with {s}; low-risk reversible action."))
            .unwrap_or_else(|| "Proceed; low-risk reversible action.".into());
        summary.push("Low-risk reversible action cleared threshold.".into());
    } else if confidence >= threshold && reversible {
        outcome = "proceed".into();
        selected = options.first().cloned();
        recommendation = "Proceed; confidence meets threshold and no restriction matched.".into();
        summary.push("Confidence exceeded threshold.".into());
    }

    if (std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() == Some("1")
        || std::env::var("BRAINMAP_GATE_MODE").ok().as_deref() == Some("ask-always"))
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
        "ask_user" => Some(focused_question(&input, selected.as_deref())),
        "needs_more_context" => {
            Some("What situation, options, and risk should Brainmap evaluate?".into())
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

fn focused_question(input: &GateInput, selected: Option<&str>) -> String {
    if input.risk.eq_ignore_ascii_case("high") || input.risk.eq_ignore_ascii_case("critical") {
        return "Do you explicitly approve this high-risk or irreversible action?".into();
    }
    if let Some(selected) = selected {
        format!("Proceed with {selected}, or choose another option?")
    } else if input.options.is_empty() {
        "What option should Brainmap compare against the default reversible path?".into()
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
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "proceed");
        assert_eq!(res.selected_option.as_deref(), Some("Markdown+JSONL"));
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
                agent_confidence: None,
                dry_run: true,
            },
        )
        .unwrap();
        assert_eq!(res.outcome, "ask_user");
    }
}
