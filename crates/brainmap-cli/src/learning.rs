use crate::cli::{
    ApplyArgs, BuildArgs, CalibrateArgs, CaptureArgs, ExtractArgs, LearnDecisionArgs,
    LearnFeedbackArgs, PruneImportsArgs, RecordDecisionArgs,
};
use crate::{index, markdown, privacy, util, vault};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
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
#[serde(deny_unknown_fields)]
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
    #[serde(
        default,
        rename = "decisionRule",
        skip_serializing_if = "Option::is_none"
    )]
    pub decision_rule: Option<markdown::DecisionRule>,
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
    record_decision_quiet(args)?;
    println!("recorded decision");
    Ok(())
}

pub(crate) fn record_decision_quiet(args: RecordDecisionArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let mut ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    let decision_id = if let Some(id) = args.decision_id {
        validate_decision_id(&id)?;
        Some(id)
    } else {
        None
    };
    let active_run = crate::dogfood::active_run_context(&root)?;
    if active_run.is_some() && decision_id.is_none() {
        bail!("an active dogfood run requires --decision-id for every action record");
    }
    if active_run.is_some() && args.was_asked.is_none() {
        bail!("an active dogfood run requires explicit --was-asked=true or --was-asked=false");
    }
    let event_id = util::id("action", decision_id.as_deref().unwrap_or("manual"));
    let chosen = args
        .chosen
        .map(|value| privacy::redact(&value))
        .filter(|value| !value.trim().is_empty());
    let was_asked = args.was_asked.unwrap_or(false);
    if active_run.is_some() && chosen.is_none() && !was_asked {
        bail!("an active dogfood action record requires --chosen unless --was-asked=true");
    }
    ledger.append(&json!({
        "id": event_id,
        "decisionId": decision_id,
        "createdAt": util::now_iso(),
        "kind": "record-decision",
        "dogfoodRunId": active_run.map(|run| run.run_id),
        "chosen": chosen,
        "wasAsked": was_asked,
        "evidenceStrength": if was_asked { "medium" } else { "weak" }
    }))?;
    Ok(())
}

pub fn learn_feedback(args: LearnFeedbackArgs) -> Result<()> {
    if let Some(packet_id) = learn_feedback_quiet(args)? {
        println!("created high-strength update packet {packet_id}");
    } else {
        println!("secret feedback rejected/redacted; no packet created");
    }
    Ok(())
}

pub(crate) fn parse_rejected_choices(choices: &str) -> Vec<String> {
    choices
        .split('|')
        .map(str::trim)
        .filter(|choice| !choice.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn learn_feedback_quiet(args: LearnFeedbackArgs) -> Result<Option<String>> {
    let rejected = args.rejected.as_deref().map(parse_rejected_choices);
    learn_feedback_quiet_with_rejected(args, rejected)
}

pub(crate) fn learn_feedback_quiet_with_rejected(
    args: LearnFeedbackArgs,
    rejected: Option<Vec<String>>,
) -> Result<Option<String>> {
    let root = vault::resolve_vault(args.vault);
    let mut ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    let active_run = crate::dogfood::active_run_context(&root)?;
    validate_decision_id(&args.decision_id)?;
    if args.correction.is_none() && args.chosen.is_none() {
        bail!("feedback requires either a correction or a chosen option");
    }
    let rejected_summary = rejected
        .as_ref()
        .filter(|choices| !choices.is_empty())
        .map(|choices| choices.join(", "))
        .unwrap_or_else(|| "none".into());
    let raw_feedback = args.correction.clone().unwrap_or_else(|| {
        format!(
            "choose {}; rejected {}",
            args.chosen.as_deref().unwrap_or_default(),
            rejected_summary
        )
    });
    let redacted = privacy::redact(&raw_feedback);
    let mut packet = packet(
        "harness",
        "corrected-decision",
        &redacted,
        "very-strong",
        privacy::sensitivity(&raw_feedback),
        "create",
        None,
    );
    if packet.sensitivity == "secret" {
        return Ok(None);
    }
    let ledger_bytes = ledger.read_all()?;
    let context =
        decision_context_from_bytes(&ledger_bytes, &args.decision_id)?.with_context(|| {
            format!(
                "decision id {} was not found in the gate ledger",
                args.decision_id
            )
        })?;
    validate_feedback_incident(args.incident, &context)?;
    let (chosen, rejected) = if let Some(chosen) = args.chosen {
        (
            privacy::redact(&chosen),
            rejected
                .unwrap_or_default()
                .iter()
                .map(|choice| privacy::redact(choice))
                .collect(),
        )
    } else {
        normalize_feedback_rule(&redacted)
    };
    packet.decision_rule = Some(markdown::DecisionRule {
        situation: context.situation,
        decision_type: Some(context.decision_type),
        scope: Some(context.scope),
        options: context.options,
        chosen,
        rejected,
    });
    let packet_path = write_packet(&root, &args.decision_id, &packet)?;
    let rule = packet
        .decision_rule
        .as_ref()
        .context("feedback packet is missing its decision rule")?;
    if let Err(error) = ledger.append(&json!({
        "id": util::id("feedback", &args.decision_id),
        "decisionId": args.decision_id,
        "createdAt": util::now_iso(),
        "kind": "learn-feedback",
        "dogfoodRunId": active_run.map(|run| run.run_id),
        "packetId": packet.id,
        "chosen": rule.chosen,
        "rejected": rule.rejected,
        "classification": "corrected-decision",
        "incidentType": args.incident.map(crate::cli::FeedbackIncident::as_str)
    })) {
        let _ = fs::remove_file(packet_path);
        return Err(error);
    }
    Ok(Some(packet.id))
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedDecisionUpdate {
    packet: UpdatePacket,
}

impl PreparedDecisionUpdate {
    pub fn preview_value(&self) -> serde_json::Value {
        json!({
            "path": format!(
                "99-meta/pending-update-packets/manual-decision-{}.json",
                self.packet.id
            ),
            "packet": &self.packet
        })
    }

    pub fn write(&self, root: &Path) -> Result<()> {
        write_packet(root, "manual-decision", &self.packet).map(|_| ())
    }
}

pub(crate) fn prepare_decision_update(
    args: LearnDecisionArgs,
) -> Result<Option<PreparedDecisionUpdate>> {
    let scope = util::resolve_learning_scope(&args.scope);
    let rejected = args
        .rejected
        .clone()
        .unwrap_or_else(|| "none recorded".into());
    let rationale = args
        .rationale
        .clone()
        .unwrap_or_else(|| "not supplied".into());
    let claim = format!(
        "When {}, choose {}; rejected {}; rationale {}",
        args.situation, args.chosen, rejected, rationale
    );
    let redacted = privacy::redact(&claim);
    let mut packet = packet(
        "manual",
        "decision-example",
        &redacted,
        "strong",
        privacy::sensitivity(&claim),
        "create",
        None,
    );
    if packet.sensitivity == "secret" {
        return Ok(None);
    }
    packet.decision_rule = Some(markdown::DecisionRule {
        situation: privacy::redact(&args.situation),
        decision_type: Some(privacy::redact(&args.decision_type)),
        scope: Some(privacy::redact(&scope)),
        options: args
            .options
            .split('|')
            .map(str::trim)
            .filter(|option| !option.is_empty())
            .map(privacy::redact)
            .collect(),
        chosen: privacy::redact(&args.chosen),
        rejected: args
            .rejected
            .iter()
            .map(|value| privacy::redact(value))
            .collect(),
    });
    Ok(Some(PreparedDecisionUpdate { packet }))
}

pub fn learn_decision(args: LearnDecisionArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault.clone());
    let Some(prepared) = prepare_decision_update(args)? else {
        println!("secret decision rejected/redacted; no packet created");
        return Ok(());
    };
    let packet_id = prepared.packet.id.clone();
    prepared.write(&root)?;
    println!("created decision update packet {packet_id}");
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
    let redacted_source = privacy::redact(source);
    util::append_jsonl(
        &root.join(".brainmap/capture-queue.jsonl"),
        &json!({
            "id": util::id("cap", &redacted),
            "createdAt": util::now_iso(),
            "source": redacted_source,
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
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    let _lock = util::FileLock::acquire(&root.join(".brainmap/locks"), "update-apply.lock")?;
    let mut ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    let active_run = crate::dogfood::active_run_context(&root)?;
    let dir = root.join("99-meta/pending-update-packets");
    fs::create_dir_all(&dir)?;
    let mut applied_packet_ids = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".applied.json"))
        {
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let raw_packet: serde_json::Value = serde_json::from_str(&text)?;
        if json_value_contains_secret(&raw_packet) {
            bail!("packet {} contains secret-like material", path.display());
        }
        let packet: UpdatePacket = serde_json::from_value(raw_packet)?;
        util::validate_safe_component("packet id", &packet.id)?;
        if packet_contains_secret(&packet) {
            bail!("packet {} contains secret-like material", packet.id);
        }
        if packet.sensitivity == "secret" {
            continue;
        }
        if args.dry_run || !args.yes {
            if let Some(rule) = &packet.decision_rule {
                println!(
                    "would apply {}: when {:?}, choose {:?}, scope={}",
                    packet.id,
                    rule.situation,
                    rule.chosen,
                    rule.scope.as_deref().unwrap_or("global")
                );
            } else {
                println!("would apply {}: {}", packet.id, packet.claim);
            }
            if args.pending {
                append_packet_lifecycle_event(
                    &mut ledger,
                    active_run.as_ref(),
                    "preview-update",
                    &packet.id,
                    None,
                )?;
            }
            continue;
        }
        write_applied_packet(&root, &path, &packet)?;
        applied_packet_ids.push(packet.id);
    }
    if !applied_packet_ids.is_empty() {
        index::rebuild(&root)?;
        for packet_id in &applied_packet_ids {
            append_packet_lifecycle_event(
                &mut ledger,
                active_run.as_ref(),
                "apply-update",
                packet_id,
                Some(true),
            )?;
        }
    }
    println!("applied {} packet(s)", applied_packet_ids.len());
    Ok(())
}

pub fn pending_updates_value(root: &Path, requested_id: Option<&str>) -> Result<serde_json::Value> {
    if let Some(id) = requested_id {
        util::validate_safe_component("packet id", id)?;
    }
    let dir = root.join("99-meta/pending-update-packets");
    fs::create_dir_all(&dir)?;
    let mut values = Vec::new();
    let mut paths = fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .filter(|path| {
            !path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".applied.json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        if json_value_contains_secret(&raw) {
            bail!("packet {} contains secret-like material", path.display());
        }
        let packet: UpdatePacket = serde_json::from_value(raw)?;
        if requested_id.is_some_and(|id| packet.id != id) {
            continue;
        }
        values.push(json!({
            "id": packet.id,
            "classification": packet.classification,
            "claim": packet.claim,
            "confidence": packet.confidence,
            "sensitivity": packet.sensitivity,
            "decisionRule": packet.decision_rule,
            "status": packet.status,
        }));
    }
    Ok(serde_json::Value::Array(values))
}

pub fn preview_update_by_id(root: &Path, packet_id: &str) -> Result<serde_json::Value> {
    util::validate_safe_component("packet id", packet_id)?;
    let mut ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    let active_run = crate::dogfood::active_run_context(root)?;
    let preview = pending_updates_value(root, Some(packet_id))?;
    if preview.as_array().is_none_or(Vec::is_empty) {
        bail!("pending update packet {packet_id} was not found");
    }
    append_packet_lifecycle_event(
        &mut ledger,
        active_run.as_ref(),
        "preview-update",
        packet_id,
        None,
    )?;
    Ok(preview)
}

pub fn apply_update_by_id(root: &Path, packet_id: &str) -> Result<()> {
    util::validate_safe_component("packet id", packet_id)?;
    let _maintenance = util::acquire_vault_maintenance(root)?;
    let _lock = util::FileLock::acquire(&root.join(".brainmap/locks"), "update-apply.lock")?;
    let mut ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    let active_run = crate::dogfood::active_run_context(root)?;
    let dir = root.join("99-meta/pending-update-packets");
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json")
            || path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(".applied.json"))
        {
            continue;
        }
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        if json_value_contains_secret(&raw) {
            bail!("packet {} contains secret-like material", path.display());
        }
        let packet: UpdatePacket = serde_json::from_value(raw)?;
        if packet.id != packet_id {
            continue;
        }
        if packet_contains_secret(&packet) || packet.sensitivity == "secret" {
            bail!("packet {} contains secret-like material", packet.id);
        }
        write_applied_packet(root, &path, &packet)?;
        index::rebuild(root)?;
        append_packet_lifecycle_event(
            &mut ledger,
            active_run.as_ref(),
            "apply-update",
            packet_id,
            Some(true),
        )?;
        return Ok(());
    }
    bail!("pending update packet {packet_id} was not found")
}

fn append_packet_lifecycle_event(
    ledger: &mut util::LockedJsonl,
    active_run: Option<&crate::dogfood::DogfoodRunContext>,
    kind: &str,
    packet_id: &str,
    approved: Option<bool>,
) -> Result<()> {
    let Some(run) = active_run else {
        return Ok(());
    };
    ledger.append(&json!({
        "id": util::id("update", packet_id),
        "createdAt": util::now_iso(),
        "kind": kind,
        "packetId": packet_id,
        "approved": approved,
        "dogfoodRunId": run.run_id,
    }))
}

fn write_applied_packet(root: &Path, path: &Path, packet: &UpdatePacket) -> Result<()> {
    let target = root
        .join("60-decision-examples")
        .join(format!("{}.md", packet.id));
    let body = format!(
        "{}# {}\n\n## Claim\n\n{}\n\n{}## Evidence\n\n- {}\n\n## Links\n\n{}\n",
        crate::markdown::frontmatter(
            &packet.id,
            &packet.classification,
            "ask-before-action",
            &packet.sensitivity
        ),
        packet.claim,
        packet.claim,
        packet
            .decision_rule
            .as_ref()
            .map(markdown::decision_rule_marker)
            .transpose()?
            .map(|marker| format!("## Deterministic Rule\n\n{marker}\n\n"))
            .unwrap_or_default(),
        packet
            .evidence
            .first()
            .and_then(|value| value.get("quoteOrSummary"))
            .and_then(|value| value.as_str())
            .unwrap_or("packet evidence"),
        packet.suggested_links.join("\n")
    );
    util::write_atomic(&target, body.as_bytes())?;
    fs::rename(path, path.with_extension("applied.json"))?;
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
    println!(
        "{}",
        serde_json::to_string_pretty(&autopilot_status_value(&root)?)?
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AutopilotConfig {
    pub mode: String,
    pub level: String,
    pub threshold: f64,
}

pub(crate) fn autopilot_config(root: &Path) -> AutopilotConfig {
    let default = || AutopilotConfig {
        mode: "shadow".into(),
        level: "conservative".into(),
        threshold: 0.82,
    };
    let fail_closed = || AutopilotConfig {
        mode: "disabled".into(),
        level: "off".into(),
        threshold: 1.0,
    };
    let path = root.join(".brainmap/autopilot.json");
    if !path.exists() {
        return default();
    }
    let Ok(text) = fs::read_to_string(path) else {
        return fail_closed();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return fail_closed();
    };
    let threshold = value
        .get("threshold")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .unwrap_or(0.82);
    let Some(mode) = value.get("mode").and_then(|value| value.as_str()) else {
        return fail_closed();
    };
    if !matches!(mode, "shadow" | "disabled" | "conservative" | "balanced") {
        return fail_closed();
    }
    AutopilotConfig {
        mode: mode.into(),
        level: value
            .get("level")
            .and_then(|value| value.as_str())
            .unwrap_or("conservative")
            .into(),
        threshold,
    }
}

pub(crate) fn gate_mode_config(root: &Path) -> String {
    let path = root.join(".brainmap/gate-mode");
    if !path.exists() {
        return "shadow".into();
    }
    let Ok(mode) = fs::read_to_string(path) else {
        return "ask-always".into();
    };
    let mode = mode.trim();
    if matches!(mode, "ask-always" | "suggest-only" | "shadow" | "active") {
        mode.into()
    } else {
        "ask-always".into()
    }
}

pub(crate) fn autopilot_status_value(root: &Path) -> Result<serde_json::Value> {
    let config = autopilot_config(root);
    let shadow_metrics = shadow_metrics(root)?;
    Ok(json!({
        "mode": config.mode,
        "threshold": config.threshold,
        "level": config.level,
        "gateMode": gate_mode_config(root),
        "killSwitch": std::env::var("BRAINMAP_DISABLE_AUTOPILOT").ok().as_deref() == Some("1"),
        "shadowMetrics": shadow_metrics
    }))
}

pub fn autopilot_set(
    vault: Option<PathBuf>,
    mode: &str,
    level: &str,
    threshold: Option<f64>,
) -> Result<()> {
    if !matches!(mode, "shadow" | "disabled" | "conservative" | "balanced") {
        bail!("unsupported autopilot mode: {mode}");
    }
    if !matches!(level, "off" | "conservative" | "balanced" | "aggressive") {
        bail!("unsupported autopilot level: {level}");
    }
    let root = vault::resolve_vault(vault);
    let _ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    if crate::dogfood::active_run_context(&root)?.is_some() {
        bail!("cannot change autopilot configuration during an active dogfood run");
    }
    let threshold = threshold.unwrap_or_else(|| autopilot_config(&root).threshold);
    write_autopilot_config(&root, mode, level, threshold)?;
    println!("autopilot: mode={mode} level={level} threshold={threshold}");
    Ok(())
}

pub(crate) fn write_autopilot_config(
    root: &Path,
    mode: &str,
    level: &str,
    threshold: f64,
) -> Result<()> {
    fs::create_dir_all(root.join(".brainmap"))?;
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        bail!("autopilot threshold must be between 0.0 and 1.0");
    }
    util::write_atomic(
        &root.join(".brainmap/autopilot.json"),
        serde_json::to_vec_pretty(
            &json!({ "mode": mode, "level": level, "threshold": threshold }),
        )?
        .as_slice(),
    )?;
    Ok(())
}

pub fn autopilot_set_threshold(vault: Option<PathBuf>, threshold: f64) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let config = autopilot_config(&root);
    autopilot_set(Some(root), &config.mode, &config.level, Some(threshold))
}

pub fn autopilot_promote(vault: Option<PathBuf>, to: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let stats = autopilot_stats(&root)?;
    if !stats.ledger_integrity_valid {
        bail!(
            "promotion denied: shadow ledger integrity is invalid or action recording is incomplete"
        );
    }
    match to {
        "shadow" => autopilot_set(Some(root), "shadow", "conservative", None),
        "conservative" => {
            if stats.decisions < 30
                || stats.serious_mismatches >= 2
                || stats.privacy_violations > 0
                || stats.confirmed_cross_domain_applications > 0
            {
                bail!(
                    "promotion denied: need >=30 shadow decisions, <2 serious mismatches, 0 privacy/hard-rule violations, 0 confirmed cross-domain applications; got decisions={}, mismatches={}, violations={}, cross_domain={}",
                    stats.decisions,
                    stats.serious_mismatches,
                    stats.privacy_violations,
                    stats.confirmed_cross_domain_applications
                );
            }
            autopilot_set(Some(root), "conservative", "conservative", None)
        }
        "balanced" => {
            if stats.decisions < 100
                || stats.false_proceeds > 0
                || stats.privacy_violations > 0
                || stats.confirmed_cross_domain_applications > 0
            {
                bail!(
                    "promotion denied: balanced requires >=100 decisions, 0 false proceeds, 0 privacy/hard-rule violations, and 0 confirmed cross-domain applications"
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
    ledger_integrity_valid: bool,
    decisions: usize,
    serious_mismatches: usize,
    privacy_violations: usize,
    false_proceeds: usize,
    confirmed_cross_domain_applications: usize,
}

fn autopilot_stats(root: &Path) -> Result<AutopilotStats> {
    let metrics = shadow_metrics(root)?;
    Ok(AutopilotStats {
        ledger_integrity_valid: metrics.ledger_integrity_valid,
        decisions: metrics.decisions,
        serious_mismatches: metrics.corrections,
        privacy_violations: metrics.privacy_violations,
        false_proceeds: metrics.false_proceeds,
        confirmed_cross_domain_applications: metrics.confirmed_cross_domain_applications,
    })
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShadowMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate_binary_sha256: Option<String>,
    decisions: usize,
    exact_matches: usize,
    fuzzy_matches: usize,
    exact_match_rate: f64,
    fuzzy_match_rate: f64,
    agreement_opportunities: usize,
    agreements: usize,
    agreement_rate: f64,
    action_records: usize,
    complete_gate_action_pairs: usize,
    missing_action_records: usize,
    invalid_actual_action_records: usize,
    action_recording_rate: f64,
    duplicate_gate_ids: usize,
    duplicate_action_records: usize,
    duplicate_event_ids: usize,
    orphan_action_records: usize,
    orphan_feedback_records: usize,
    feedback_records: usize,
    feedback_packets: usize,
    feedback_missing_packet_ids: usize,
    preview_update_records: usize,
    apply_update_records: usize,
    previewed_feedback_packets: usize,
    applied_feedback_packets: usize,
    unpreviewed_feedback_packets: usize,
    unapplied_feedback_packets: usize,
    orphan_preview_update_records: usize,
    orphan_apply_update_records: usize,
    unapproved_apply_update_records: usize,
    packet_lifecycle_order_violations: usize,
    provenance_mismatches: usize,
    out_of_interval_events: usize,
    ledger_integrity_valid: bool,
    corrections: usize,
    correction_rate: f64,
    false_asks: usize,
    false_ask_rate: f64,
    candidate_collisions: usize,
    candidate_collision_rate: f64,
    confirmed_collisions: usize,
    confirmed_collision_rate: f64,
    confirmed_cross_domain_applications: usize,
    confirmed_cross_domain_application_rate: f64,
    mean_match_margin: Option<f64>,
    latency_p50_ms: Option<f64>,
    latency_p95_ms: Option<f64>,
    first_decision_at: Option<String>,
    last_decision_at: Option<String>,
    observation_started_at: Option<String>,
    observation_ended_at: Option<String>,
    observation_days: f64,
    distinct_decision_scenarios: usize,
    distinct_scopes: usize,
    distinct_decision_types: usize,
    intensive_session_distribution_valid: bool,
    privacy_violations: usize,
    privacy_violation_rate: f64,
    hard_rule_violations: usize,
    false_proceeds: usize,
    false_proceed_rate: f64,
    raw_prompts_retained: bool,
}

#[derive(Debug)]
struct ShadowGateSample {
    outcome: String,
    selected: Option<String>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    scenario_fingerprint: Option<String>,
    scope_fingerprint: Option<String>,
    decision_type_fingerprint: Option<String>,
}

#[derive(Debug)]
struct ShadowFeedbackSample {
    decision_id: String,
    packet_id: Option<String>,
    incident_type: Option<String>,
    ledger_position: usize,
}

#[derive(Debug)]
struct ShadowActionSample {
    chosen: Option<String>,
    was_asked: Option<bool>,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn shadow_metrics(root: &Path) -> Result<ShadowMetrics> {
    shadow_metrics_at(root, chrono::Utc::now())
}

fn aggregate_dimension_fingerprint(value: Option<&str>) -> Option<String> {
    let normalized = value?
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    (!normalized.is_empty()).then(|| util::sha256_hex(normalized.as_bytes()))
}

#[cfg(test)]
pub(crate) fn shadow_metrics_value_at(
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(shadow_metrics_at(root, now)?)?)
}

fn shadow_metrics_at(root: &Path, now: chrono::DateTime<chrono::Utc>) -> Result<ShadowMetrics> {
    let ledger = root.join("90-calibration/decision-ledger.jsonl");
    let mut locked = util::lock_jsonl(&ledger)?;
    let ledger_bytes = locked.read_all()?;
    shadow_metrics_from_bytes(root, now, &ledger_bytes)
}

pub(crate) fn shadow_metrics_value_from_locked_ledger_at(
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
    ledger_bytes: &[u8],
) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(shadow_metrics_from_bytes(
        root,
        now,
        ledger_bytes,
    )?)?)
}

fn shadow_metrics_from_bytes(
    root: &Path,
    now: chrono::DateTime<chrono::Utc>,
    ledger_bytes: &[u8],
) -> Result<ShadowMetrics> {
    let ledger = root.join("90-calibration/decision-ledger.jsonl");
    let active_run = crate::dogfood::active_run_context(root)?;
    if let Some(run) = &active_run {
        if run.mode != "shadow" {
            bail!(
                "active dogfood run {} is not configured for shadow mode",
                run.run_id
            );
        }
        if gate_mode_config(root) != "shadow" || autopilot_config(root).mode != "shadow" {
            bail!(
                "active dogfood run {} detected gate/autopilot mode drift from shadow",
                run.run_id
            );
        }
    }
    let (ledger_slice, line_offset) = if let Some(run) = &active_run {
        let boundary = usize::try_from(run.ledger_boundary_bytes)
            .context("dogfood ledger boundary does not fit this platform")?;
        if boundary > ledger_bytes.len() {
            bail!(
                "active dogfood run {} ledger boundary {} exceeds current ledger length {}",
                run.run_id,
                boundary,
                ledger_bytes.len()
            );
        }
        if util::sha256_hex(&ledger_bytes[..boundary]) != run.ledger_boundary_sha256 {
            bail!(
                "active dogfood run {} ledger prefix checksum does not match its start boundary",
                run.run_id
            );
        }
        if boundary > 0 && boundary < ledger_bytes.len() && ledger_bytes[boundary - 1] != b'\n' {
            bail!(
                "active dogfood run {} ledger boundary is not on a JSONL line boundary",
                run.run_id
            );
        }
        (
            &ledger_bytes[boundary..],
            run.ledger_boundary_lines as usize,
        )
    } else {
        (ledger_bytes, 0)
    };
    let text = std::str::from_utf8(ledger_slice)
        .with_context(|| format!("{} contains invalid UTF-8", ledger.display()))?;
    let mut metrics = ShadowMetrics {
        run_id: active_run.as_ref().map(|run| run.run_id.clone()),
        candidate_commit: active_run.as_ref().map(|run| run.candidate_commit.clone()),
        candidate_binary_sha256: active_run
            .as_ref()
            .map(|run| run.candidate_binary_sha256.clone()),
        ..ShadowMetrics::default()
    };
    let mut gates = HashMap::<String, ShadowGateSample>::new();
    let mut records = HashMap::<String, ShadowActionSample>::new();
    let mut corrected = HashSet::<String>::new();
    let mut feedback = Vec::<ShadowFeedbackSample>::new();
    let mut previewed_packets = HashMap::<String, usize>::new();
    let mut applied_packets = HashMap::<String, usize>::new();
    let mut seen_event_ids = HashSet::<String>::new();
    let mut margins = Vec::<f64>::new();
    let mut latencies = Vec::<u64>::new();
    let mut gate_timestamps = Vec::<(chrono::DateTime<chrono::Utc>, String)>::new();
    for (line_index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: serde_json::Value = serde_json::from_str(line).with_context(|| {
            format!(
                "parse decision ledger line {} in {}",
                line_index + 1,
                ledger.display()
            )
        })?;
        let kind = event.get("kind").and_then(serde_json::Value::as_str);
        let tracked_event = matches!(
            kind,
            Some(
                "decision-gate"
                    | "record-decision"
                    | "learn-feedback"
                    | "preview-update"
                    | "apply-update"
            )
        );
        if let Some(run) = &active_run
            && tracked_event
        {
            let event_run_id = event
                .get("dogfoodRunId")
                .and_then(serde_json::Value::as_str)
                .with_context(|| {
                    format!(
                        "{} event at ledger line {} is missing dogfoodRunId for active run {}",
                        kind.unwrap_or_default(),
                        line_offset + line_index + 1,
                        run.run_id
                    )
                })?;
            if event_run_id != run.run_id {
                bail!(
                    "{} event at ledger line {} belongs to dogfood run {}, expected {}",
                    kind.unwrap_or_default(),
                    line_offset + line_index + 1,
                    event_run_id,
                    run.run_id
                );
            }
        }
        let event_timestamp = if tracked_event {
            match event.get("createdAt").and_then(serde_json::Value::as_str) {
                Some(raw) => Some(
                    chrono::DateTime::parse_from_rfc3339(raw)
                        .with_context(|| {
                            format!(
                                "{} at ledger line {} has invalid createdAt",
                                kind.unwrap_or_default(),
                                line_offset + line_index + 1
                            )
                        })?
                        .with_timezone(&chrono::Utc),
                ),
                None if active_run.is_some() => {
                    bail!(
                        "{} at ledger line {} is missing createdAt",
                        kind.unwrap_or_default(),
                        line_offset + line_index + 1
                    )
                }
                None => None,
            }
        } else {
            None
        };
        if let (Some(run), Some(timestamp)) = (&active_run, event_timestamp)
            && (timestamp < run.started_at || timestamp > now)
        {
            metrics.out_of_interval_events += 1;
        }
        let id = event
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let mut duplicate_event_id = false;
        if tracked_event {
            if id.is_empty() {
                bail!(
                    "{} at ledger line {} is missing an id",
                    kind.unwrap_or_default(),
                    line_offset + line_index + 1
                );
            }
            if !seen_event_ids.insert(id.to_string()) {
                metrics.duplicate_event_ids += 1;
                duplicate_event_id = true;
            }
        }
        if duplicate_event_id && active_run.is_some() {
            continue;
        }
        match kind {
            Some("decision-gate") => {
                if gates.contains_key(id) {
                    metrics.duplicate_gate_ids += 1;
                    continue;
                }
                metrics.decisions += 1;
                match event.get("matchKind").and_then(serde_json::Value::as_str) {
                    Some("exact") => metrics.exact_matches += 1,
                    Some("fuzzy") => metrics.fuzzy_matches += 1,
                    _ => {}
                }
                metrics.candidate_collisions += usize::from(
                    event
                        .get("candidateCollision")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                );
                if let Some(margin) = event.get("matchMargin").and_then(serde_json::Value::as_f64) {
                    margins.push(margin);
                }
                if let Some(latency) = event
                    .get("evaluationLatencyMicros")
                    .and_then(serde_json::Value::as_u64)
                {
                    latencies.push(latency);
                }
                if let (Some(timestamp), Some(created_at)) = (
                    event_timestamp,
                    event.get("createdAt").and_then(serde_json::Value::as_str),
                ) {
                    gate_timestamps.push((timestamp, created_at.into()));
                }
                if let Some(run) = &active_run {
                    let threshold_matches = event
                        .get("dogfoodThreshold")
                        .and_then(serde_json::Value::as_f64)
                        .is_some_and(|value| (value - run.threshold).abs() <= f64::EPSILON);
                    let provenance_matches =
                        event.get("gateMode").and_then(serde_json::Value::as_str)
                            == Some(run.gate_mode.as_str())
                            && event
                                .get("autopilotMode")
                                .and_then(serde_json::Value::as_str)
                                == Some(run.autopilot_mode.as_str())
                            && event
                                .get("autopilotLevel")
                                .and_then(serde_json::Value::as_str)
                                == Some(run.autopilot_level.as_str())
                            && event
                                .get("dogfoodCandidateBinarySha256")
                                .and_then(serde_json::Value::as_str)
                                == Some(run.candidate_binary_sha256.as_str())
                            && threshold_matches;
                    metrics.provenance_mismatches += usize::from(!provenance_matches);
                }
                gates.insert(
                    id.into(),
                    ShadowGateSample {
                        outcome: event
                            .get("predictedOutcome")
                            .or_else(|| event.get("outcome"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .into(),
                        selected: event
                            .get("predictedSelectedOption")
                            .or_else(|| event.get("selectedOption"))
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        created_at: event_timestamp,
                        scenario_fingerprint: aggregate_dimension_fingerprint(
                            event.get("situation").and_then(serde_json::Value::as_str),
                        ),
                        scope_fingerprint: aggregate_dimension_fingerprint(
                            event.get("scope").and_then(serde_json::Value::as_str),
                        ),
                        decision_type_fingerprint: aggregate_dimension_fingerprint(
                            event
                                .get("decisionType")
                                .and_then(serde_json::Value::as_str),
                        ),
                    },
                );
            }
            Some("record-decision") => {
                let decision_id = event
                    .get("decisionId")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| {
                        active_run
                            .is_none()
                            .then_some(id)
                            .filter(|id| !id.is_empty())
                    })
                    .with_context(|| {
                        format!(
                            "record-decision at ledger line {} is missing decisionId",
                            line_offset + line_index + 1
                        )
                    })?;
                if records.contains_key(decision_id) {
                    metrics.duplicate_action_records += 1;
                    continue;
                }
                records.insert(
                    decision_id.into(),
                    ShadowActionSample {
                        chosen: event
                            .get("chosen")
                            .and_then(serde_json::Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_string),
                        was_asked: event
                            .get("wasAsked")
                            .and_then(serde_json::Value::as_bool)
                            .or_else(|| active_run.is_none().then_some(false)),
                        created_at: event_timestamp,
                    },
                );
            }
            Some("learn-feedback") => {
                let decision_id = event
                    .get("decisionId")
                    .and_then(serde_json::Value::as_str)
                    .with_context(|| {
                        format!(
                            "learn-feedback at ledger line {} is missing decisionId",
                            line_offset + line_index + 1
                        )
                    })?;
                feedback.push(ShadowFeedbackSample {
                    decision_id: decision_id.into(),
                    packet_id: event
                        .get("packetId")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    incident_type: event
                        .get("incidentType")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                    ledger_position: line_index,
                });
            }
            Some("preview-update") => {
                metrics.preview_update_records += 1;
                let packet_id = event
                    .get("packetId")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .with_context(|| {
                        format!(
                            "preview-update at ledger line {} is missing packetId",
                            line_offset + line_index + 1
                        )
                    })?;
                previewed_packets
                    .entry(packet_id.into())
                    .or_insert(line_index);
            }
            Some("apply-update") => {
                metrics.apply_update_records += 1;
                let packet_id = event
                    .get("packetId")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.is_empty())
                    .with_context(|| {
                        format!(
                            "apply-update at ledger line {} is missing packetId",
                            line_offset + line_index + 1
                        )
                    })?;
                if event.get("approved").and_then(serde_json::Value::as_bool) == Some(true) {
                    applied_packets
                        .entry(packet_id.into())
                        .or_insert(line_index);
                } else {
                    metrics.unapproved_apply_update_records += 1;
                }
            }
            _ => {}
        }
    }
    let mut decision_scenarios = HashSet::new();
    let mut scopes = HashSet::new();
    let mut decision_types = HashSet::new();
    for (id, gate) in &gates {
        if let Some(record) = records.get(id) {
            let timestamps_are_chronological = active_run.is_none()
                || matches!(
                    (gate.created_at, record.created_at),
                    (Some(gate_at), Some(action_at)) if action_at >= gate_at
                );
            let valid_actual = record.was_asked.is_some()
                && (record.chosen.is_some() || record.was_asked == Some(true))
                && timestamps_are_chronological;
            if !valid_actual {
                metrics.invalid_actual_action_records += 1;
                continue;
            }
            metrics.complete_gate_action_pairs += 1;
            decision_scenarios.extend(gate.scenario_fingerprint.iter().cloned());
            scopes.extend(gate.scope_fingerprint.iter().cloned());
            decision_types.extend(gate.decision_type_fingerprint.iter().cloned());
            match gate.outcome.as_str() {
                "proceed" => {
                    metrics.agreement_opportunities += 1;
                    metrics.agreements += usize::from(
                        record.was_asked == Some(false)
                            && gate.selected.as_ref().is_some_and(|predicted| {
                                record
                                    .chosen
                                    .as_ref()
                                    .is_some_and(|actual| predicted.eq_ignore_ascii_case(actual))
                            }),
                    );
                }
                "ask_user" => {
                    metrics.agreement_opportunities += 1;
                    metrics.agreements += usize::from(record.was_asked == Some(true));
                    metrics.false_asks += usize::from(record.was_asked == Some(false));
                }
                _ => {}
            }
        }
    }
    metrics.action_records = records
        .keys()
        .filter(|decision_id| gates.contains_key(*decision_id))
        .count();
    metrics.distinct_decision_scenarios = decision_scenarios.len();
    metrics.distinct_scopes = scopes.len();
    metrics.distinct_decision_types = decision_types.len();
    metrics.intensive_session_distribution_valid = metrics.complete_gate_action_pairs >= 30
        && metrics.distinct_decision_scenarios >= 5
        && (metrics.distinct_scopes >= 3 || metrics.distinct_decision_types >= 3);
    metrics.orphan_action_records = records
        .keys()
        .filter(|decision_id| !gates.contains_key(*decision_id))
        .count();
    metrics.missing_action_records = gates.len().saturating_sub(metrics.action_records);
    metrics.action_recording_rate = metric_ratio(metrics.action_records, metrics.decisions);
    let mut feedback_packet_positions = HashMap::new();
    for sample in feedback {
        metrics.feedback_records += 1;
        if !gates.contains_key(&sample.decision_id) {
            metrics.orphan_feedback_records += 1;
            continue;
        }
        if let Some(packet_id) = sample.packet_id {
            feedback_packet_positions
                .entry(packet_id)
                .or_insert(sample.ledger_position);
        } else if active_run.is_some() {
            metrics.feedback_missing_packet_ids += 1;
        }
        corrected.insert(sample.decision_id);
        match sample.incident_type.as_deref() {
            Some("false-proceed") => metrics.false_proceeds += 1,
            Some("confirmed-collision") => metrics.confirmed_collisions += 1,
            Some("cross-domain-application") => {
                metrics.confirmed_cross_domain_applications += 1;
            }
            Some("privacy-violation") => metrics.privacy_violations += 1,
            Some("hard-rule-violation") => metrics.hard_rule_violations += 1,
            _ => {}
        }
    }
    metrics.feedback_packets = feedback_packet_positions.len();
    metrics.previewed_feedback_packets = feedback_packet_positions
        .keys()
        .filter(|packet_id| previewed_packets.contains_key(*packet_id))
        .count();
    metrics.applied_feedback_packets = feedback_packet_positions
        .keys()
        .filter(|packet_id| applied_packets.contains_key(*packet_id))
        .count();
    metrics.unpreviewed_feedback_packets = feedback_packet_positions
        .len()
        .saturating_sub(metrics.previewed_feedback_packets);
    metrics.unapplied_feedback_packets = feedback_packet_positions
        .len()
        .saturating_sub(metrics.applied_feedback_packets);
    metrics.orphan_preview_update_records = previewed_packets
        .keys()
        .filter(|packet_id| !feedback_packet_positions.contains_key(*packet_id))
        .count();
    metrics.orphan_apply_update_records = applied_packets
        .keys()
        .filter(|packet_id| !feedback_packet_positions.contains_key(*packet_id))
        .count();
    metrics.packet_lifecycle_order_violations = feedback_packet_positions
        .iter()
        .filter(|(packet_id, feedback_position)| {
            matches!(
                (
                    previewed_packets.get(*packet_id),
                    applied_packets.get(*packet_id)
                ),
                (Some(preview_position), Some(apply_position))
                    if *feedback_position >= preview_position || preview_position >= apply_position
            )
        })
        .count();
    metrics.corrections = corrected.len();
    metrics.exact_match_rate = metric_ratio(metrics.exact_matches, metrics.decisions);
    metrics.fuzzy_match_rate = metric_ratio(metrics.fuzzy_matches, metrics.decisions);
    metrics.agreement_rate = metric_ratio(metrics.agreements, metrics.agreement_opportunities);
    metrics.correction_rate = metric_ratio(metrics.corrections, metrics.decisions);
    metrics.false_ask_rate = metric_ratio(metrics.false_asks, metrics.decisions);
    metrics.candidate_collision_rate =
        metric_ratio(metrics.candidate_collisions, metrics.decisions);
    metrics.confirmed_collision_rate =
        metric_ratio(metrics.confirmed_collisions, metrics.decisions);
    metrics.confirmed_cross_domain_application_rate = metric_ratio(
        metrics.confirmed_cross_domain_applications,
        metrics.decisions,
    );
    metrics.privacy_violation_rate = metric_ratio(metrics.privacy_violations, metrics.decisions);
    metrics.false_proceed_rate = metric_ratio(metrics.false_proceeds, metrics.decisions);
    metrics.mean_match_margin =
        (!margins.is_empty()).then(|| margins.iter().sum::<f64>() / margins.len() as f64);
    latencies.sort_unstable();
    metrics.latency_p50_ms = metric_percentile_ms(&latencies, 50);
    metrics.latency_p95_ms = metric_percentile_ms(&latencies, 95);
    gate_timestamps.sort_by_key(|(timestamp, _)| *timestamp);
    metrics.first_decision_at = gate_timestamps.first().map(|(_, raw)| raw.clone());
    metrics.last_decision_at = gate_timestamps.last().map(|(_, raw)| raw.clone());
    if let Some(run) = &active_run {
        if now < run.started_at {
            bail!(
                "active dogfood run {} observation clock precedes its start time",
                run.run_id
            );
        }
        metrics.observation_started_at = Some(
            run.started_at
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );
        metrics.observation_ended_at = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
        metrics.observation_days = (now - run.started_at).num_seconds() as f64 / 86_400.0;
    } else if let (Some((first, first_raw)), Some((last, last_raw))) =
        (gate_timestamps.first(), gate_timestamps.last())
    {
        metrics.observation_started_at = Some(first_raw.clone());
        metrics.observation_ended_at = Some(last_raw.clone());
        metrics.observation_days = (*last - *first).num_seconds().max(0) as f64 / 86_400.0;
    }
    metrics.ledger_integrity_valid = metrics.duplicate_gate_ids == 0
        && metrics.duplicate_action_records == 0
        && metrics.duplicate_event_ids == 0
        && metrics.orphan_action_records == 0
        && metrics.orphan_feedback_records == 0
        && metrics.missing_action_records == 0
        && metrics.invalid_actual_action_records == 0
        && metrics.feedback_missing_packet_ids == 0
        && metrics.unpreviewed_feedback_packets == 0
        && metrics.unapplied_feedback_packets == 0
        && metrics.orphan_preview_update_records == 0
        && metrics.orphan_apply_update_records == 0
        && metrics.unapproved_apply_update_records == 0
        && metrics.packet_lifecycle_order_violations == 0
        && metrics.provenance_mismatches == 0
        && metrics.out_of_interval_events == 0;
    Ok(metrics)
}

fn metric_ratio(part: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 / total as f64
    }
}

fn metric_percentile_ms(samples_micros: &[u64], percentile: usize) -> Option<f64> {
    if samples_micros.is_empty() {
        return None;
    }
    let index = ((samples_micros.len() * percentile).div_ceil(100)).saturating_sub(1);
    Some(samples_micros[index.min(samples_micros.len() - 1)] as f64 / 1_000.0)
}

pub fn gate_mode(vault: Option<PathBuf>, mode: &str) -> Result<()> {
    if !matches!(mode, "ask-always" | "suggest-only" | "shadow" | "active") {
        bail!("unsupported gate mode: {mode}");
    }
    let root = vault::resolve_vault(vault);
    let _ledger = util::lock_jsonl(&root.join("90-calibration/decision-ledger.jsonl"))?;
    if crate::dogfood::active_run_context(&root)?.is_some() {
        bail!("cannot change gate mode during an active dogfood run");
    }
    write_gate_mode_config(&root, mode)?;
    println!("gate mode: {mode}");
    Ok(())
}

pub(crate) fn write_gate_mode_config(root: &Path, mode: &str) -> Result<()> {
    fs::create_dir_all(root.join(".brainmap"))?;
    util::write_atomic(&root.join(".brainmap/gate-mode"), mode.as_bytes())?;
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
        decision_rule: None,
        status: "pending".into(),
    }
}

struct DecisionContext {
    situation: String,
    options: Vec<String>,
    decision_type: String,
    scope: String,
    outcome: String,
    candidate_collision: bool,
    learned_rule_applied: bool,
}

fn decision_context_from_bytes(
    ledger_bytes: &[u8],
    decision_id: &str,
) -> Result<Option<DecisionContext>> {
    let text = std::str::from_utf8(ledger_bytes).context("decision ledger is not valid UTF-8")?;
    for line in text.lines().rev().filter(|line| !line.trim().is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        if value.get("id").and_then(|value| value.as_str()) == Some(decision_id)
            && let Some(situation) = value.get("situation").and_then(|value| value.as_str())
        {
            let options = value
                .get("options")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect();
            let decision_type = value
                .get("decisionType")
                .and_then(|value| value.as_str())
                .unwrap_or("general")
                .to_string();
            let scope = value
                .get("scope")
                .and_then(|value| value.as_str())
                .unwrap_or("global")
                .to_string();
            let outcome = value
                .get("predictedOutcome")
                .or_else(|| value.get("outcome"))
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            let learned_rule_applied = value
                .get("appliedPolicies")
                .and_then(|value| value.as_array())
                .is_some_and(|policies| {
                    policies.iter().any(|policy| {
                        policy
                            .as_str()
                            .is_some_and(|path| path.contains("60-decision-examples/"))
                    })
                });
            let candidate_collision = value
                .get("candidateCollision")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            return Ok(Some(DecisionContext {
                situation: situation.to_string(),
                options,
                decision_type,
                scope,
                outcome,
                candidate_collision,
                learned_rule_applied,
            }));
        }
    }
    Ok(None)
}

fn validate_feedback_incident(
    incident: Option<crate::cli::FeedbackIncident>,
    context: &DecisionContext,
) -> Result<()> {
    use crate::cli::FeedbackIncident;
    match incident {
        Some(FeedbackIncident::FalseProceed) if context.outcome != "proceed" => {
            bail!("false-proceed incident requires an original proceed outcome")
        }
        Some(FeedbackIncident::ConfirmedCollision) if !context.candidate_collision => {
            bail!("confirmed-collision incident requires a candidate collision")
        }
        Some(FeedbackIncident::CrossDomainApplication) if !context.learned_rule_applied => {
            bail!("cross-domain incident requires an applied learned decision rule")
        }
        _ => Ok(()),
    }
}

fn validate_decision_id(decision_id: &str) -> Result<()> {
    util::validate_safe_component("decision id", decision_id)?;
    if privacy::contains_secret(decision_id) {
        bail!("decision id contains secret-like material");
    }
    Ok(())
}

fn normalize_feedback_rule(correction: &str) -> (String, Vec<String>) {
    let lower = correction.to_ascii_lowercase();
    let rejected = rejected_feedback_choices(correction);
    let negative_ask = ["do not ask", "don't ask", "never ask"]
        .iter()
        .any(|marker| lower.contains(marker));
    if !negative_ask
        && ["ask user", "ask me", "always ask", "require approval"]
            .iter()
            .any(|marker| lower.contains(marker))
    {
        return ("ask user".into(), rejected);
    }
    if negative_ask
        && let Some(last_clause) = correction.rsplit(';').next()
        && !last_clause.to_lowercase().contains("ask")
    {
        return (clean_feedback_clause(last_clause), rejected);
    }
    if let Some((_, choice)) = correction.rsplit_once(" instead ") {
        return (clean_feedback_clause(choice), rejected);
    }
    for marker in ["choose ", "use ", "prefer "] {
        if let Some(start) = lower.find(marker) {
            return (
                clean_feedback_clause(&correction[start + marker.len()..]),
                rejected,
            );
        }
    }
    (compact(correction), rejected)
}

fn rejected_feedback_choices(correction: &str) -> Vec<String> {
    let lower = correction.to_ascii_lowercase();
    let mut rejected = Vec::new();
    for marker in ["never ", "do not ", "don't "] {
        let mut offset = 0usize;
        while let Some(relative) = lower[offset..].find(marker) {
            let start = offset + relative + marker.len();
            let tail = &correction[start..];
            let end = tail.find([';', ',', '.']).unwrap_or(tail.len());
            let choice = clean_feedback_clause(&tail[..end]);
            if !choice.is_empty() && !rejected.contains(&choice) {
                rejected.push(choice);
            }
            offset = start + end;
            if offset >= lower.len() {
                break;
            }
        }
    }
    rejected
}

fn clean_feedback_clause(value: &str) -> String {
    value
        .trim()
        .trim_matches(|ch: char| matches!(ch, ';' | ',' | '.'))
        .trim()
        .to_string()
}

fn write_packet(root: &Path, stem: &str, packet: &UpdatePacket) -> Result<PathBuf> {
    util::validate_safe_component("packet filename stem", stem)?;
    util::validate_safe_component("packet id", &packet.id)?;
    if packet_contains_secret(packet) {
        bail!("packet {} contains secret-like material", packet.id);
    }
    let bytes = serde_json::to_vec_pretty(packet)?;
    fs::create_dir_all(root.join("99-meta/pending-update-packets"))?;
    let path = root
        .join("99-meta/pending-update-packets")
        .join(format!("{stem}-{}.json", packet.id));
    util::write_atomic(&path, &bytes)?;
    Ok(path)
}

fn packet_contains_secret(packet: &UpdatePacket) -> bool {
    let strings = [
        packet.id.as_str(),
        packet.created_at.as_str(),
        packet.classification.as_str(),
        packet.claim.as_str(),
        packet.sensitivity.as_str(),
        packet.action.as_str(),
        packet.status.as_str(),
    ];
    strings.into_iter().any(privacy::contains_secret)
        || json_value_contains_secret(&packet.source)
        || packet.evidence.iter().any(json_value_contains_secret)
        || packet
            .target_notes
            .iter()
            .chain(packet.suggested_links.iter())
            .any(|value| privacy::contains_secret(value))
        || packet
            .human_question
            .as_deref()
            .is_some_and(privacy::contains_secret)
        || packet.decision_rule.as_ref().is_some_and(|rule| {
            privacy::contains_secret(&rule.situation)
                || privacy::contains_secret(&rule.chosen)
                || rule
                    .options
                    .iter()
                    .chain(rule.rejected.iter())
                    .any(|value| privacy::contains_secret(value))
        })
}

fn json_value_contains_secret(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(value) => privacy::contains_secret(value),
        serde_json::Value::Array(values) => values.iter().any(json_value_contains_secret),
        serde_json::Value::Object(values) => {
            values.keys().any(|key| privacy::contains_secret(key))
                || values.values().any(json_value_contains_secret)
        }
        _ => false,
    }
}

fn compact(text: &str) -> String {
    text.split_whitespace()
        .take(200)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activate_test_dogfood_run(root: &Path, run_id: &str) -> chrono::DateTime<chrono::Utc> {
        let started_at = chrono::Utc::now() - chrono::Duration::hours(1);
        activate_test_dogfood_run_at(root, run_id, started_at);
        started_at
    }

    fn activate_test_dogfood_run_at(
        root: &Path,
        run_id: &str,
        started_at: chrono::DateTime<chrono::Utc>,
    ) {
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        let ledger_bytes = fs::read(&ledger).unwrap_or_default();
        let candidate_binary_identity = crate::dogfood::current_binary_identity().unwrap();
        util::write_atomic(
            &root.join(".brainmap/dogfood.json"),
            serde_json::to_vec_pretty(&json!({
                "format": "brainmap-dogfood-runs",
                "version": 3,
                "runs": [{
                    "runId": run_id,
                    "status": "active",
                    "candidateCommit": "1111111111111111111111111111111111111111",
                    "candidateBinarySha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "candidateBrainmapdSha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                    "candidateBinaryIdentity": candidate_binary_identity,
                    "host": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH },
                    "adapter": "codex",
                    "startedAt": started_at,
                    "mode": "shadow",
                    "gateMode": "shadow",
                    "autopilotMode": "shadow",
                    "autopilotLevel": "conservative",
                    "threshold": 0.82,
                    "startBackup": { "relativePath": format!("99-meta/backups/{run_id}-start.brainmap.tar.zst"), "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
                    "qualificationBundleSha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "qualificationManifestSha256": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                    "qualificationBundleRelativePath": format!(".brainmap/dogfood/{run_id}/qualification"),
                    "ledgerBoundaryBytes": ledger_bytes.len(),
                    "ledgerBoundaryLines": ledger_bytes.iter().filter(|byte| **byte == b'\n').count(),
                    "ledgerBoundarySha256": util::sha256_hex(&ledger_bytes)
                }]
            }))
            .unwrap()
            .as_slice(),
        )
        .unwrap();
    }

    fn test_timestamp(started_at: chrono::DateTime<chrono::Utc>, minutes: i64) -> String {
        (started_at + chrono::Duration::minutes(minutes))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    #[test]
    fn feedback_secret_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        learn_feedback(LearnFeedbackArgs {
            decision_id: "dec_1".into(),
            correction: Some("api_key=abcdef1234567890".into()),
            chosen: None,
            rejected: None,
            incident: None,
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

    #[test]
    fn shadow_metrics_are_aggregate_and_contain_no_raw_prompts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "createdAt": "2026-07-01T00:00:00Z",
                "situation": "do-not-emit-this-raw-prompt",
                "outcome": "proceed",
                "selectedOption": "biome",
                "matchKind": "exact",
                "matchMargin": 0.4,
                "candidateCollision": false,
                "evaluationLatencyMicros": 100
            }),
            json!({
                "id": "action-1",
                "decisionId": "decision-1",
                "kind": "record-decision",
                "createdAt": "2026-07-01T00:01:00Z",
                "chosen": "biome",
                "wasAsked": false
            }),
            json!({
                "id": "decision-2",
                "kind": "decision-gate",
                "createdAt": "2026-07-10T00:00:00Z",
                "situation": "another-do-not-emit-prompt",
                "outcome": "ask_user",
                "selectedOption": null,
                "matchKind": "fuzzy",
                "matchMargin": 0.05,
                "candidateCollision": true,
                "evaluationLatencyMicros": 500
            }),
            json!({
                "id": "action-2",
                "decisionId": "decision-2",
                "kind": "record-decision",
                "createdAt": "2026-07-10T00:01:00Z",
                "chosen": "prettier",
                "wasAsked": false
            }),
            json!({
                "id": "feedback-1",
                "decisionId": "decision-2",
                "kind": "learn-feedback",
                "createdAt": "2026-07-10T00:02:00Z",
                "chosen": "prettier",
                "incidentType": "cross-domain-application"
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];
        assert_eq!(metrics["decisions"], 2);
        assert_eq!(metrics["exactMatches"], 1);
        assert_eq!(metrics["fuzzyMatches"], 1);
        assert_eq!(metrics["agreementOpportunities"], 2);
        assert_eq!(metrics["agreements"], 1);
        assert_eq!(metrics["corrections"], 1);
        assert_eq!(metrics["falseAsks"], 1);
        assert_eq!(metrics["candidateCollisions"], 1);
        assert_eq!(metrics["candidateCollisionRate"], 0.5);
        assert_eq!(metrics["confirmedCollisions"], 0);
        assert_eq!(metrics["confirmedCollisionRate"], 0.0);
        assert_eq!(metrics["confirmedCrossDomainApplications"], 1);
        assert_eq!(metrics["confirmedCrossDomainApplicationRate"], 0.5);
        assert_eq!(metrics["falseProceeds"], 0);
        assert_eq!(metrics["latencyP50Ms"], 0.1);
        assert_eq!(metrics["latencyP95Ms"], 0.5);
        assert_eq!(metrics["observationDays"], 9.0);
        assert_eq!(metrics["actionRecords"], 2);
        assert_eq!(metrics["missingActionRecords"], 0);
        assert_eq!(metrics["ledgerIntegrityValid"], true);
        assert_eq!(metrics["rawPromptsRetained"], false);
        assert!(metrics.get("collisions").is_none());
        let serialized = serde_json::to_string(&status).unwrap();
        assert!(!serialized.contains("do-not-emit"));
    }

    #[test]
    fn confirmed_collision_incident_is_not_a_cross_domain_application() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "outcome": "ask_user",
                "predictedOutcome": "ask_user",
                "candidateCollision": true
            }),
            json!({
                "id": "action-1",
                "decisionId": "decision-1",
                "kind": "record-decision",
                "chosen": "biome",
                "wasAsked": true
            }),
            json!({
                "id": "feedback-1",
                "decisionId": "decision-1",
                "kind": "learn-feedback",
                "incidentType": "confirmed-collision"
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];

        assert_eq!(metrics["confirmedCollisions"], 1);
        assert_eq!(metrics["confirmedCrossDomainApplications"], 0);
    }

    #[test]
    fn autopilot_status_surfaces_malformed_ledger_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        fs::write(
            root.join("90-calibration/decision-ledger.jsonl"),
            "{\"id\":\"decision-1\",\"kind\":\"decision-gate\",\"outcome\":\"ask_user\"}\nnot-json\n",
        )
        .unwrap();

        let error = autopilot_status_value(&root).unwrap_err();

        assert!(error.to_string().contains("ledger line 2"), "{error:#}");
    }

    #[test]
    fn autopilot_status_surfaces_duplicate_gate_ids_without_counting_them_twice() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "biome"
            }),
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "prettier"
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];

        assert_eq!(metrics["decisions"], 1);
        assert_eq!(metrics["duplicateGateIds"], 1);
        assert_eq!(metrics["duplicateEventIds"], 1);
        assert_eq!(metrics["ledgerIntegrityValid"], false);
    }

    #[test]
    fn active_run_metrics_ignore_pre_boundary_history() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "old-gate",
                "kind": "decision-gate",
                "outcome": "proceed",
                "selectedOption": "old-choice",
                "matchKind": "exact"
            }),
            json!({
                "id": "old-action",
                "decisionId": "old-gate",
                "kind": "record-decision",
                "chosen": "old-choice",
                "wasAsked": false
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        for event in [
            json!({
                "id": "current-gate",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "biome",
                "matchKind": "fuzzy"
            }),
            json!({
                "id": "current-action",
                "decisionId": "current-gate",
                "kind": "record-decision",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 2),
                "chosen": "biome",
                "wasAsked": false
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];

        assert_eq!(metrics["runId"], "dogfood_current");
        assert_eq!(metrics["decisions"], 1);
        assert_eq!(metrics["fuzzyMatches"], 1);
        assert_eq!(metrics["agreements"], 1);
    }

    #[test]
    fn action_record_uses_unique_event_id_and_active_run_linkage() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        util::append_jsonl(
            &ledger,
            &json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "biome"
            }),
        )
        .unwrap();

        record_decision_quiet(RecordDecisionArgs {
            decision_id: Some("decision-1".into()),
            chosen: Some("biome".into()),
            was_asked: Some(true),
            vault: Some(root.clone()),
        })
        .unwrap();

        let ledger = fs::read_to_string(ledger).unwrap();
        let action: serde_json::Value =
            serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
        assert_ne!(action["id"], "decision-1");
        assert_eq!(action["decisionId"], "decision-1");
        assert_eq!(action["dogfoodRunId"], "dogfood_current");
    }

    #[test]
    fn feedback_record_carries_active_run_linkage() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        util::append_jsonl(
            &ledger,
            &json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "situation": "Choose a formatter",
                "options": ["biome", "prettier"],
                "decisionType": "tooling",
                "scope": "project:test",
                "appliedPolicies": ["[[60-decision-examples/formatter.md]]"]
            }),
        )
        .unwrap();

        learn_feedback_quiet(LearnFeedbackArgs {
            decision_id: "decision-1".into(),
            correction: None,
            chosen: Some("prettier".into()),
            rejected: Some("biome".into()),
            incident: None,
            vault: Some(root.clone()),
        })
        .unwrap();

        let ledger = fs::read_to_string(ledger).unwrap();
        let feedback: serde_json::Value =
            serde_json::from_str(ledger.lines().last().unwrap()).unwrap();
        assert_eq!(feedback["kind"], "learn-feedback");
        assert_eq!(feedback["decisionId"], "decision-1");
        assert_eq!(feedback["dogfoodRunId"], "dogfood_current");
    }

    #[test]
    fn active_run_rejects_enforcement_mode_drift() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        activate_test_dogfood_run(&root, "dogfood_current");

        let autopilot_error =
            autopilot_set(Some(root.clone()), "conservative", "conservative", None).unwrap_err();
        let gate_error = gate_mode(Some(root.clone()), "active").unwrap_err();

        assert!(autopilot_error.to_string().contains("active dogfood run"));
        assert!(gate_error.to_string().contains("active dogfood run"));
        assert_eq!(autopilot_config(&root).mode, "shadow");
        assert_eq!(gate_mode_config(&root), "shadow");
    }

    #[test]
    fn active_run_metrics_report_missing_action_coverage() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "biome"
            }),
            json!({
                "id": "decision-2",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 2),
                "outcome": "ask_user",
                "predictedOutcome": "ask_user",
                "predictedSelectedOption": null
            }),
            json!({
                "id": "action-1",
                "decisionId": "decision-1",
                "kind": "record-decision",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 3),
                "chosen": "biome",
                "wasAsked": true
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];

        assert_eq!(metrics["actionRecords"], 1);
        assert_eq!(metrics["missingActionRecords"], 1);
        assert_eq!(metrics["actionRecordingRate"], 0.5);
    }

    #[test]
    fn active_run_metrics_report_only_aggregate_intensive_session_dimensions() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");

        for index in 0..30 {
            util::append_jsonl(
                &ledger,
                &json!({
                    "id": format!("decision-{index}"),
                    "kind": "decision-gate",
                    "dogfoodRunId": "dogfood_current",
                    "createdAt": test_timestamp(started_at, i64::from(index) * 2 + 1),
                    "outcome": "ask_user",
                    "predictedOutcome": "proceed",
                    "predictedSelectedOption": "biome",
                    "situation": format!("SECRET_SCENARIO_{}", index % 5),
                    "scope": format!("project:SENSITIVE_SCOPE_{}", index % 2),
                    "decisionType": format!("PRIVATE_TYPE_{}", index % 3)
                }),
            )
            .unwrap();
            util::append_jsonl(
                &ledger,
                &json!({
                    "id": format!("action-{index}"),
                    "decisionId": format!("decision-{index}"),
                    "kind": "record-decision",
                    "dogfoodRunId": "dogfood_current",
                    "createdAt": test_timestamp(started_at, i64::from(index) * 2 + 2),
                    "chosen": "biome",
                    "wasAsked": false
                }),
            )
            .unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];
        let serialized = serde_json::to_string(metrics).unwrap();

        assert_eq!(metrics["completeGateActionPairs"], 30);
        assert_eq!(metrics["distinctDecisionScenarios"], 5);
        assert_eq!(metrics["distinctScopes"], 2);
        assert_eq!(metrics["distinctDecisionTypes"], 3);
        assert_eq!(metrics["intensiveSessionDistributionValid"], true);
        assert!(!serialized.contains("SECRET_SCENARIO"));
        assert!(!serialized.contains("SENSITIVE_SCOPE"));
        assert!(!serialized.contains("PRIVATE_TYPE"));
        assert!(metrics.get("decisionScenarios").is_none());
        assert!(metrics.get("scopes").is_none());
        assert!(metrics.get("decisionTypes").is_none());
    }

    #[test]
    fn active_run_metrics_surface_duplicate_action_records() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "biome"
            }),
            json!({
                "id": "action-1",
                "decisionId": "decision-1",
                "kind": "record-decision",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 2),
                "chosen": "biome",
                "wasAsked": true
            }),
            json!({
                "id": "action-2",
                "decisionId": "decision-1",
                "kind": "record-decision",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 3),
                "chosen": "prettier",
                "wasAsked": true
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];

        assert_eq!(metrics["actionRecords"], 1);
        assert_eq!(metrics["duplicateActionRecords"], 1);
        assert_eq!(metrics["ledgerIntegrityValid"], false);
    }

    #[test]
    fn active_run_metrics_surface_orphan_action_and_feedback_records() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "action-orphan",
                "decisionId": "decision-missing",
                "kind": "record-decision",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "chosen": "biome",
                "wasAsked": true
            }),
            json!({
                "id": "feedback-orphan",
                "decisionId": "decision-missing",
                "kind": "learn-feedback",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 2),
                "chosen": "prettier"
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];

        assert_eq!(metrics["orphanActionRecords"], 1);
        assert_eq!(metrics["orphanFeedbackRecords"], 1);
        assert_eq!(metrics["actionRecords"], 0);
        assert_eq!(metrics["corrections"], 0);
        assert_eq!(metrics["ledgerIntegrityValid"], false);
    }

    #[test]
    fn active_run_metrics_reject_out_of_order_feedback_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = activate_test_dogfood_run(&root, "dogfood_current");
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for event in [
            json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 1),
                "outcome": "ask_user",
                "predictedOutcome": "proceed",
                "predictedSelectedOption": "biome"
            }),
            json!({
                "id": "action-1",
                "decisionId": "decision-1",
                "kind": "record-decision",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 2),
                "chosen": "biome",
                "wasAsked": false
            }),
            json!({
                "id": "feedback-1",
                "decisionId": "decision-1",
                "kind": "learn-feedback",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 3),
                "packetId": "packet-1"
            }),
            json!({
                "id": "apply-1",
                "kind": "apply-update",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 4),
                "packetId": "packet-1",
                "approved": true
            }),
            json!({
                "id": "preview-1",
                "kind": "preview-update",
                "dogfoodRunId": "dogfood_current",
                "createdAt": test_timestamp(started_at, 5),
                "packetId": "packet-1"
            }),
        ] {
            util::append_jsonl(&ledger, &event).unwrap();
        }

        let status = autopilot_status_value(&root).unwrap();
        let metrics = &status["shadowMetrics"];
        assert_eq!(metrics["packetLifecycleOrderViolations"], 1);
        assert_eq!(metrics["ledgerIntegrityValid"], false);
    }

    #[test]
    fn active_run_metrics_use_the_run_clock_for_observation_duration() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = chrono::DateTime::parse_from_rfc3339("2026-07-10T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        activate_test_dogfood_run_at(&root, "dogfood_current", started_at);
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-12T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        let metrics = shadow_metrics_value_at(&root, now).unwrap();

        assert_eq!(metrics["runId"], "dogfood_current");
        assert_eq!(metrics["observationStartedAt"], "2026-07-10T00:00:00Z");
        assert_eq!(metrics["observationEndedAt"], "2026-07-12T12:00:00Z");
        assert_eq!(metrics["observationDays"], 2.5);
        assert_eq!(metrics["firstDecisionAt"], serde_json::Value::Null);
        assert_eq!(metrics["lastDecisionAt"], serde_json::Value::Null);
    }

    #[test]
    fn active_run_metrics_reject_malformed_gate_timestamps() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let started_at = chrono::DateTime::parse_from_rfc3339("2026-07-10T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        activate_test_dogfood_run_at(&root, "dogfood_current", started_at);
        util::append_jsonl(
            &root.join("90-calibration/decision-ledger.jsonl"),
            &json!({
                "id": "decision-1",
                "kind": "decision-gate",
                "dogfoodRunId": "dogfood_current",
                "createdAt": "not-a-timestamp",
                "outcome": "ask_user",
                "predictedOutcome": "ask_user"
            }),
        )
        .unwrap();

        let error = shadow_metrics_value_at(
            &root,
            chrono::DateTime::parse_from_rfc3339("2026-07-12T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        )
        .unwrap_err();

        assert!(error.to_string().contains("invalid createdAt"), "{error:#}");
    }

    #[test]
    fn autopilot_promotion_rejects_incomplete_action_recording() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let ledger = root.join("90-calibration/decision-ledger.jsonl");
        for index in 0..30 {
            util::append_jsonl(
                &ledger,
                &json!({
                    "id": format!("decision-{index}"),
                    "kind": "decision-gate",
                    "outcome": "ask_user",
                    "predictedOutcome": "proceed",
                    "predictedSelectedOption": "biome"
                }),
            )
            .unwrap();
        }

        let error = autopilot_promote(Some(root.clone()), "conservative").unwrap_err();

        assert!(error.to_string().contains("ledger integrity"), "{error:#}");
        assert_eq!(autopilot_config(&root).mode, "shadow");
    }

    #[test]
    fn record_decision_never_persists_secret_text() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();

        record_decision(RecordDecisionArgs {
            decision_id: None,
            chosen: Some("api_key=abcdef1234567890".into()),
            was_asked: Some(false),
            vault: Some(root.clone()),
        })
        .unwrap();

        let ledger = fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl")).unwrap();
        assert!(!ledger.contains("abcdef1234567890"));
        assert!(ledger.contains("[REDACTED]"));
    }

    #[test]
    fn packet_filename_rejects_path_components() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();

        let err = learn_feedback(LearnFeedbackArgs {
            decision_id: "../escaped".into(),
            correction: Some("always ask before publishing".into()),
            chosen: None,
            rejected: None,
            incident: None,
            vault: Some(root.clone()),
        })
        .unwrap_err();

        assert!(err.to_string().contains("invalid decision id"));
        assert!(!root.join("99-meta/escaped").exists());
    }

    #[test]
    fn feedback_rejects_unknown_decision_id() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();

        let err = learn_feedback(LearnFeedbackArgs {
            decision_id: "dec_missing".into(),
            correction: Some("always ask before publishing".into()),
            chosen: None,
            rejected: None,
            incident: None,
            vault: Some(root.clone()),
        })
        .unwrap_err();

        assert!(err.to_string().contains("was not found"));
        assert_eq!(
            fs::read_dir(root.join("99-meta/pending-update-packets"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn decision_ids_reject_secret_shaped_values() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();

        let err = record_decision(RecordDecisionArgs {
            decision_id: Some("sk-abcdefghijklmnop".into()),
            chosen: Some("ask user".into()),
            was_asked: Some(true),
            vault: Some(root.clone()),
        })
        .unwrap_err();

        assert!(err.to_string().contains("secret-like"));
        assert!(
            fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn apply_rejects_unsafe_packet_id() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let mut forged = packet(
            "import",
            "decision-example",
            "When publishing, choose ask user",
            "strong",
            "personal",
            "create",
            None,
        );
        forged.id = "../../escaped".into();
        util::write_atomic(
            &root.join("99-meta/pending-update-packets/forged.json"),
            &serde_json::to_vec_pretty(&forged).unwrap(),
        )
        .unwrap();

        let err = apply(ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root),
        })
        .unwrap_err();

        assert!(err.to_string().contains("invalid packet id"));
    }

    #[test]
    fn apply_rejects_mislabeled_secret_packet() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let forged = packet(
            "import",
            "decision-example",
            "api_key=abcdef1234567890",
            "strong",
            "personal",
            "create",
            None,
        );
        util::write_atomic(
            &root.join("99-meta/pending-update-packets/forged.json"),
            &serde_json::to_vec_pretty(&forged).unwrap(),
        )
        .unwrap();

        let err = apply(ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root),
        })
        .unwrap_err();

        assert!(err.to_string().contains("secret-like"));
    }

    #[test]
    fn apply_rejects_secret_in_rendered_packet_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let mut forged = packet(
            "import",
            "decision-example",
            "When publishing, choose ask user",
            "strong",
            "personal",
            "create",
            None,
        );
        forged.suggested_links = vec!["api_key=abcdef1234567890".into()];
        util::write_atomic(
            &root.join("99-meta/pending-update-packets/forged.json"),
            &serde_json::to_vec_pretty(&forged).unwrap(),
        )
        .unwrap();

        let err = apply(ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root),
        })
        .unwrap_err();

        assert!(err.to_string().contains("secret-like"));
    }

    #[test]
    fn apply_rejects_unknown_packet_fields_before_archiving() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let forged = packet(
            "import",
            "decision-example",
            "When publishing, choose ask user",
            "strong",
            "personal",
            "create",
            None,
        );
        let mut value = serde_json::to_value(forged).unwrap();
        value["unknownField"] = serde_json::json!("unmodeled value");
        let packet_path = root.join("99-meta/pending-update-packets/forged.json");
        util::write_atomic(&packet_path, &serde_json::to_vec_pretty(&value).unwrap()).unwrap();

        let err = apply(ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root),
        })
        .unwrap_err();

        assert!(err.to_string().contains("unknown field"));
        assert!(packet_path.exists());
    }

    #[test]
    fn apply_skips_packets_already_marked_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        learn_decision(LearnDecisionArgs {
            situation: "publishing finished work".into(),
            options: "publish|ask user".into(),
            chosen: "ask user".into(),
            rejected: Some("publish".into()),
            rationale: None,
            decision_type: "workflow".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        let apply_args = || ApplyArgs {
            pending: false,
            yes: true,
            dry_run: false,
            vault: Some(root.clone()),
        };

        apply(apply_args()).unwrap();
        let names_after_first = pending_packet_names(&root);
        apply(apply_args()).unwrap();

        assert_eq!(pending_packet_names(&root), names_after_first);
        assert!(
            names_after_first
                .iter()
                .all(|name| !name.contains(".applied.applied.json"))
        );
    }

    #[test]
    fn applying_pending_packets_requires_explicit_yes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        learn_decision(LearnDecisionArgs {
            situation: "Choose package manager".into(),
            options: "npm|pnpm".into(),
            chosen: "pnpm".into(),
            rejected: Some("npm".into()),
            rationale: None,
            decision_type: "tooling".into(),
            scope: "global".into(),
            vault: Some(root.clone()),
        })
        .unwrap();
        let examples_before = fs::read_dir(root.join("60-decision-examples"))
            .unwrap()
            .count();

        apply(ApplyArgs {
            pending: true,
            yes: false,
            dry_run: false,
            vault: Some(root.clone()),
        })
        .unwrap();

        assert!(
            pending_packet_names(&root)
                .iter()
                .any(|name| name.ends_with(".json"))
        );
        assert_eq!(
            fs::read_dir(root.join("60-decision-examples"))
                .unwrap()
                .count(),
            examples_before
        );
    }

    fn pending_packet_names(root: &Path) -> Vec<String> {
        let mut names = fs::read_dir(root.join("99-meta/pending-update-packets"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
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
