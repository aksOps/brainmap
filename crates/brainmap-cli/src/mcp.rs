use crate::{gate, index, learning, vault};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const TOOLS: &[&str] = &[
    "brainmap_decision_gate",
    "brainmap_should_ask_user",
    "brainmap_record_decision",
    "brainmap_learn_feedback",
    "brainmap_context",
    "brainmap_import",
    "brainmap_export",
    "brainmap_restore",
    "brainmap_autopilot_status",
];

pub fn serve(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = io::stdout();
    while let Some(message) = read_message(&mut reader)? {
        let request: Value = serde_json::from_str(&message.body)?;
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let response = match handle_method(&root, id.clone(), method, params) {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(err) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":err.to_string()}})
            }
        };
        write_response(
            &mut stdout,
            message.framed,
            &serde_json::to_string(&response)?,
        )?;
    }
    Ok(())
}

fn handle_method(root: &Path, _id: Value, method: &str, params: Value) -> Result<Value> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "brainmap-mcp", "version": env!("CARGO_PKG_VERSION")}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_descriptors()})),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .context("tools/call requires params.name")?;
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            call_tool(root, name, args)
        }
        _ => bail!("unsupported MCP method: {method}"),
    }
}

fn tool_descriptors() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "description": format!("Brainmap allowlisted tool: {name}"),
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": true
                }
            })
        })
        .collect()
}

fn call_tool(root: &Path, name: &str, args: Value) -> Result<Value> {
    if !TOOLS.contains(&name) {
        bail!("tool not allowlisted: {name}");
    }
    let value = match name {
        "brainmap_decision_gate" => serde_json::to_value(gate::evaluate(root, gate_input(args)?)?)?,
        "brainmap_should_ask_user" => {
            let situation = string_arg(&args, "situation").unwrap_or_default();
            let question = string_arg(&args, "question").unwrap_or_default();
            serde_json::to_value(gate::evaluate(
                root,
                gate::GateInput {
                    intent: "would-ask-user".into(),
                    situation: if situation.is_empty() {
                        question.clone()
                    } else {
                        situation
                    },
                    options: Vec::new(),
                    proposed_action: question,
                    risk: "medium".into(),
                    reversible: Some(true),
                    decision_type: "general".into(),
                    agent_confidence: None,
                    dry_run: false,
                },
            )?)?
        }
        "brainmap_record_decision" => {
            learning::record_decision(crate::cli::RecordDecisionArgs {
                decision_id: string_arg(&args, "decisionId"),
                chosen: string_arg(&args, "chosen"),
                was_asked: args.get("wasAsked").and_then(Value::as_bool),
                vault: Some(root.to_path_buf()),
            })?;
            json!({"recorded": true})
        }
        "brainmap_learn_feedback" => {
            learning::learn_feedback(crate::cli::LearnFeedbackArgs {
                decision_id: string_arg(&args, "decisionId")
                    .context("learn feedback requires decisionId")?,
                correction: string_arg(&args, "correction")
                    .context("learn feedback requires correction")?,
                vault: Some(root.to_path_buf()),
            })?;
            json!({"packetCreated": true})
        }
        "brainmap_context" => {
            let status = index::status(root)?;
            json!({
                "mode": "decision-engine",
                "source": "compiled-sqlite-index",
                "index": status,
                "hotPath": {
                    "llm": false,
                    "agentMemory": false,
                    "network": false,
                    "embeddingGeneration": false,
                    "modelLoad": false,
                    "fullVaultScan": false
                }
            })
        }
        "brainmap_import" => {
            json!({"supported": true, "useCli": "brainmap import --file ... --to ..."})
        }
        "brainmap_export" => {
            json!({"supported": true, "useCli": "brainmap export --mode portable --vault ... --out ..."})
        }
        "brainmap_restore" => {
            json!({"supported": true, "useCli": "brainmap restore --file ... --to ..."})
        }
        "brainmap_autopilot_status" => learning::autopilot_status_value(root),
        _ => unreachable!(),
    };
    Ok(json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&value)?}]}))
}

fn gate_input(args: Value) -> Result<gate::GateInput> {
    Ok(gate::GateInput {
        intent: string_arg(&args, "intent").unwrap_or_else(|| "unknown".into()),
        situation: string_arg(&args, "situation").unwrap_or_default(),
        options: options_arg(&args),
        proposed_action: string_arg(&args, "proposedAction").unwrap_or_default(),
        risk: string_arg(&args, "risk").unwrap_or_else(|| "medium".into()),
        reversible: args.get("reversible").and_then(Value::as_bool),
        decision_type: string_arg(&args, "decisionType").unwrap_or_else(|| "general".into()),
        agent_confidence: args.get("agentConfidence").and_then(Value::as_f64),
        dry_run: args.get("dryRun").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_string)
}

fn options_arg(args: &Value) -> Vec<String> {
    match args.get("options") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(value)) => value
            .split('|')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

struct Message {
    body: String,
    framed: bool,
}

fn read_message<R: BufRead>(reader: &mut R) -> Result<Option<Message>> {
    let mut first = String::new();
    if reader.read_line(&mut first)? == 0 {
        return Ok(None);
    }
    if first.trim().is_empty() {
        return read_message(reader);
    }
    if first.trim_start().starts_with('{') {
        return Ok(Some(Message {
            body: first,
            framed: false,
        }));
    }
    let mut content_length = parse_content_length(&first);
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        content_length = content_length.or_else(|| parse_content_length(&header));
    }
    let length = content_length.context("missing Content-Length header")?;
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(Some(Message {
        body: String::from_utf8(bytes)?,
        framed: true,
    }))
}

fn parse_content_length(line: &str) -> Option<usize> {
    let (name, value) = line.split_once(':')?;
    if name.eq_ignore_ascii_case("content-length") {
        value.trim().parse().ok()
    } else {
        None
    }
}

fn write_response<W: Write>(writer: &mut W, framed: bool, json: &str) -> Result<()> {
    if framed {
        write!(writer, "Content-Length: {}\r\n\r\n{}", json.len(), json)?;
    } else {
        writeln!(writer, "{json}")?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_allowlisted_tools() {
        let tools = tool_descriptors();
        assert!(tools.iter().any(|tool| {
            tool.get("name").and_then(Value::as_str) == Some("brainmap_decision_gate")
        }));
        assert!(!TOOLS.contains(&"shell"));
    }

    #[test]
    fn parses_content_length_frame() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let raw = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut reader = BufReader::new(raw.as_bytes());
        let message = read_message(&mut reader).unwrap().unwrap();
        assert!(message.framed);
        assert_eq!(message.body, body);
    }
}
