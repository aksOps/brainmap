use crate::{index, util, vault};
use anyhow::Result;
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

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

fn html(root: &Path) -> Result<String> {
    let data = data_json(root)?;
    Ok(format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Brainmap Decision Engine Explorer</title>
<style>
:root {{ color-scheme: dark; --bg:#08090b; --panel:#11151a; --line:#2a323c; --text:#f3f7fb; --muted:#9aa7b4; --accent:#54d2b8; --warn:#e7bd5e; --bad:#f07178; }}
* {{ box-sizing:border-box }}
body {{ margin:0; font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif; background:var(--bg); color:var(--text); }}
.shell {{ display:grid; grid-template-columns: 260px 1fr 340px; min-height:100vh; }}
aside {{ border-right:1px solid var(--line); padding:20px; background:#0c0f13; }}
main {{ padding:22px; }}
.right {{ border-left:1px solid var(--line); padding:20px; background:#0c0f13; }}
h1 {{ font-size:24px; margin:0 0 18px; letter-spacing:0; }}
h2 {{ font-size:15px; margin:0 0 12px; color:var(--muted); }}
button,.chip {{ border:1px solid var(--line); background:var(--panel); color:var(--text); border-radius:6px; padding:8px 10px; }}
button {{ display:block; width:100%; text-align:left; margin:7px 0; cursor:pointer; }}
button:focus {{ outline:2px solid var(--accent); outline-offset:2px; }}
.chips {{ display:flex; flex-wrap:wrap; gap:8px; margin-bottom:18px; }}
.chip {{ color:var(--accent); }}
.map {{ display:grid; place-items:center; min-height:520px; border:1px solid var(--line); background:#0a0d11; border-radius:8px; overflow:hidden; }}
svg {{ width:min(780px,100%); height:500px; }}
.node {{ fill:#151b21; stroke:var(--accent); stroke-width:1.6; cursor:pointer; }}
.node.active {{ fill:#1b312d; }}
.label {{ fill:var(--text); font-size:13px; text-anchor:middle; pointer-events:none; }}
.edge {{ stroke:#3b4652; stroke-width:1.5; }}
.card {{ border:1px solid var(--line); background:var(--panel); border-radius:8px; padding:14px; margin-bottom:12px; }}
.search {{ width:100%; border:1px solid var(--line); background:#07090c; color:var(--text); border-radius:6px; padding:10px; margin-bottom:12px; }}
.muted {{ color:var(--muted); }}
@media (max-width: 900px) {{ .shell {{ grid-template-columns:1fr; }} aside,.right {{ border:0; }} }}
</style>
</head>
<body>
<div class="shell">
<aside>
<h1>Brainmap Decision Engine Explorer</h1>
<div class="chips" id="chips"></div>
<h2>Ontology</h2>
<div id="sections"></div>
</aside>
<main>
<input class="search" id="search" placeholder="Search policies, tradeoffs, examples" aria-label="Search">
<div class="map">
<svg viewBox="0 0 760 500" role="img" aria-label="Brainmap section graph">
<g id="edges"></g><g id="nodes"></g>
</svg>
</div>
</main>
<section class="right">
<h2>Selected Policy Cards</h2>
<div id="cards"></div>
<h2>Engine Insights</h2>
<div id="insights" class="card"></div>
</section>
</div>
<script>
const DATA = {data};
const positions = [[360,70],[550,160],[510,330],[310,390],[170,300],[160,150],[360,240]];
const nodes = document.getElementById('nodes'), edges = document.getElementById('edges'), sections = document.getElementById('sections'), cards = document.getElementById('cards');
let active = DATA.sections[0];
document.getElementById('chips').innerHTML = DATA.status.map(s=>`<span class="chip">${{s}}</span>`).join('');
function draw() {{
  edges.innerHTML = DATA.sections.map((s,i)=> i ? `<line class="edge" x1="${{positions[i-1][0]}}" y1="${{positions[i-1][1]}}" x2="${{positions[i][0]}}" y2="${{positions[i][1]}}"></line>` : '').join('');
  nodes.innerHTML = DATA.sections.map((s,i)=>`<g tabindex="0" role="button" aria-label="${{s}}" onclick="select('${{s}}')" onkeydown="if(event.key==='Enter')select('${{s}}')"><ellipse class="node ${{s===active?'active':''}}" cx="${{positions[i][0]}}" cy="${{positions[i][1]}}" rx="92" ry="38"></ellipse><text class="label" x="${{positions[i][0]}}" y="${{positions[i][1]+5}}">${{s}}</text></g>`).join('');
  sections.innerHTML = DATA.sections.map(s=>`<button onclick="select('${{s}}')">${{s}}</button>`).join('');
  cards.innerHTML = `<div class="card"><strong>${{active}}</strong><p class="muted">Read-only policy section. Use CLI update packets to change canonical Markdown.</p></div><div class="card">Graph relationships and matching notes are loaded from the compiled SQLite index.</div>`;
}}
function select(s) {{ active=s; draw(); }}
document.getElementById('insights').textContent = JSON.stringify(DATA.insights, null, 2);
document.getElementById('search').addEventListener('change', async e => {{
  const res = await fetch('/api/search?q=' + encodeURIComponent(e.target.value || 'local'));
  const rows = await res.json();
  cards.innerHTML = rows.map(r=>`<div class="card"><strong>${{r.title}}</strong><p class="muted">${{r.path}}</p><p>${{r.snippet}}</p></div>`).join('') || '<div class="card">No results</div>';
}});
draw();
</script>
</body>
</html>"#
    ))
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
