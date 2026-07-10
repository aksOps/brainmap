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
        let response = gate::evaluate(
            &root,
            gate::GateInput {
                intent: request.intent.unwrap_or_else(|| "would-ask-user".into()),
                situation: request.situation.unwrap_or_default(),
                options: parse_options(request.options),
                proposed_action: request.proposed_action.unwrap_or_default(),
                risk: request.risk.unwrap_or_else(|| "medium".into()),
                reversible: request.reversible,
                decision_type: request.decision_type.unwrap_or_else(|| "general".into()),
                scope: request.scope.unwrap_or_else(|| "global".into()),
                agent_confidence: request.agent_confidence,
                dry_run: false,
            },
        )?;
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

pub fn hook(args: HookArgs) -> Result<()> {
    let mut payload = String::new();
    io::stdin().read_to_string(&mut payload)?;
    let root = vault::resolve_vault(args.vault);
    let response = gate::evaluate(&root, hook_gate_input(&args.host, &args.event, &payload))?;

    if hook_should_block(&args.event, &response.outcome) {
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
        scope: "global".into(),
        agent_confidence: Some(0.86),
        dry_run: true,
    }
}

fn hook_should_block(event: &str, outcome: &str) -> bool {
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
    fn hook_blocks_pre_tool_approval_but_not_prompt_advice() {
        assert!(hook_should_block("PreToolUse", "ask_user"));
        assert!(hook_should_block("UserPromptSubmit", "block"));
        assert!(!hook_should_block("UserPromptSubmit", "ask_user"));
        assert!(!hook_should_block("PreToolUse", "proceed"));
    }

    #[test]
    fn user_prompt_advice_hides_policy_layer() {
        let advice = user_prompt_advice(&gate::GateResponse {
            decision_id: "dec_test".into(),
            outcome: "ask_user".into(),
            recommendation: "Ask before proceeding.".into(),
            selected_option: None,
            rejected_options: vec![],
            confidence: 0.56,
            risk_tier: "ask_before_action".into(),
            reasoning_summary: vec![],
            matched_policies: vec![],
            restrictions_applied: vec![],
            ask_user_question: Some("Which path should I take?".into()),
            default_if_no_answer: None,
            learning_event: serde_json::json!({}),
        })
        .unwrap();

        assert_eq!(advice, "Which path should I take?");
        assert!(!advice.contains("Brainmap"));
        assert!(!advice.contains("outcome="));
        assert!(!advice.contains("confidence="));
    }
}
