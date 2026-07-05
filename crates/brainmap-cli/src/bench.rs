use crate::cli::BenchArgs;
use crate::{gate, index, learning, vault};
use anyhow::Result;
use serde_json::json;
use std::time::Instant;

pub fn run(args: BenchArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    if !root.exists() {
        vault::init_vault(Some(root.clone()), false, true)?;
        index::rebuild(&root)?;
    } else if !index::db_path(&root).exists() {
        index::rebuild(&root)?;
    }
    let gate_start = Instant::now();
    let _ = gate::evaluate(
        &root,
        gate::GateInput {
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
    )?;
    let gate_ms = gate_start.elapsed().as_millis();
    let cap_start = Instant::now();
    learning::capture(crate::cli::CaptureArgs {
        stdin: false,
        text: Some("User chose local-first storage over external vector DB.".into()),
        source: "manual".into(),
        vault: Some(root.clone()),
    })?;
    let capture_ms = cap_start.elapsed().as_millis();
    let fts_start = Instant::now();
    let fts = index::search_text(&root, "local", 10).unwrap_or_default();
    let fts_ms = fts_start.elapsed().as_millis();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "vault": root,
            "gateMs": gate_ms,
            "captureMs": capture_ms,
            "ftsMs": fts_ms,
            "ftsResults": fts.len(),
            "daemonGateMs": null,
            "memoryMb": null,
            "hotPath": {
                "llm": false,
                "agentMemory": false,
                "network": false,
                "embeddingGeneration": false,
                "modelLoad": false
            }
        }))?
    );
    Ok(())
}
