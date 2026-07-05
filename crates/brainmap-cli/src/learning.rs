use crate::cli::{
    ApplyArgs, BuildArgs, CalibrateArgs, CaptureArgs, ExtractArgs, LearnDecisionArgs,
    LearnFeedbackArgs, RecordDecisionArgs,
};
use crate::{index, privacy, util, vault};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const INTERVIEW_QUESTIONS: &[&str] = &[
    "What should future agents understand about how you decide that they usually miss?",
    "What should this decision engine help with: coding choices, design choices, model/tool choices, workflow choices, privacy boundaries, time decisions, or something else?",
    "What should never be stored or inferred?",
    "When should an agent ask immediately, batch questions, or make a reversible guess?",
    "What kinds of details should be treated only as evidence and discarded after extracting the decision pattern?",
    "What makes a system feel useful instead of heavy or noisy?",
    "Which decisions can lightweight models/harnesses make automatically, and which require your approval?",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePacket {
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub source: serde_json::Value,
    pub classification: String,
    pub claim: String,
    pub evidence: Vec<serde_json::Value>,
    #[serde(rename = "targetNotes")]
    pub target_notes: Vec<String>,
    #[serde(rename = "suggestedLinks")]
    pub suggested_links: Vec<String>,
    pub confidence: f64,
    pub sensitivity: String,
    pub action: String,
    #[serde(rename = "humanQuestion")]
    pub human_question: Option<String>,
    pub status: String,
}

pub fn build_decision_engine(args: BuildArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    match args.mode.as_str() {
        "auto" => {
            println!("AgentMemory optional: unavailable or disabled; using interview fallback.");
            print_questions(args.questions);
        }
        "interview" => {
            if args.dry_run {
                print_questions(args.questions);
            } else {
                fs::create_dir_all(root.join("99-meta/pending-update-packets"))?;
                for (i, question) in INTERVIEW_QUESTIONS.iter().take(args.questions).enumerate() {
                    let packet = packet(
                        "interactive",
                        "calibration-question",
                        question,
                        "weak",
                        "personal",
                        "ask",
                        Some((*question).to_string()),
                    );
                    write_packet(&root, &format!("interview-{i}"), &packet)?;
                }
                println!(
                    "created {} interview update packets",
                    args.questions.min(INTERVIEW_QUESTIONS.len())
                );
            }
        }
        "agentmemory" | "agentmemory-mcp" => {
            println!("AgentMemory source unavailable; fallback questions:");
            print_questions(args.questions);
        }
        "export" => {
            if let Some(file) = args.file {
                let signals = parse_agentmemory_export(&file)?;
                if args.dry_run {
                    println!(
                        "would parse AgentMemory export {}: {} decision signal(s)",
                        file.display(),
                        signals.len()
                    );
                    println!("project/code details discarded; decision traces only");
                } else {
                    let mut created = 0usize;
                    for signal in signals {
                        let packet = packet(
                            "agentmemory-export",
                            &signal.classification,
                            &signal.claim,
                            &signal.strength,
                            &signal.sensitivity,
                            "create",
                            None,
                        );
                        if packet.sensitivity != "secret" {
                            write_packet(&root, "agentmemory-export", &packet)?;
                            created += 1;
                        }
                    }
                    println!("created {created} AgentMemory export update packet(s)");
                }
            } else {
                println!("export mode needs --file; no mutation performed");
            }
        }
        "manual" | "current-session" => {
            print_questions(args.questions);
        }
        other => bail!("unsupported build mode: {other}"),
    }
    Ok(())
}

fn print_questions(n: usize) {
    for (i, question) in INTERVIEW_QUESTIONS.iter().take(n).enumerate() {
        println!("{}. {}", i + 1, question);
    }
}

#[derive(Debug, Clone)]
struct BrainmapSignal {
    classification: String,
    claim: String,
    strength: String,
    sensitivity: String,
}

fn parse_agentmemory_export(path: &Path) -> Result<Vec<BrainmapSignal>> {
    let text = fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&text)?;
    let mut strings = Vec::new();
    collect_strings(&json, &mut strings);
    let mut signals = Vec::new();
    for text in strings {
        if should_discard_signal(&text) {
            continue;
        }
        let lower = text.to_lowercase();
        if !is_decision_signal(&lower) {
            continue;
        }
        let redacted = privacy::redact(&text);
        let sensitivity = privacy::sensitivity(&text).to_string();
        if sensitivity == "secret" {
            continue;
        }
        signals.push(BrainmapSignal {
            classification: classify_signal(&lower).into(),
            claim: compact(&redacted),
            strength: signal_strength(&lower).into(),
            sensitivity,
        });
    }
    signals.sort_by(|a, b| a.claim.cmp(&b.claim));
    signals.dedup_by(|a, b| a.claim == b.claim);
    Ok(signals)
}

fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_strings(value, out);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values() {
                collect_strings(value, out);
            }
        }
        _ => {}
    }
}

fn should_discard_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    privacy::contains_secret(text)
        || lower.contains("```")
        || lower.contains("diff --git")
        || lower.contains("cargo run")
        || lower.contains("git status")
        || lower.contains("stack trace")
        || lower.contains("src/")
        || lower.contains("target/")
        || lower.contains("node_modules")
}

fn is_decision_signal(lower: &str) -> bool {
    [
        "user chose",
        "user rejected",
        "user corrected",
        "user preferred",
        "user refused",
        "should ask",
        "should not ask",
        "ask me",
        "don't ask me",
        "approval",
        "tradeoff",
        "restriction",
        "default",
        "prefer",
        "never",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn classify_signal(lower: &str) -> &'static str {
    if lower.contains("corrected") || lower.contains("correction") {
        "corrected-decision"
    } else if lower.contains("rejected") || lower.contains("refused") {
        "rejection-pattern"
    } else if lower.contains("ask") || lower.contains("approval") {
        "ask-trigger"
    } else if lower.contains("never") || lower.contains("restriction") {
        "hard-constraint"
    } else if lower.contains("tradeoff") {
        "tradeoff-rule"
    } else {
        "decision-example"
    }
}

fn signal_strength(lower: &str) -> &'static str {
    if lower.contains("corrected") || lower.contains("correction") || lower.contains("refused") {
        "strong"
    } else {
        "medium"
    }
}

pub fn record_decision(args: RecordDecisionArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let id = args
        .decision_id
        .unwrap_or_else(|| util::id("dec", "manual"));
    util::append_jsonl(
        &root.join("90-calibration/decision-ledger.jsonl"),
        &json!({
            "id": id,
            "createdAt": util::now_iso(),
            "kind": "record-decision",
            "chosen": args.chosen,
            "wasAsked": args.was_asked.unwrap_or(false),
            "evidenceStrength": if args.was_asked.unwrap_or(false) { "medium" } else { "weak" }
        }),
    )?;
    println!("recorded decision");
    Ok(())
}

pub fn learn_feedback(args: LearnFeedbackArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let redacted = privacy::redact(&args.correction);
    let packet = packet(
        "harness",
        "corrected-decision",
        &redacted,
        "very-strong",
        privacy::sensitivity(&args.correction),
        "create",
        None,
    );
    if packet.sensitivity == "secret" {
        println!("secret feedback rejected/redacted; no packet created");
        return Ok(());
    }
    write_packet(&root, &args.decision_id, &packet)?;
    println!("created high-strength update packet {}", packet.id);
    Ok(())
}

pub fn learn_decision(args: LearnDecisionArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let claim = format!(
        "When {}, choose {}; rejected {}; rationale {}",
        args.situation,
        args.chosen,
        args.rejected.unwrap_or_else(|| "none recorded".into()),
        args.rationale.unwrap_or_else(|| "not supplied".into())
    );
    let redacted = privacy::redact(&claim);
    let packet = packet(
        "manual",
        "decision-example",
        &redacted,
        "strong",
        privacy::sensitivity(&claim),
        "create",
        None,
    );
    if packet.sensitivity == "secret" {
        println!("secret decision rejected/redacted; no packet created");
        return Ok(());
    }
    write_packet(&root, "manual-decision", &packet)?;
    println!("created decision update packet {}", packet.id);
    Ok(())
}

pub fn capture(args: CaptureArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let mut text = args.text.unwrap_or_default();
    if args.stdin {
        io::stdin().read_to_string(&mut text)?;
    }
    capture_text(&root, &text, &args.source)?;
    println!("captured");
    Ok(())
}

pub(crate) fn capture_text(root: &Path, text: &str, source: &str) -> Result<()> {
    let redacted = privacy::redact(text);
    util::append_jsonl(
        &root.join(".brainmap/capture-queue.jsonl"),
        &json!({
            "id": util::id("cap", &redacted),
            "createdAt": util::now_iso(),
            "source": source,
            "text": compact(&redacted),
            "sensitivity": privacy::sensitivity(text)
        }),
    )?;
    Ok(())
}

pub fn extract(args: ExtractArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let queue = root.join(".brainmap/capture-queue.jsonl");
    if args.from_queue && queue.exists() {
        let text = fs::read_to_string(&queue)?;
        let count = text.lines().filter(|l| !l.trim().is_empty()).count();
        println!("queued capture events: {count}");
    } else {
        println!("nothing to extract; use --from-queue after capture");
    }
    Ok(())
}

pub fn apply(args: ApplyArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let dir = root.join("99-meta/pending-update-packets");
    fs::create_dir_all(&dir)?;
    let mut applied = 0usize;
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let packet: UpdatePacket = serde_json::from_str(&text)?;
        if packet.sensitivity == "secret" {
            continue;
        }
        if args.dry_run || (!args.yes && !args.pending) {
            println!("would apply {}", packet.id);
            continue;
        }
        let target = root
            .join("60-decision-examples")
            .join(format!("{}.md", packet.id));
        let body = format!(
            "{}# {}\n\n## Claim\n\n{}\n\n## Evidence\n\n- {}\n\n## Links\n\n{}\n",
            crate::markdown::frontmatter(
                &packet.id,
                &packet.classification,
                "ask-before-action",
                &packet.sensitivity
            ),
            packet.claim,
            packet.claim,
            packet
                .evidence
                .first()
                .and_then(|v| v.get("quoteOrSummary"))
                .and_then(|v| v.as_str())
                .unwrap_or("packet evidence"),
            packet.suggested_links.join("\n")
        );
        util::write_atomic(&target, body.as_bytes())?;
        fs::rename(&path, path.with_extension("applied.json"))?;
        applied += 1;
    }
    if applied > 0 {
        index::rebuild(&root)?;
    }
    println!("applied {applied} packet(s)");
    Ok(())
}

pub fn calibrate(args: CalibrateArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let questions = [
        "When building a v1 local personal tool, choose one: A. Simple files plus SQLite index first. B. Full database/vector system first. Which feels more like your decision style and why?",
        "For uncertain reversible work, choose one: A. Make a conservative guess. B. Ask immediately. Why?",
        "For privacy-sensitive work, choose one: A. Ask before action. B. Decide from history. Why?",
    ];
    for q in questions.iter().cycle().take(args.n) {
        println!("- [{}] {q}", args.topic);
    }
    println!("vault: {}", root.display());
    Ok(())
}

pub fn autopilot_status(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let cfg = read_config(&root);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "mode": cfg.0,
            "threshold": cfg.2,
            "level": cfg.1,
            "killSwitch": std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() == Some("1")
        }))?
    );
    Ok(())
}

pub fn autopilot_set(
    vault: Option<PathBuf>,
    mode: &str,
    level: &str,
    threshold: Option<f64>,
) -> Result<()> {
    let root = vault::resolve_vault(vault);
    fs::create_dir_all(root.join(".brainmap"))?;
    let threshold = threshold.unwrap_or(0.82);
    util::write_atomic(
        &root.join(".brainmap/autopilot.json"),
        serde_json::to_vec_pretty(
            &json!({ "mode": mode, "level": level, "threshold": threshold }),
        )?
        .as_slice(),
    )?;
    println!("autopilot: mode={mode} level={level} threshold={threshold}");
    Ok(())
}

pub fn autopilot_promote(vault: Option<PathBuf>, to: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let stats = autopilot_stats(&root)?;
    match to {
        "shadow" => autopilot_set(Some(root), "shadow", "conservative", None),
        "conservative" => {
            if stats.decisions < 30 || stats.serious_mismatches >= 2 || stats.privacy_violations > 0
            {
                bail!(
                    "promotion denied: need >=30 shadow decisions, <2 serious mismatches, 0 privacy/hard-rule violations; got decisions={}, mismatches={}, violations={}",
                    stats.decisions,
                    stats.serious_mismatches,
                    stats.privacy_violations
                );
            }
            autopilot_set(Some(root), "conservative", "conservative", None)
        }
        "balanced" => {
            if stats.decisions < 100 || stats.false_proceeds > 0 || stats.privacy_violations > 0 {
                bail!(
                    "promotion denied: balanced requires >=100 decisions, 0 false proceeds in MVP, 0 privacy/hard-rule violations"
                );
            }
            autopilot_set(Some(root), "balanced", "balanced", None)
        }
        "aggressive" => bail!("promotion denied: aggressive autopilot is never automatic"),
        other => bail!("unknown autopilot target: {other}"),
    }
}

#[derive(Debug, Default)]
struct AutopilotStats {
    decisions: usize,
    serious_mismatches: usize,
    privacy_violations: usize,
    false_proceeds: usize,
}

fn autopilot_stats(root: &Path) -> Result<AutopilotStats> {
    let ledger = root.join("90-calibration/decision-ledger.jsonl");
    let Ok(text) = fs::read_to_string(ledger) else {
        return Ok(AutopilotStats::default());
    };
    let mut stats = AutopilotStats::default();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        stats.decisions += 1;
        let lower = line.to_lowercase();
        if lower.contains("serious mismatch") || lower.contains("corrected-decision") {
            stats.serious_mismatches += 1;
        }
        if lower.contains("privacy violation") || lower.contains("hard-rule violation") {
            stats.privacy_violations += 1;
        }
        if lower.contains("false proceed") {
            stats.false_proceeds += 1;
        }
    }
    Ok(stats)
}

pub fn gate_mode(vault: Option<PathBuf>, mode: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    fs::create_dir_all(root.join(".brainmap"))?;
    util::write_atomic(&root.join(".brainmap/gate-mode"), mode.as_bytes())?;
    println!("gate mode: {mode}");
    Ok(())
}

pub fn review(vault: Option<PathBuf>, cadence: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    println!(
        "review {cadence}: deterministic review queued for {}",
        root.display()
    );
    Ok(())
}

pub fn dream(vault: Option<PathBuf>, mode: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    if mode == "deep" {
        println!("dream-deep requires explicit harness approval; pending packets only");
    } else {
        println!(
            "dream-{mode}: scan for duplicates, contradictions, stale policies, missing approval rules"
        );
    }
    println!("vault: {}", root.display());
    Ok(())
}

fn packet(
    source_kind: &str,
    classification: &str,
    claim: &str,
    strength: &str,
    sensitivity: &str,
    action: &str,
    human_question: Option<String>,
) -> UpdatePacket {
    let id = util::id("upd", claim);
    UpdatePacket {
        id,
        created_at: util::now_iso(),
        source: json!({ "kind": source_kind, "confidence": 0.7 }),
        classification: classification.into(),
        claim: claim.into(),
        evidence: vec![
            json!({ "quoteOrSummary": claim, "sourceRef": source_kind, "strength": strength }),
        ],
        target_notes: vec!["[[20-decision-frames/learning-decisions.md]]".into()],
        suggested_links: vec!["[[70-question-triggers/ask-when-uncertain.md]]".into()],
        confidence: if strength == "very-strong" { 0.95 } else { 0.7 },
        sensitivity: sensitivity.into(),
        action: action.into(),
        human_question,
        status: "pending".into(),
    }
}

fn write_packet(root: &Path, stem: &str, packet: &UpdatePacket) -> Result<()> {
    fs::create_dir_all(root.join("99-meta/pending-update-packets"))?;
    let path = root
        .join("99-meta/pending-update-packets")
        .join(format!("{stem}-{}.json", packet.id));
    util::write_atomic(&path, serde_json::to_vec_pretty(packet)?.as_slice())
}

fn compact(text: &str) -> String {
    text.split_whitespace()
        .take(200)
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_config(root: &Path) -> (String, String, f64) {
    let path = root.join(".brainmap/autopilot.json");
    let Ok(text) = fs::read_to_string(path) else {
        return ("shadow".into(), "conservative".into(), 0.82);
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return ("shadow".into(), "conservative".into(), 0.82);
    };
    (
        json.get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("shadow")
            .into(),
        json.get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("conservative")
            .into(),
        json.get("threshold")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.82),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_secret_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        learn_feedback(LearnFeedbackArgs {
            decision_id: "dec_1".into(),
            correction: "api_key=abcdef1234567890".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        let count = fs::read_dir(root.join("99-meta/pending-update-packets"))
            .unwrap()
            .count();
        assert_eq!(count, 0);
    }

    #[test]
    fn interview_creates_packets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        build_decision_engine(BuildArgs {
            mode: "interview".into(),
            vault: Some(root.clone()),
            questions: 2,
            dry_run: false,
            file: None,
        })
        .unwrap();
        assert!(
            fs::read_dir(root.join("99-meta/pending-update-packets"))
                .unwrap()
                .count()
                >= 2
        );
    }

    #[test]
    fn agentmemory_export_creates_decision_packets_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let export = tmp.path().join("agentmemory.json");
        fs::write(
            &export,
            serde_json::json!({
                "memories": [
                    {"text": "User chose Markdown plus SQLite and rejected external vector DB."},
                    {"text": "cargo run -- build project chronology"},
                    {"text": "api_key=abcdef1234567890"}
                ]
            })
            .to_string(),
        )
        .unwrap();
        build_decision_engine(BuildArgs {
            mode: "export".into(),
            vault: Some(root.clone()),
            questions: 7,
            dry_run: false,
            file: Some(export),
        })
        .unwrap();
        let count = fs::read_dir(root.join("99-meta/pending-update-packets"))
            .unwrap()
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn autopilot_promotion_requires_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let err = autopilot_promote(Some(root), "conservative").unwrap_err();
        assert!(err.to_string().contains("promotion denied"));
    }
}
