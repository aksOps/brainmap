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
    "brainmap_list_pending",
    "brainmap_preview_update",
    "brainmap_apply_update",
    "brainmap_context",
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
    TOOLS.iter().map(|name| tool_descriptor(name)).collect()
}

fn tool_descriptor(name: &str) -> Value {
    let (description, properties, required, extra) = match name {
        "brainmap_decision_gate" => (
            "Evaluate one structured local decision through the deterministic Brainmap gate.",
            json!({
                "intent": {"type":"string"},
                "situation": {"type":"string"},
                "options": {"oneOf":[{"type":"array","items":{"type":"string"}},{"type":"string"}]},
                "proposedAction": {"type":"string"},
                "risk": {"type":"string","enum":["low","medium","high","critical"]},
                "reversible": {"type":"boolean"},
                "decisionType": {"type":"string"},
                "scope": {"type":"string"},
                "agentConfidence": {"type":"number","minimum":0,"maximum":1},
                "dryRun": {"type":"boolean"}
            }),
            json!(["situation", "options", "risk", "reversible"]),
            json!({}),
        ),
        "brainmap_should_ask_user" => (
            "Check whether a proposed user question should be asked.",
            json!({
                "question": {"type":"string"},
                "situation": {"type":"string"}
            }),
            json!(["question"]),
            json!({}),
        ),
        "brainmap_record_decision" => (
            "Record the action taken for a prior Brainmap decision ID.",
            json!({
                "decisionId": {"type":"string"},
                "chosen": {"type":"string"},
                "wasAsked": {"type":"boolean"}
            }),
            json!(["decisionId", "chosen"]),
            json!({}),
        ),
        "brainmap_learn_feedback" => (
            "Create a pending scoped correction for a prior decision; this does not activate it.",
            json!({
                "decisionId": {"type":"string"},
                "correction": {"type":"string"},
                "chosen": {"type":"string"},
                "rejected": {"type":"string"},
                "incidentType": {
                    "type":"string",
                    "enum": crate::cli::FeedbackIncident::ALL
                        .iter()
                        .map(|incident| incident.as_str())
                        .collect::<Vec<_>>()
                }
            }),
            json!(["decisionId"]),
            json!({"anyOf":[{"required":["correction"]},{"required":["chosen"]}]}),
        ),
        "brainmap_list_pending" => (
            "List pending update packets without activating them.",
            json!({}),
            json!([]),
            json!({}),
        ),
        "brainmap_preview_update" => (
            "Preview one pending update packet before approval.",
            json!({"packetId":{"type":"string"}}),
            json!(["packetId"]),
            json!({}),
        ),
        "brainmap_apply_update" => (
            "Apply one explicitly approved pending update packet.",
            json!({
                "packetId":{"type":"string"},
                "approved":{"type":"boolean","const":true}
            }),
            json!(["packetId", "approved"]),
            json!({}),
        ),
        "brainmap_context" => (
            "Read bounded decision context and local hot-path status.",
            json!({}),
            json!([]),
            json!({}),
        ),
        "brainmap_autopilot_status" => (
            "Read autopilot configuration and aggregate shadow metrics.",
            json!({}),
            json!([]),
            json!({}),
        ),
        _ => unreachable!(),
    };
    let mut schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    });
    if let (Some(schema), Some(extra)) = (schema.as_object_mut(), extra.as_object()) {
        schema.extend(extra.clone());
    }
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema
    })
}

fn call_tool(root: &Path, name: &str, args: Value) -> Result<Value> {
    if !TOOLS.contains(&name) {
        bail!("tool not allowlisted: {name}");
    }
    validate_tool_arguments(name, &args)?;
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
                    scope: crate::util::default_project_scope(),
                    agent_confidence: None,
                    dry_run: false,
                },
            )?)?
        }
        "brainmap_record_decision" => {
            learning::record_decision_quiet(crate::cli::RecordDecisionArgs {
                decision_id: Some(
                    string_arg(&args, "decisionId")
                        .context("record decision requires decisionId")?,
                ),
                chosen: Some(
                    string_arg(&args, "chosen").context("record decision requires chosen")?,
                ),
                was_asked: args.get("wasAsked").and_then(Value::as_bool),
                vault: Some(root.to_path_buf()),
            })?;
            json!({"recorded": true})
        }
        "brainmap_learn_feedback" => {
            let packet_id = learning::learn_feedback_quiet(crate::cli::LearnFeedbackArgs {
                decision_id: string_arg(&args, "decisionId")
                    .context("learn feedback requires decisionId")?,
                correction: string_arg(&args, "correction"),
                chosen: string_arg(&args, "chosen"),
                rejected: string_arg(&args, "rejected"),
                incident: string_arg(&args, "incidentType")
                    .map(|value| crate::cli::FeedbackIncident::parse(&value))
                    .transpose()
                    .map_err(anyhow::Error::msg)?,
                vault: Some(root.to_path_buf()),
            })?;
            json!({"packetCreated": packet_id.is_some(), "packetId": packet_id})
        }
        "brainmap_list_pending" => learning::pending_updates_value(root, None)?,
        "brainmap_preview_update" => {
            let packet_id =
                string_arg(&args, "packetId").context("preview update requires packetId")?;
            learning::pending_updates_value(root, Some(&packet_id))?
        }
        "brainmap_apply_update" => {
            if args.get("approved").and_then(Value::as_bool) != Some(true) {
                bail!("apply update requires approved=true");
            }
            let packet_id =
                string_arg(&args, "packetId").context("apply update requires packetId")?;
            learning::apply_update_by_id(root, &packet_id)?;
            json!({"applied": true, "packetId": packet_id})
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
        "brainmap_autopilot_status" => learning::autopilot_status_value(root),
        _ => unreachable!(),
    };
    Ok(json!({"content":[{"type":"text","text":serde_json::to_string_pretty(&value)?}]}))
}

fn validate_tool_arguments(name: &str, args: &Value) -> Result<()> {
    let arguments = args
        .as_object()
        .context("MCP tool arguments must be a JSON object")?;
    let descriptor = tool_descriptor(name);
    let properties = descriptor["inputSchema"]["properties"]
        .as_object()
        .context("MCP tool descriptor is missing properties")?;
    if let Some(argument) = arguments
        .keys()
        .find(|argument| !properties.contains_key(*argument))
    {
        bail!("unsupported argument {argument} for MCP tool {name}");
    }
    Ok(())
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
        scope: string_arg(&args, "scope").unwrap_or_else(crate::util::default_project_scope),
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
        for expected in [
            "brainmap_list_pending",
            "brainmap_preview_update",
            "brainmap_apply_update",
        ] {
            assert!(
                tools
                    .iter()
                    .any(|tool| tool.get("name").and_then(Value::as_str) == Some(expected)),
                "missing allowlisted learning tool {expected}"
            );
        }
        assert!(!TOOLS.contains(&"shell"));
        assert!(!TOOLS.contains(&"brainmap_import"));
        for tool in tools {
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            assert!(tool["inputSchema"]["properties"].is_object());
        }
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

    #[test]
    fn mcp_learning_tools_preview_and_require_approval() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        learning::learn_decision(crate::cli::LearnDecisionArgs {
            situation: "Choose package manager".into(),
            options: "npm|pnpm".into(),
            chosen: "pnpm".into(),
            rejected: Some("npm".into()),
            rationale: None,
            decision_type: "tooling".into(),
            scope: "project:alpha".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        let pending = learning::pending_updates_value(&root, None).unwrap();
        let packet_id = pending[0]["id"].as_str().unwrap();

        let preview = call_tool(
            &root,
            "brainmap_preview_update",
            json!({"packetId": packet_id}),
        )
        .unwrap();
        assert!(preview.to_string().contains(packet_id));

        let error = call_tool(
            &root,
            "brainmap_apply_update",
            json!({"packetId": packet_id}),
        )
        .unwrap_err();
        assert!(error.to_string().contains("approved=true"));

        call_tool(
            &root,
            "brainmap_apply_update",
            json!({"packetId": packet_id, "approved": true}),
        )
        .unwrap();
        assert_eq!(
            learning::pending_updates_value(&root, None)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn mcp_rejects_arguments_outside_the_advertised_schema() {
        let error = call_tool(
            std::path::Path::new("/unused"),
            "brainmap_decision_gate",
            json!({
                "situation": "Choose formatter",
                "options": ["biome", "prettier"],
                "risk": "low",
                "reversible": true,
                "shell": "rm -rf /"
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unsupported argument shell"));
    }
}
