use crate::{index, util, vault};
use anyhow::Result;
use serde_json::json;
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
    util::write_atomic(&out.join("data.json"), data_json(&root)?.as_bytes())?;
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
    let status = index::status(root).unwrap_or(index::IndexStatus {
        valid: false,
        path: index::db_path(root).display().to_string(),
        notes: 0,
        message: "index unavailable".into(),
    });
    let autopilot = serde_json::to_value(json!({
        "mode": "shadow",
        "level": "conservative",
        "threshold": 0.82
    }))?;
    Ok(serde_json::to_string_pretty(&json!({
        "status": ["Read-only", "Shadow Mode", "Autopilot: Conservative"],
        "sections": sections(),
        "insights": {
            "notes": status.notes,
            "index": status.message,
            "autopilot": autopilot,
            "stalePolicies": 0,
            "coverage": "seed",
            "calibrationScore": "untrained"
        }
    }))?)
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
