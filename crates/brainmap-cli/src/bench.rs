use crate::cli::BenchArgs;
use crate::{context, gate, index, learning, markdown, model, vault};
use anyhow::{Context, Result, bail};
use rusqlite::params;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::time::Instant;

const SCALE_DIR: &str = "90-calibration/scale-bench";
const GATE_WARMUP_ITERATIONS: usize = 10;
const GATE_SAMPLE_ITERATIONS: usize = 200;

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
    let executable_rules = indexed_rule_count(&root)?;
    let collision_request = || gate::GateInput {
        intent: "would-ask-user".into(),
        situation: "Pick benchmark storage local deterministic".into(),
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
    };
    let unavailable_choice_request = || gate::GateInput {
        intent: "would-ask-user".into(),
        situation: "Pick formatter database benchmark fallback alpha beta gamma".into(),
        options: vec!["rustfmt".into(), "dprint".into()],
        proposed_action: String::new(),
        risk: "low".into(),
        reversible: Some(true),
        decision_type: "architecture".into(),
        scope: "global".into(),
        agent_confidence: None,
        dry_run: true,
    };
    let gate_probe = gate::evaluate(&root, collision_request())?;
    let unavailable_choice_probe = gate::evaluate(&root, unavailable_choice_request())?;
    for _ in 0..GATE_WARMUP_ITERATIONS {
        let _ = gate::evaluate(&root, unavailable_choice_request())?;
    }
    let mut gate_samples_us = Vec::with_capacity(GATE_SAMPLE_ITERATIONS);
    for _ in 0..GATE_SAMPLE_ITERATIONS {
        let gate_start = Instant::now();
        let _ = gate::evaluate(&root, unavailable_choice_request())?;
        gate_samples_us.push(gate_start.elapsed().as_micros());
    }
    gate_samples_us.sort_unstable();
    let gate_p50_ms = percentile_ms(&gate_samples_us, 50);
    let gate_p95_ms = percentile_ms(&gate_samples_us, 95);
    let gate_max_ms = gate_samples_us.last().copied().unwrap_or_default() as f64 / 1_000.0;
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
            "scaleMax": index::MAX_DECISION_RULES,
            "generatedUnder": args.scale.map(|_| SCALE_DIR),
            "notes": notes,
            "executableRules": executable_rules,
            "indexRebuildMs": rebuild_ms,
            "gateMs": gate_p95_ms,
            "gateIterations": GATE_SAMPLE_ITERATIONS,
            "gateP50Ms": gate_p50_ms,
            "gateP95Ms": gate_p95_ms,
            "gateMaxMs": gate_max_ms,
            "gateProbe": {
                "outcome": gate_probe.outcome,
                "matchKind": gate_probe.match_kind,
                "candidateCollision": gate_probe.candidate_collision
            },
            "unavailableChoiceProbe": {
                "outcome": unavailable_choice_probe.outcome,
                "matchKind": unavailable_choice_probe.match_kind,
                "candidateCollision": unavailable_choice_probe.candidate_collision,
                "matchedPolicies": unavailable_choice_probe.matched_policies
            },
            "contextFastMs": context_ms,
            "captureMs": capture_ms,
            "ftsMs": fts_ms,
            "ftsResults": fts.len(),
            "query": args.query,
            "embeddings": vector,
            "daemonGateMs": null,
            "memoryMb": null,
            "candidateBounds": {
                "queryTerms": index::MAX_DECISION_QUERY_TERMS,
                "requestOptions": index::MAX_DECISION_OPTIONS,
                "rowsPerTerm": index::MAX_DECISION_ROWS_PER_TERM,
                "executableRules": index::MAX_DECISION_RULES,
                "unavailableChoices": index::MAX_DECISION_UNAVAILABLE_CHOICES,
                "maximumExactRows": index::MAX_DECISION_EXACT_ROWS,
                "maximumFuzzyRowsScored": index::MAX_DECISION_FUZZY_ROWS,
                "retrieval": "actual-rule-term-postings"
            },
            "host": {
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "logicalCpus": std::thread::available_parallelism().map(usize::from).unwrap_or(1),
                "optimized": !cfg!(debug_assertions),
                "brainmapVersion": env!("CARGO_PKG_VERSION")
            },
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

fn percentile_ms(samples_us: &[u128], percentile: usize) -> f64 {
    if samples_us.is_empty() {
        return 0.0;
    }
    let index = ((samples_us.len() * percentile).div_ceil(100)).saturating_sub(1);
    samples_us[index.min(samples_us.len() - 1)] as f64 / 1_000.0
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
    if count == 0 || count > index::MAX_DECISION_RULES {
        bail!(
            "--scale must be between 1 and {}",
            index::MAX_DECISION_RULES
        );
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
    let (situation, options, chosen, rejected) = if i == 0 {
        (
            "Choose formatter database benchmark fallback alpha beta gamma".into(),
            vec!["rustfmt".into(), "z custom formatter".into()],
            "z custom formatter".into(),
            vec!["rustfmt".into()],
        )
    } else if (1..=8).contains(&i) {
        let prefix = char::from(b'a' + (i - 1) as u8);
        let absent_choice = format!("{prefix} absent formatter");
        (
            format!("Choose formatter unrelated omega decoy {i:02}"),
            vec!["rustfmt".into(), absent_choice.clone()],
            absent_choice,
            vec!["rustfmt".into()],
        )
    } else if (9..=137).contains(&i) {
        (
            "Choose database formatter benchmark fallback alpha beta gamma".into(),
            vec!["rustfmt".into(), "dprint".into()],
            "rustfmt".into(),
            vec!["dprint".into()],
        )
    } else {
        let prefer_markdown = i.is_multiple_of(2);
        let chosen = if prefer_markdown {
            "Markdown+JSONL"
        } else {
            "SQLite"
        };
        let rejected = if prefer_markdown {
            vec!["SQLite".into(), "External Vector DB".into()]
        } else {
            vec!["Markdown+JSONL".into(), "External Vector DB".into()]
        };
        (
            format!("Choose benchmark storage local deterministic {i:05}"),
            vec![
                "Markdown+JSONL".into(),
                "SQLite".into(),
                "External Vector DB".into(),
            ],
            chosen.into(),
            rejected,
        )
    };
    let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
        situation,
        options,
        decision_type: Some("architecture".into()),
        scope: Some("global".into()),
        chosen,
        rejected,
    })?;
    let link = if i == 0 {
        String::new()
    } else {
        format!(
            "\nRelated precedent: [[bench-decision-{:05}]].",
            i.saturating_sub(1)
        )
    };
    Ok(format!(
        "{}# Bench Decision {:05}\n\n## Policy\n\nPrefer local-first decisions, embedded SQLite, deterministic gates, and reversible defaults for personal tooling.\n\n## Deterministic Rule\n\n{marker}\n\n## Signals\n\nThis synthetic note exercises executable posting retrieval, full-text search, graph links, and local 256-dimensional note embeddings for production scale checks.{link}\n",
        markdown::frontmatter(id, "decision-example", "reversible-auto", "personal"),
        i,
    ))
}

fn indexed_note_count(root: &Path) -> Result<usize> {
    let conn = index::connection(root)?;
    let count: i64 = conn.query_row("select count(*) from notes", params![], |row| row.get(0))?;
    Ok(count as usize)
}

fn indexed_rule_count(root: &Path) -> Result<usize> {
    let conn = index::connection(root)?;
    let count: i64 = conn.query_row("select count(*) from decision_rules", params![], |row| {
        row.get(0)
    })?;
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
        assert!(
            markdown::parse_decision_rule_result(&note.body)
                .unwrap()
                .is_some()
        );
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
