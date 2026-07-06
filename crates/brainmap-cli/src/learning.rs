use crate::cli::{
    ApplyArgs, BuildArgs, CalibrateArgs, CaptureArgs, ExtractArgs, LearnDecisionArgs,
    LearnFeedbackArgs, PruneImportsArgs, RecordDecisionArgs,
};
use crate::{index, privacy, util, vault};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const INTERVIEW_QUESTIONS: &[InterviewQuestion] = &[
    InterviewQuestion {
        prompt: "What should future agents understand about how you decide that they usually miss?",
        options: &[
            "A. Prefer reversible local changes first.",
            "B. Ask before high-risk or irreversible actions.",
            "C. Optimize for speed unless privacy or data loss is involved.",
        ],
        free_text: "Describe the decision habit in your own words.",
    },
    InterviewQuestion {
        prompt: "What should this decision engine help with most?",
        options: &[
            "A. Coding and architecture choices.",
            "B. Tool/model/workflow choices.",
            "C. Privacy, approval, and risk boundaries.",
        ],
        free_text: "List another decision area or rank the options.",
    },
    InterviewQuestion {
        prompt: "What should never be stored or inferred?",
        options: &[
            "A. Secrets, credentials, tokens, cookies, and private keys.",
            "B. Raw project archives, raw transcripts, and copied source blobs.",
            "C. Personal facts not needed for decision policy.",
        ],
        free_text: "Add any extra hard-no storage rule.",
    },
    InterviewQuestion {
        prompt: "When should an agent ask instead of acting?",
        options: &[
            "A. Ask immediately for irreversible or account-impacting actions.",
            "B. Batch low-risk clarifications and proceed with reversible defaults.",
            "C. Make a conservative guess when confidence is high and rollback is easy.",
        ],
        free_text: "Describe the boundary between guessing and asking.",
    },
    InterviewQuestion {
        prompt: "What details should be evidence only and discarded after extracting the decision pattern?",
        options: &[
            "A. File names, code snippets, command output, and implementation facts.",
            "B. One-off project context that does not express a reusable preference.",
            "C. Temporary debugging traces and logs.",
        ],
        free_text: "Name any evidence type that should never become a policy note.",
    },
    InterviewQuestion {
        prompt: "What makes Brainmap useful instead of heavy or noisy?",
        options: &[
            "A. Only strong repeated preferences become durable policy.",
            "B. Weak signals stay pending until confirmed.",
            "C. The engine asks fewer, better questions with clear options.",
        ],
        free_text: "Describe what would feel noisy or overbearing.",
    },
    InterviewQuestion {
        prompt: "Which decisions can the harness make automatically?",
        options: &[
            "A. Low-risk reversible workflow defaults.",
            "B. Local-only implementation choices within existing policy.",
            "C. None unless the user explicitly approved the category.",
        ],
        free_text: "List decisions that always require your approval.",
    },
];

struct InterviewQuestion {
    prompt: &'static str,
    options: &'static [&'static str],
    free_text: &'static str,
}

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
            println!(
                "AgentMemory can seed Brainmap, but it never replaces required calibration. Ask these when confidence or coverage is missing:"
            );
            print_questions(args.questions);
        }
        "interview" => {
            if args.dry_run {
                print_questions(args.questions);
            } else {
                fs::create_dir_all(root.join("99-meta/pending-update-packets"))?;
                for (i, question) in INTERVIEW_QUESTIONS.iter().take(args.questions).enumerate() {
                    let human_question = question_text(question);
                    let packet = packet(
                        "interactive",
                        "calibration-question",
                        question.prompt,
                        "weak",
                        "personal",
                        "ask",
                        Some(human_question),
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
            println!(
                "AgentMemory source is seed context only; required calibration still needs questions with options and free text:"
            );
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
                println!("AgentMemory import is incomplete until calibration gaps are answered:");
                print_questions(args.questions);
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
        println!("{}. {}", i + 1, question.prompt);
        println!("   Options:");
        for option in question.options {
            println!("   - {option}");
        }
        println!("   Free text: {}", question.free_text);
    }
}

fn question_text(question: &InterviewQuestion) -> String {
    format!(
        "{} Options: {} Free text: {}",
        question.prompt,
        question.options.join(" "),
        question.free_text
    )
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
        if !is_decision_signal(&text) {
            continue;
        }
        let lower = text.to_lowercase();
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

fn is_decision_signal(text: &str) -> bool {
    let lower = text.to_lowercase();
    if lower.split_whitespace().count() < 5 || looks_like_code_or_knowledge_fact(&lower) {
        return false;
    }

    [
        "user chose",
        "user rejected",
        "user corrected",
        "user preferred",
        "user refused",
        "future agents should",
        "agents should",
        "agent should",
        "brainmap should",
        "harness should",
        "should ask",
        "should not ask",
        "ask me",
        "don't ask me",
        "do not ask me",
        "requires approval",
        "approval required",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || (has_decision_frame(&lower) && has_decision_action(&lower))
}

fn looks_like_code_or_knowledge_fact(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.starts_with("--")
        || trimmed.starts_with('"')
        || trimmed.contains("\"--")
        || trimmed.contains("`--")
        || trimmed.contains(" on github ")
        || trimmed.contains("stack trace")
        || trimmed.contains(" must be inspected")
        || trimmed.contains(" defaults to ")
        || trimmed.contains(" implements ")
        || trimmed.contains("user chose to inspect")
        || trimmed.contains("user chose to examine")
        || trimmed.contains("user chose to verify")
        || trimmed.contains("user wants to verify")
        || trimmed.contains("user wants to check")
        || trimmed.contains("using a terminal command")
        || trimmed.contains("implementation details")
        || trimmed.contains("source files")
        || trimmed.contains("bearer key")
        || trimmed.contains("env-var")
        || trimmed.contains("main.go")
        || trimmed.contains("line ")
        || trimmed.contains("b.rate")
        || trimmed.contains("cfg ")
        || trimmed.contains("::")
        || trimmed.contains(".rs")
        || trimmed.contains(".ts")
        || trimmed.contains(".tsx")
        || trimmed.contains(".py")
    {
        return true;
    }

    lower.matches('|').count() >= 3 || lower.matches('\\').count() >= 2
}

fn has_decision_frame(lower: &str) -> bool {
    lower.starts_with("when ")
}

fn has_decision_action(lower: &str) -> bool {
    [
        " choose ",
        " chose ",
        " prefer ",
        " preferred ",
        " default to ",
        " ask ",
        " approval ",
        " reject ",
        " rejected ",
        " never ",
        " must ",
        " should ",
        " tradeoff",
        " restriction",
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

pub fn prune_imports(args: PruneImportsArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let dir = root.join("60-decision-examples");
    let mut plans = Vec::new();
    let mut kept = 0usize;

    if dir.exists() {
        let mut paths = fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("md"))
            .filter(|path| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|stem| stem.starts_with("upd_"))
            })
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            let rel = path.strip_prefix(&root)?.to_path_buf();
            let text = fs::read_to_string(&path)?;
            let signal_text = note_decision_text(&rel, &text);
            if is_decision_signal(&signal_text) {
                kept += 1;
                continue;
            }
            let bytes = fs::read(&path)?;
            plans.push(ArchivePlan {
                rel,
                sha256: util::sha256_hex(&bytes),
                reason: "knowledge-like AgentMemory import".into(),
            });
        }
    }

    if args.dry_run || !args.yes {
        println!(
            "would archive {} knowledge-like import note(s); would keep {} decision-like import note(s)",
            plans.len(),
            kept
        );
        for plan in plans.iter().take(10) {
            println!("- {}", plan.rel.display());
        }
        return Ok(());
    }

    if plans.is_empty() {
        println!("archived 0 knowledge-like import note(s); kept {kept}");
        return Ok(());
    }

    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let archive_root = root.join("99-meta/archived-knowledge-imports").join(&stamp);
    let mut entries = Vec::new();
    for plan in &plans {
        let from = root.join(&plan.rel);
        let to = archive_root.join(&plan.rel);
        util::ensure_parent(&to)?;
        fs::rename(&from, &to)?;
        entries.push(ArchiveEntry {
            original_path: plan.rel.to_string_lossy().to_string(),
            archive_path: to
                .strip_prefix(&root)
                .unwrap_or(&to)
                .to_string_lossy()
                .to_string(),
            sha256: plan.sha256.clone(),
            reason: plan.reason.clone(),
        });
    }

    let manifest = ArchiveManifest {
        created_at: util::now_iso(),
        archived_count: entries.len(),
        kept_count: kept,
        entries,
    };
    util::write_atomic(
        &archive_root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?.as_slice(),
    )?;
    index::rebuild(&root)?;
    println!(
        "archived {} knowledge-like import note(s); kept {}; manifest {}",
        manifest.archived_count,
        manifest.kept_count,
        archive_root.join("manifest.json").display()
    );
    Ok(())
}

struct ArchivePlan {
    rel: PathBuf,
    sha256: String,
    reason: String,
}

#[derive(Serialize)]
struct ArchiveManifest {
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "archivedCount")]
    archived_count: usize,
    #[serde(rename = "keptCount")]
    kept_count: usize,
    entries: Vec<ArchiveEntry>,
}

#[derive(Serialize)]
struct ArchiveEntry {
    #[serde(rename = "originalPath")]
    original_path: String,
    #[serde(rename = "archivePath")]
    archive_path: String,
    sha256: String,
    reason: String,
}

fn note_decision_text(rel: &Path, text: &str) -> String {
    if let Some(note) = crate::markdown::parse_note(rel.to_path_buf(), text) {
        format!("{} {}", note.title, note.body)
    } else {
        text.to_string()
    }
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
        let packet = fs::read_dir(root.join("99-meta/pending-update-packets"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let packet_text = fs::read_to_string(packet.path()).unwrap();
        assert!(packet_text.contains("Options:"));
        assert!(packet_text.contains("Free text:"));
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
                    {"text": "When building local v1 tools, default to Markdown and SQLite unless scale proves otherwise."},
                    {"text": "Future agents should ask before irreversible deletion."},
                    {"text": "\"--severity\" defaults to Severity.INFO and accepts BLOCKER, CRITICAL, MAJOR, MINOR, INFO"},
                    {"text": "$bL.Scale must be inspected to ensure it is never <= 0"},
                    {"text": "\"sebschmi/deterministic-default-hasher\" on GitHub implements deterministic default hashing"},
                    {"text": "(--config|profile|target|release) are accepted CLI flags"},
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
        assert_eq!(count, 3);
    }

    #[test]
    fn signal_filter_rejects_knowledge_facts() {
        assert!(is_decision_signal(
            "User chose Markdown plus SQLite and rejected a complex index."
        ));
        assert!(is_decision_signal(
            "When building local v1 tools, default to Markdown and SQLite unless scale proves otherwise."
        ));
        assert!(is_decision_signal(
            "Future agents should prefer questions with clear options when confidence is low."
        ));
        assert!(!is_decision_signal(
            "\"--severity\" defaults to Severity.INFO and accepts BLOCKER, CRITICAL, MAJOR, MINOR, INFO"
        ));
        assert!(!is_decision_signal(
            "$bL.Scale must be inspected to ensure it is never <= 0"
        ));
        assert!(!is_decision_signal(
            "\"sebschmi/deterministic-default-hasher\" on GitHub implements deterministic default hashing"
        ));
        assert!(!is_decision_signal(
            "(--config|profile|target|release) are accepted CLI flags"
        ));
        assert!(!is_decision_signal(
            "App.tsx mounts only ServicesView at line 26; LogsView and its hook are never rendered, so no user can trigger the dead code path"
        ));
        assert!(!is_decision_signal(
            "After confirming that tests succeed, the user plans to introduce a parsePaging helper and clamp logic for paging restrictions."
        ));
        assert!(!is_decision_signal(
            "Agent performs a comprehensive audit of the plugin implementation and verifies CLI command coverage."
        ));
        assert!(!is_decision_signal(
            "User wants to verify that env-var defaults are identical across both reading paths."
        ));
        assert!(!is_decision_signal(
            "The user chose to inspect the design system before implementation."
        ));
    }

    #[test]
    fn prune_imports_archives_knowledge_like_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        write_import_note(
            &root,
            "upd_noise",
            "\"--severity\" defaults to Severity.INFO and accepts BLOCKER, CRITICAL, MAJOR, MINOR, INFO",
        );
        write_import_note(
            &root,
            "upd_decision",
            "User chose Markdown plus SQLite and rejected a complex index.",
        );

        prune_imports(PruneImportsArgs {
            dry_run: false,
            yes: true,
            vault: Some(root.clone()),
        })
        .unwrap();

        assert!(!root.join("60-decision-examples/upd_noise.md").exists());
        assert!(root.join("60-decision-examples/upd_decision.md").exists());
        let archives = fs::read_dir(root.join("99-meta/archived-knowledge-imports"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(archives.len(), 1);
        let archive_root = archives[0].path();
        assert!(
            archive_root
                .join("60-decision-examples/upd_noise.md")
                .exists()
        );
        let manifest = fs::read_to_string(archive_root.join("manifest.json")).unwrap();
        assert!(manifest.contains("\"archivedCount\": 1"));
        assert!(manifest.contains("\"sha256\""));
        let live_paths = vault::load_notes(&root)
            .unwrap()
            .into_iter()
            .map(|note| note.path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();
        assert!(
            live_paths
                .iter()
                .any(|path| path == "60-decision-examples/upd_decision.md")
        );
        assert!(
            !live_paths
                .iter()
                .any(|path| path.contains("archived-knowledge-imports"))
        );
    }

    #[test]
    fn autopilot_promotion_requires_evidence() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let err = autopilot_promote(Some(root), "conservative").unwrap_err();
        assert!(err.to_string().contains("promotion denied"));
    }

    fn write_import_note(root: &Path, stem: &str, title: &str) {
        let body = format!(
            "{}# {}\n\n## Claim\n\n{}\n",
            crate::markdown::frontmatter(stem, "decision-example", "ask-before-action", "personal"),
            title,
            title
        );
        util::write_atomic(
            &root.join("60-decision-examples").join(format!("{stem}.md")),
            body.as_bytes(),
        )
        .unwrap();
    }
}
