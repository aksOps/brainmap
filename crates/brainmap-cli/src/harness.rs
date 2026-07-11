use crate::{gate, vault};
use anyhow::Result;
use clap::Args;
use serde::Deserialize;
use serde_json::Value;
use std::io::{self, BufRead, Read};
use std::path::PathBuf;

#[derive(Args)]
pub struct StdioArgs {
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub fail_on_block: bool,
}

#[derive(Args)]
pub struct HookArgs {
    #[arg(long)]
    pub host: String,
    #[arg(long)]
    pub event: String,
    #[arg(long)]
    pub vault: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GateRequest {
    intent: Option<String>,
    situation: Option<String>,
    options: Option<serde_json::Value>,
    #[serde(rename = "proposedAction")]
    proposed_action: Option<String>,
    risk: Option<String>,
    reversible: Option<bool>,
    #[serde(rename = "decisionType")]
    decision_type: Option<String>,
    scope: Option<String>,
    #[serde(rename = "agentConfidence")]
    agent_confidence: Option<f64>,
}

pub fn stdio(args: StdioArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let mut blocked = false;
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: GateRequest = serde_json::from_str(&line)?;
        let response = gate::evaluate(&root, stdio_gate_input(request))?;
        if response.outcome == "block" {
            blocked = true;
        }
        println!("{}", serde_json::to_string(&response)?);
    }
    if blocked && args.fail_on_block {
        std::process::exit(2);
    }
    Ok(())
}

fn stdio_gate_input(request: GateRequest) -> gate::GateInput {
    gate::GateInput {
        intent: request.intent.unwrap_or_else(|| "would-ask-user".into()),
        situation: request.situation.unwrap_or_default(),
        options: parse_options(request.options),
        proposed_action: request.proposed_action.unwrap_or_default(),
        risk: request.risk.unwrap_or_else(|| "medium".into()),
        reversible: request.reversible,
        decision_type: request.decision_type.unwrap_or_else(|| "general".into()),
        scope: request
            .scope
            .unwrap_or_else(crate::util::default_project_scope),
        agent_confidence: request.agent_confidence,
        dry_run: false,
    }
}

pub fn hook(args: HookArgs) -> Result<()> {
    let mut payload = String::new();
    io::stdin().read_to_string(&mut payload)?;
    let root = vault::resolve_vault(args.vault);
    let response = gate::evaluate(&root, hook_gate_input(&args.host, &args.event, &payload))?;

    if hook_should_block(&args.event, &response) {
        let label = if response.outcome == "block" {
            "Action rejected"
        } else {
            "Action needs confirmation"
        };
        eprintln!("{label}: {}", response.recommendation);
        if let Some(question) = response.ask_user_question {
            eprintln!("{question}");
        }
        std::process::exit(2);
    }

    if is_user_prompt(&args.event)
        && let Some(advice) = user_prompt_advice(&response)
    {
        println!("{advice}");
    }

    Ok(())
}

fn user_prompt_advice(response: &gate::GateResponse) -> Option<String> {
    match response.outcome.as_str() {
        "ask_user" => Some(
            response
                .ask_user_question
                .clone()
                .unwrap_or_else(|| response.recommendation.clone()),
        ),
        "no_action" => Some("No need to ask this question.".into()),
        _ => None,
    }
}

fn parse_options(value: Option<serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(values)) => values
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => s
            .split('|')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn hook_gate_input(host: &str, event: &str, payload: &str) -> gate::GateInput {
    let parsed = serde_json::from_str::<Value>(payload).ok();
    let tool = parsed
        .as_ref()
        .and_then(|v| first_string(v, &["tool_name", "toolName", "tool"]))
        .unwrap_or_else(|| "none".into());
    let detail = parsed
        .as_ref()
        .map(hook_detail)
        .unwrap_or_else(|| compact(payload));
    let dangerous = is_pre_tool(event) && dangerous_action(&tool, &detail);
    let reversible = !dangerous;
    let risk = if dangerous { "high" } else { "low" };
    let situation = if dangerous {
        format!(
            "Host {host}; event {event}; tool {tool}; irreversible-risk=true; {}",
            detail
        )
    } else {
        format!("Host {host}; event {event}; tool {tool}; {detail}")
    };

    gate::GateInput {
        intent: format!("agent-hook:{event}"),
        situation,
        options: vec!["proceed".into(), "ask_user".into(), "block".into()],
        proposed_action: detail,
        risk: risk.into(),
        reversible: Some(reversible),
        decision_type: "agent-harness".into(),
        scope: crate::util::default_project_scope(),
        agent_confidence: Some(0.86),
        dry_run: false,
    }
}

fn hook_should_block(event: &str, response: &gate::GateResponse) -> bool {
    let shadow_prediction_is_routine = is_pre_tool(event)
        && (response.gate_mode == "shadow" || response.autopilot_mode == "shadow")
        && response.predicted_outcome == "proceed";
    let outcome = if shadow_prediction_is_routine {
        &response.predicted_outcome
    } else {
        &response.outcome
    };
    hook_outcome_should_block(event, outcome)
}

fn hook_outcome_should_block(event: &str, outcome: &str) -> bool {
    outcome == "block" || (is_pre_tool(event) && outcome == "ask_user")
}

fn is_pre_tool(event: &str) -> bool {
    event.eq_ignore_ascii_case("PreToolUse") || event.eq_ignore_ascii_case("pre_tool_use")
}

fn is_user_prompt(event: &str) -> bool {
    event.eq_ignore_ascii_case("UserPromptSubmit")
        || event.eq_ignore_ascii_case("user_prompt_submit")
}

fn hook_detail(value: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(prompt) = first_string(value, &["prompt", "user_prompt", "message"]) {
        parts.push(format!("prompt={}", compact(&prompt)));
    }
    if let Some(input) = value.get("tool_input").or_else(|| value.get("toolInput")) {
        parts.push(format!("tool_input={}", compact_value(input)));
    }
    if let Some(command) = first_string(value, &["command"]) {
        parts.push(format!("command={}", compact(&command)));
    }
    if parts.is_empty() {
        parts.push(compact_value(value));
    }
    compact(&parts.join("; "))
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(s) = value.get(*key).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    None
}

fn compact_value(value: &Value) -> String {
    compact(&serde_json::to_string(value).unwrap_or_default())
}

fn compact(value: &str) -> String {
    const MAX: usize = 2000;
    let one_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.len() <= MAX {
        one_line
    } else {
        format!("{}...", one_line.chars().take(MAX).collect::<String>())
    }
}

fn dangerous_action(tool: &str, detail: &str) -> bool {
    let combined = format!("{tool} {detail}").to_lowercase();
    combined.contains("rm -rf")
        || combined.contains("git reset --hard")
        || combined.contains("drop database")
        || combined.contains("delete")
        || combined.contains("destroy")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_array_and_pipe_options() {
        assert_eq!(
            parse_options(Some(serde_json::json!(["A", "B"]))),
            vec!["A", "B"]
        );
        assert_eq!(
            parse_options(Some(serde_json::json!("A|B"))),
            vec!["A", "B"]
        );
    }

    #[test]
    fn golden_generic_stdio_payload_maps_every_contract_field() {
        let request: GateRequest = serde_json::from_value(serde_json::json!({
            "intent": "would-ask-user",
            "situation": "Choose formatter",
            "options": ["biome", "prettier"],
            "proposedAction": "write biome.json",
            "risk": "low",
            "reversible": true,
            "decisionType": "tooling",
            "scope": "project:alpha",
            "agentConfidence": 0.75
        }))
        .unwrap();
        let input = stdio_gate_input(request);

        assert_eq!(input.intent, "would-ask-user");
        assert_eq!(input.situation, "Choose formatter");
        assert_eq!(input.options, ["biome", "prettier"]);
        assert_eq!(input.proposed_action, "write biome.json");
        assert_eq!(input.risk, "low");
        assert_eq!(input.reversible, Some(true));
        assert_eq!(input.decision_type, "tooling");
        assert_eq!(input.scope, "project:alpha");
        assert_eq!(input.agent_confidence, Some(0.75));
        assert!(!input.dry_run);
    }

    #[test]
    fn hook_marks_destructive_tool_use_irreversible() {
        let input = hook_gate_input(
            "codex",
            "PreToolUse",
            r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf target"}}"#,
        );
        assert_eq!(input.risk, "high");
        assert_eq!(input.reversible, Some(false));
        assert!(input.situation.contains("irreversible-risk=true"));
    }

    #[test]
    fn golden_codex_and_claude_hook_payloads_map_to_the_safety_contract() {
        let codex = hook_gate_input(
            "codex",
            "PreToolUse",
            r#"{"toolName":"Write","toolInput":{"path":"src/lib.rs"}}"#,
        );
        assert_eq!(codex.intent, "agent-hook:PreToolUse");
        assert_eq!(codex.decision_type, "agent-harness");
        assert!(codex.situation.contains("tool Write"));
        assert!(codex.proposed_action.contains("src/lib.rs"));
        assert_eq!(codex.reversible, Some(true));

        let claude = hook_gate_input(
            "claude-code",
            "PreToolUse",
            r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf target"}}"#,
        );
        assert!(claude.situation.contains("tool Bash"));
        assert!(claude.situation.contains("irreversible-risk=true"));
        assert_eq!(claude.risk, "high");
        assert_eq!(claude.reversible, Some(false));

        let prompt = hook_gate_input(
            "codex",
            "UserPromptSubmit",
            r#"{"prompt":"Should I use npm or pnpm?"}"#,
        );
        assert!(prompt.proposed_action.contains("npm or pnpm"));
        assert_eq!(prompt.intent, "agent-hook:UserPromptSubmit");
        assert!(
            !prompt.dry_run,
            "host hook execution must be recorded for audit and qualification"
        );
    }

    #[test]
    fn pre_tool_hook_allows_routine_actions_and_stops_destructive_actions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        crate::vault::init_vault(Some(root.clone()), false, true).unwrap();
        crate::learning::autopilot_set(Some(root.clone()), "conservative", "conservative", None)
            .unwrap();
        crate::learning::gate_mode(Some(root.clone()), "active").unwrap();
        crate::index::rebuild(&root).unwrap();

        let routine = gate::evaluate(
            &root,
            hook_gate_input(
                "codex",
                "PreToolUse",
                r#"{"tool_name":"Write","tool_input":{"path":"src/lib.rs"}}"#,
            ),
        )
        .unwrap();
        assert_eq!(routine.outcome, "proceed");
        assert_eq!(routine.selected_option.as_deref(), Some("proceed"));
        let ledger = std::fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl"))
            .expect("read hook audit ledger");
        let event: serde_json::Value =
            serde_json::from_str(ledger.lines().last().expect("hook audit event"))
                .expect("parse hook audit event");
        assert_eq!(event["intent"], "agent-hook:PreToolUse");
        assert_eq!(event["decisionType"], "agent-harness");

        let destructive = gate::evaluate(
            &root,
            hook_gate_input(
                "claude-code",
                "PreToolUse",
                r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf target"}}"#,
            ),
        )
        .unwrap();
        assert!(matches!(destructive.outcome.as_str(), "ask_user" | "block"));
        assert!(hook_should_block("PreToolUse", &destructive));
    }

    #[test]
    fn shadow_pre_tool_hook_observes_routine_actions_without_self_deadlock() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        crate::vault::init_vault(Some(root.clone()), false, true).unwrap();
        crate::index::rebuild(&root).unwrap();

        let routine = gate::evaluate(
            &root,
            hook_gate_input(
                "codex",
                "PreToolUse",
                r#"{"tool_name":"Bash","tool_input":{"command":"brainmap skill build-decision-engine --host codex"}}"#,
            ),
        )
        .unwrap();
        assert_eq!(routine.gate_mode, "shadow");
        assert_eq!(routine.autopilot_mode, "shadow");
        assert_eq!(routine.predicted_outcome, "proceed");
        assert_eq!(routine.outcome, "ask_user");
        assert!(!hook_should_block("PreToolUse", &routine));

        let destructive = gate::evaluate(
            &root,
            hook_gate_input(
                "codex",
                "PreToolUse",
                r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf target"}}"#,
            ),
        )
        .unwrap();
        assert!(matches!(
            destructive.predicted_outcome.as_str(),
            "ask_user" | "block"
        ));
        assert!(hook_should_block("PreToolUse", &destructive));
    }

    #[test]
    fn hook_blocks_pre_tool_approval_but_not_prompt_advice() {
        assert!(hook_outcome_should_block("PreToolUse", "ask_user"));
        assert!(hook_outcome_should_block("UserPromptSubmit", "block"));
        assert!(!hook_outcome_should_block("UserPromptSubmit", "ask_user"));
        assert!(!hook_outcome_should_block("PreToolUse", "proceed"));
    }

    #[test]
    fn user_prompt_advice_hides_policy_layer() {
        let advice = user_prompt_advice(&gate::GateResponse {
            decision_id: "dec_test".into(),
            outcome: "ask_user".into(),
            predicted_outcome: "ask_user".into(),
            recommendation: "Ask before proceeding.".into(),
            selected_option: None,
            predicted_selected_option: None,
            rejected_options: vec![],
            confidence: 0.56,
            rule_id: None,
            rule_scope: None,
            match_score: None,
            match_kind: None,
            match_margin: None,
            candidate_collision: false,
            risk_tier: "ask_before_action".into(),
            reasoning_summary: vec![],
            matched_policies: vec![],
            applied_policies: vec![],
            restrictions_applied: vec![],
            ask_user_question: Some("Which path should I take?".into()),
            default_if_no_answer: None,
            gate_mode: "active".into(),
            autopilot_mode: "conservative".into(),
            dogfood_run_id: None,
            learning_event: serde_json::json!({}),
        })
        .unwrap();

        assert_eq!(advice, "Which path should I take?");
        assert!(!advice.contains("Brainmap"));
        assert!(!advice.contains("outcome="));
        assert!(!advice.contains("confidence="));
    }
}
