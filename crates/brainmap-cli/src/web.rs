use crate::{index, markdown::Note, util, vault};
use anyhow::Result;
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

const WEB_HTML: &str = include_str!("../assets/web.html");

pub fn serve(vault: Option<PathBuf>, host: &str, port: u16, _open: bool) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let listener = TcpListener::bind((host, port))?;
    println!("Brainmap web UI: http://{host}:{port}");
    println!("read-only vault: {}", root.display());
    for stream in listener.incoming() {
        let stream = stream?;
        handle(stream, &root)?;
    }
    Ok(())
}

pub fn export_static(vault: Option<PathBuf>, out: PathBuf) -> Result<()> {
    let root = vault::resolve_vault(vault);
    fs::create_dir_all(&out)?;
    util::write_atomic(&out.join("index.html"), html(&root)?.as_bytes())?;
    util::write_atomic(&out.join("data.json"), ui_json(&root)?.as_bytes())?;
    println!("exported static web UI {}", out.display());
    Ok(())
}

fn handle(mut stream: TcpStream, root: &Path) -> Result<()> {
    let mut buf = [0u8; 8192];
    let n = stream.read(&mut buf)?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first = req.lines().next().unwrap_or_default();
    let parts = first.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 2 {
        respond(&mut stream, 400, "text/plain", "bad request")?;
        return Ok(());
    }
    let method = parts[0];
    let path = parts[1];
    if method != "GET" && method != "HEAD" {
        respond(
            &mut stream,
            405,
            "text/plain",
            "read-only: write methods are disabled",
        )?;
        return Ok(());
    }
    let (status, content_type, body) = route(root, path)?;
    if method == "HEAD" {
        respond(&mut stream, status, content_type, "")?;
    } else {
        respond(&mut stream, status, content_type, &body)?;
    }
    Ok(())
}

fn route(root: &Path, path: &str) -> Result<(u16, &'static str, String)> {
    if path == "/" || path == "/index.html" {
        return Ok((200, "text/html; charset=utf-8", html(root)?));
    }
    if path == "/api/status" {
        return Ok((200, "application/json", data_json(root)?));
    }
    if path == "/api/ui" {
        return Ok((200, "application/json", ui_json(root)?));
    }
    if path.starts_with("/api/search") {
        let q = query_param(path, "q").unwrap_or_else(|| "local".into());
        let results = index::search_text(root, &q, 12).unwrap_or_default();
        return Ok((
            200,
            "application/json",
            serde_json::to_string_pretty(&results)?,
        ));
    }
    if path == "/api/graph" {
        let data = json!({
            "sections": sections(),
            "edges": [
                ["Decision Identity","Tradeoff Models"],
                ["Tradeoff Models","Restrictions"],
                ["Restrictions","Question Triggers"],
                ["Choice Patterns","Examples"],
                ["Calibration","Examples"]
            ]
        });
        return Ok((
            200,
            "application/json",
            serde_json::to_string_pretty(&data)?,
        ));
    }
    Ok((404, "text/plain", "not found".into()))
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n{}",
        body.len(),
        body
    )?;
    Ok(())
}

fn data_json(root: &Path) -> Result<String> {
    let data = ui_payload(root)?;
    Ok(serde_json::to_string_pretty(&json!({
        "status": data["status"],
        "sections": data["sections"],
        "insights": data["insights"]
    }))?)
}

fn ui_json(root: &Path) -> Result<String> {
    Ok(serde_json::to_string_pretty(&ui_payload(root)?)?)
}

fn ui_payload(root: &Path) -> Result<Value> {
    let status = index::status(root).unwrap_or(index::IndexStatus {
        valid: false,
        path: index::db_path(root).display().to_string(),
        notes: 0,
        message: "index unavailable".into(),
    });
    let notes = vault::load_notes(root).unwrap_or_default();
    let decisions = decision_rows(root, 24);
    let autopilot = autopilot(root);
    let policy_count = count_notes(&notes, is_policy);
    let trigger_count = count_notes(&notes, is_trigger);
    let restriction_count = count_notes(&notes, is_restriction);
    let ask_count = decisions
        .iter()
        .filter(|d| d["outcome"].as_str() == Some("Ask User"))
        .count();
    let block_count = decisions
        .iter()
        .filter(|d| d["outcome"].as_str() == Some("Block"))
        .count();
    let coverage = percent(policy_count, status.notes);

    Ok(json!({
        "status": [
            if status.valid { "Local index valid" } else { "Local index missing" },
            format!("{} Mode", title_case(autopilot["mode"].as_str().unwrap_or("shadow"))),
            format!("Autopilot: {}", title_case(autopilot["level"].as_str().unwrap_or("conservative")))
        ],
        "sections": sections(),
        "metrics": [
            {
                "label": "Policy coverage",
                "value": format!("{coverage}%"),
                "note": format!("{policy_count} policy notes across {} indexed notes", status.notes)
            },
            {
                "label": "Decision samples",
                "value": decisions.len().to_string(),
                "note": "Append-only gate decisions from 90-calibration/decision-ledger.jsonl"
            },
            {
                "label": "Hard blocks",
                "value": block_count.to_string(),
                "note": format!("{restriction_count} restriction notes available")
            },
            {
                "label": "Question debt",
                "value": ask_count.to_string(),
                "note": format!("{trigger_count} question trigger notes available")
            }
        ],
        "tradeoffs": note_cards(&notes, is_tradeoff, 6, "name", "value"),
        "policies": note_cards(&notes, is_policy, 8, "id", "status"),
        "triggers": note_cards(&notes, is_trigger, 8, "id", "threshold"),
        "restrictions": note_cards(&notes, is_restriction, 8, "id", "severity"),
        "calibration": calibration_rows(&status, decisions.len(), ask_count, block_count),
        "decisions": decisions,
        "counts": {
            "policies": policy_count,
            "triggers": trigger_count,
            "restrictions": restriction_count,
            "decisions": decisions.len(),
            "coverage": coverage
        },
        "insights": {
            "notes": status.notes,
            "index": status.message,
            "autopilot": autopilot,
            "stalePolicies": 0,
            "coverage": coverage,
            "calibrationScore": if decisions.is_empty() { "untrained" } else { "ledger-derived" }
        }
    }))
}

fn sections() -> Vec<&'static str> {
    vec![
        "Decision Identity",
        "Tradeoff Models",
        "Restrictions",
        "Choice Patterns",
        "Question Triggers",
        "Calibration",
        "Examples",
    ]
}

fn html(_root: &Path) -> Result<String> {
    Ok(WEB_HTML.into())
}

fn note_cards(
    notes: &[Note],
    keep: fn(&Note) -> bool,
    limit: usize,
    name_key: &'static str,
    badge_key: &'static str,
) -> Vec<Value> {
    notes
        .iter()
        .filter(|note| keep(note))
        .take(limit)
        .map(|note| {
            json!({
                name_key: wikilink(note),
                badge_key: badge(note),
                "value": confidence_score(&note.confidence),
                "tone": tone(note),
                "text": excerpt(&note.body),
            })
        })
        .collect()
}

fn decision_rows(root: &Path, limit: usize) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(root.join("90-calibration/decision-ledger.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .take(limit)
        .map(|row| {
            let outcome = row.get("outcome").and_then(Value::as_str).unwrap_or("unknown");
            let confidence = row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
            let situation = row
                .get("situation")
                .and_then(Value::as_str)
                .unwrap_or("No situation recorded");
            let created = row
                .get("createdAt")
                .and_then(Value::as_str)
                .unwrap_or("unknown time");
            let id = row.get("id").and_then(Value::as_str).unwrap_or("decision");
            json!({
                "id": id,
                "title": compact(situation, 80),
                "outcome": title_case(outcome),
                "className": outcome_class(outcome),
                "confidence": (confidence * 100.0).round() as i64,
                "policy": row.get("primaryPolicy").and_then(Value::as_str).unwrap_or("not recorded"),
                "restriction": row.get("restriction").and_then(Value::as_str).unwrap_or("not recorded"),
                "latency": row.get("latency").and_then(Value::as_str).unwrap_or("n/a"),
                "score": (confidence * 100.0).round() as i64,
                "summary": situation,
                "reasons": [
                    format!("Outcome recorded as {outcome}."),
                    format!("Confidence recorded at {:.2}.", confidence)
                ],
                "timeline": [
                    format!("{created} decision gate appended ledger event"),
                    format!("{created} evaluated outcome {outcome}")
                ]
            })
        })
        .collect()
}

fn calibration_rows(
    status: &index::IndexStatus,
    decisions: usize,
    ask_count: usize,
    block_count: usize,
) -> Vec<Value> {
    let ask_block = ask_count + block_count;
    let ask_block_rate = percent(ask_block, decisions);
    vec![
        json!({
            "id": "Index health",
            "value": if status.valid { 100 } else { 0 },
            "tone": if status.valid { "green" } else { "danger" },
            "text": status.message
        }),
        json!({
            "id": "Decision samples",
            "value": decisions.min(100),
            "tone": if decisions > 0 { "green" } else { "warn" },
            "text": format!("{decisions} append-only gate events available")
        }),
        json!({
            "id": "Ask/block rate",
            "value": ask_block_rate,
            "tone": if ask_block_rate > 50 { "warn" } else { "green" },
            "text": format!("{ask_count} ask-user and {block_count} block outcomes")
        }),
    ]
}

fn autopilot(root: &Path) -> Value {
    let path = root.join(".brainmap/autopilot.json");
    let Ok(text) = fs::read_to_string(path) else {
        return json!({"mode": "shadow", "level": "conservative", "threshold": 0.82});
    };
    serde_json::from_str(&text)
        .unwrap_or_else(|_| json!({"mode": "shadow", "level": "conservative", "threshold": 0.82}))
}

fn count_notes(notes: &[Note], keep: fn(&Note) -> bool) -> usize {
    notes.iter().filter(|note| keep(note)).count()
}

fn is_policy(note: &Note) -> bool {
    let hay = haystack(note);
    hay.contains("policy") || hay.contains("decision-frame") || hay.contains("control")
}

fn is_tradeoff(note: &Note) -> bool {
    haystack(note).contains("tradeoff")
}

fn is_trigger(note: &Note) -> bool {
    let hay = haystack(note);
    hay.contains("question") || hay.contains("trigger") || hay.contains("ask")
}

fn is_restriction(note: &Note) -> bool {
    let hay = haystack(note);
    hay.contains("restriction") || hay.contains("hard-no") || hay.contains("approval")
}

fn haystack(note: &Note) -> String {
    format!("{} {} {}", note.path.display(), note.note_type, note.title).to_lowercase()
}

fn wikilink(note: &Note) -> String {
    format!("[[{}]]", note.path.with_extension("").display())
}

fn badge(note: &Note) -> String {
    if !note.status.is_empty() {
        note.status.clone()
    } else if !note.risk_tier.is_empty() {
        note.risk_tier.clone()
    } else {
        note.confidence.clone()
    }
}

fn tone(note: &Note) -> &'static str {
    let hay = format!("{} {}", note.risk_tier, note.status).to_lowercase();
    if hay.contains("hard") || hay.contains("never") {
        "danger"
    } else if hay.contains("ask") || hay.contains("approval") || hay.contains("medium") {
        "warn"
    } else {
        "green"
    }
}

fn confidence_score(confidence: &str) -> usize {
    match confidence.to_lowercase().as_str() {
        "high" => 90,
        "medium" => 65,
        "low" => 35,
        _ => 50,
    }
}

fn excerpt(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("---")
                && !line.starts_with("id:")
        })
        .map(|line| compact(line, 180))
        .unwrap_or_else(|| "No summary recorded.".into())
}

fn compact(text: &str, max: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        format!("{}...", one_line.chars().take(max).collect::<String>())
    }
}

fn percent(part: usize, total: usize) -> usize {
    part.checked_mul(100)
        .and_then(|value| value.checked_div(total))
        .unwrap_or(0)
        .min(100)
}

fn title_case(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn outcome_class(outcome: &str) -> &'static str {
    match outcome {
        "proceed" | "pass" => "pass",
        "block" => "block",
        "ask_user" | "needs_more_context" => "hold",
        _ => "inspect",
    }
}

fn query_param(path: &str, key: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = u8::from_str_radix(&input[i + 1..i + 3], 16)
        {
            out.push(hex);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into()
}
