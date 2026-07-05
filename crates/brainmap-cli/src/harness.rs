use crate::{gate, vault};
use anyhow::Result;
use clap::Args;
use serde::Deserialize;
use std::io::{self, BufRead};
use std::path::PathBuf;

#[derive(Args)]
pub struct StdioArgs {
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub fail_on_block: bool,
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
}
