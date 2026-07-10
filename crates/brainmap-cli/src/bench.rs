use crate::cli::BenchArgs;
use crate::{context, gate, index, learning, markdown, model, vault};
use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::Instant;

const SCALE_DIR: &str = "90-calibration/scale-bench";
const MAX_SCALE: usize = 25_000;

pub fn run(args: BenchArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    if let Some(scale) = args.scale {
        generate_scale_notes(&root, scale)?;
    } else if !root.exists() {
        vault::init_vault(Some(root.clone()), false, true)?;
    }
    let rebuild_ms = if args.scale.is_some() || !index::db_path(&root).exists() {
        Some(timed(|| index::rebuild(&root))?)
    } else {
        None
    };
    let notes = indexed_note_count(&root)?;
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
            scope: "global".into(),
            agent_confidence: None,
            dry_run: true,
        },
    )?;
    let gate_ms = gate_start.elapsed().as_millis();
    let cap_start = Instant::now();
    learning::capture_text(
        &root,
        "User chose local-first storage over external vector DB.",
        "manual",
    )?;
    let capture_ms = cap_start.elapsed().as_millis();
    let context_ms = timed(|| context::load_fast_context(&root, 8).map(|_| ()))?;
    let fts_start = Instant::now();
    let fts = index::search_text(&root, &args.query, 10).unwrap_or_default();
    let fts_ms = fts_start.elapsed().as_millis();
    let vector = if args.embeddings {
        let (model_ms, model_changed) = timed_value(|| model::materialize_model(&root, false))?;
        let (embed_ms, _) = timed_value(|| model::embed_notes(&root, false))?;
        let embedded_notes = model::embedding_count(&root)?;
        let vector_start = Instant::now();
        let vector_results = model::search_vector(&root, &args.query, 10)?.len();
        Some(json!({
            "model": "minishlab/potion-base-8M",
            "dimension": model::DIMENSION,
            "rawVectorBytes": embedded_notes * model::DIMENSION * std::mem::size_of::<f32>(),
            "materializeMs": model_ms,
            "materializedChanged": model_changed.1,
            "embedRebuildMs": embed_ms,
            "embeddedNotes": embedded_notes,
            "vectorSearchMs": vector_start.elapsed().as_millis(),
            "vectorResults": vector_results,
        }))
    } else {
        None
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "vault": root,
            "scaleRequested": args.scale,
            "scaleMax": MAX_SCALE,
            "generatedUnder": args.scale.map(|_| SCALE_DIR),
            "notes": notes,
            "indexRebuildMs": rebuild_ms,
            "gateMs": gate_ms,
            "contextFastMs": context_ms,
            "captureMs": capture_ms,
            "ftsMs": fts_ms,
            "ftsResults": fts.len(),
            "query": args.query,
            "embeddings": vector,
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

fn timed(work: impl FnOnce() -> Result<()>) -> Result<u128> {
    let start = Instant::now();
    work()?;
    Ok(start.elapsed().as_millis())
}

fn timed_value<T>(work: impl FnOnce() -> Result<T>) -> Result<(u128, T)> {
    let start = Instant::now();
    let value = work()?;
    Ok((start.elapsed().as_millis(), value))
}

fn generate_scale_notes(root: &Path, count: usize) -> Result<()> {
    if count == 0 || count > MAX_SCALE {
        bail!("--scale must be between 1 and {MAX_SCALE}");
    }
    let base = root.join(SCALE_DIR);
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).with_context(|| format!("create {}", base.display()))?;
    for i in 0..count {
        let shard = base.join(format!("{:02}", i / 1000));
        fs::create_dir_all(&shard).with_context(|| format!("create {}", shard.display()))?;
        let id = format!("bench-decision-{i:05}");
        let path = shard.join(format!("{id}.md"));
        fs::write(path, synthetic_note(i, &id)?)?;
    }
    Ok(())
}

fn synthetic_note(i: usize, id: &str) -> Result<String> {
    let note_type = match i % 6 {
        0 => "decision-policy",
        1 => "tradeoff-rule",
        2 => "hard-constraint",
        3 => "uncertainty-rule",
        4 => "soft-preference",
        _ => "ask-trigger",
    };
    let risk_tier = match i % 5 {
        0 => "reversible-auto",
        1 => "suggest-only",
        2 => "approval-required",
        3 => "ask-before-action",
        _ => "never-auto",
    };
    let link = if i == 0 {
        String::new()
    } else {
        format!(
            "\nRelated precedent: [[bench-decision-{:05}]].",
            i.saturating_sub(1)
        )
    };
    Ok(format!(
        "{}# Bench Decision {:05}\n\n## Policy\n\nPrefer local-first decisions, embedded SQLite, deterministic gates, and reversible defaults for personal tooling.\n\n## Signals\n\nThis synthetic note exercises full-text search, graph links, and local 256-dimensional note embeddings for production scale checks.{link}\n",
        markdown::frontmatter(id, note_type, risk_tier, "personal"),
        i
    ))
}

fn indexed_note_count(root: &Path) -> Result<usize> {
    let conn = index::connection(root)?;
    let count: i64 = conn.query_row("select count(*) from notes", params![], |row| row.get(0))?;
    Ok(count as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_notes_are_valid_markdown_notes() {
        let text = synthetic_note(7, "bench-decision-00007").unwrap();
        let note = markdown::parse_note("bench.md".into(), &text).unwrap();
        assert_eq!(note.id, "bench-decision-00007");
        assert_eq!(note.links, vec!["bench-decision-00006"]);
    }

    #[test]
    fn scale_generation_replaces_only_benchmark_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        generate_scale_notes(&root, 3).unwrap();
        generate_scale_notes(&root, 2).unwrap();
        let files = crate::util::collect_files(&root).unwrap();
        let markdown_count = files
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .count();
        assert_eq!(markdown_count, 2);
    }
}
