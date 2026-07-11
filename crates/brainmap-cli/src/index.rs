use crate::{cli::SearchArgs, markdown, model, util, vault};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

pub const MAX_DECISION_QUERY_TERMS: usize = 16;
pub const MAX_DECISION_OPTIONS: usize = 32;
pub const MAX_DECISION_RULES: usize = 5_000;
pub const MAX_DECISION_ROWS_PER_TERM: usize = MAX_DECISION_RULES;
pub const MAX_DECISION_UNAVAILABLE_CHOICES: usize = 8;
pub const MAX_DECISION_FUZZY_ROWS: usize = MAX_DECISION_OPTIONS + MAX_DECISION_UNAVAILABLE_CHOICES;
pub const MAX_DECISION_EXACT_ROWS: usize = MAX_DECISION_OPTIONS + 1;
pub const MIN_DECISION_RULE_SCORE: f64 = 0.75;
pub const MIN_DECISION_OPTION_MISMATCH_SCORE: f64 = 0.9;
pub const MAX_AMBIGUOUS_MATCH_MARGIN: f64 = 0.1;
pub const COMPILED_SCHEMA_VERSION: &str = "decision-engine-v4";
pub const COMPILED_SCHEMA_MIGRATION: i64 = 4;

pub fn db_path(root: &Path) -> PathBuf {
    root.join(".brainmap/brainmap.sqlite")
}

pub fn connection(root: &Path) -> Result<Connection> {
    let path = db_path(root);
    Connection::open(&path).with_context(|| format!("open {}", path.display()))
}

pub fn rebuild_cmd(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let _maintenance = util::acquire_vault_maintenance(&root)?;
    rebuild(&root)?;
    println!("index rebuilt: {}", db_path(&root).display());
    Ok(())
}

pub fn rebuild(root: &Path) -> Result<()> {
    let lock_dir = root.join(".brainmap/locks");
    let _lock = util::FileLock::acquire(&lock_dir, "index.lock")?;
    fs::create_dir_all(root.join(".brainmap"))?;
    let tmp = root.join(".brainmap/brainmap.sqlite.tmp");
    let final_path = db_path(root);
    let _ = fs::remove_file(&tmp);
    let notes = vault::load_notes(root)?;
    let mut link_index = HashMap::new();
    for note in &notes {
        let path = note.path.to_string_lossy().to_string();
        link_index.insert(note.id.clone(), note.id.clone());
        link_index.insert(path.clone(), note.id.clone());
        if let Some(path_without_ext) = path.strip_suffix(".md") {
            link_index.insert(path_without_ext.to_string(), note.id.clone());
        }
    }
    {
        let mut conn = Connection::open(&tmp)?;
        create_schema(&mut conn)?;
        let tx = conn.transaction()?;
        let mut executable_ids = HashMap::<String, (String, bool)>::new();
        for note in &notes {
            tx.execute(
                "insert into notes (id,path,title,note_type,status,confidence,risk_tier,sensitivity,body) values (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    note.id,
                    note.path.to_string_lossy(),
                    note.title,
                    note.note_type,
                    note.status,
                    note.confidence,
                    note.risk_tier,
                    note.sensitivity,
                    note.body
                ],
            )?;
            tx.execute(
                "insert into fts_notes (path,title,body) values (?1,?2,?3)",
                params![note.path.to_string_lossy(), note.title, note.body],
            )?;
            tx.execute(
                "insert into graph_nodes (id,path,kind,title) values (?1,?2,?3,?4)",
                params![
                    note.id,
                    note.path.to_string_lossy(),
                    note.note_type,
                    note.title
                ],
            )?;
            let compiled_rule = match markdown::parse_decision_rule_result(&note.body) {
                Ok(rule) => rule,
                Err(error) if markdown::executable_rule_type_is_control(&note.note_type) => {
                    bail!(
                        "invalid executable control policy {}: {error}",
                        note.path.display()
                    )
                }
                Err(error) => {
                    eprintln!(
                        "warning: excluding invalid executable rule {}: {error}",
                        note.path.display()
                    );
                    None
                }
            };
            if let Some(rule) = compiled_rule {
                let rule_path = note.path.to_string_lossy().to_string();
                let situation_normalized = normalize_decision_text(&rule.situation);
                let chosen_normalized = normalize_decision_text(&rule.chosen);
                let decision_type = rule
                    .decision_type
                    .as_deref()
                    .or_else(|| note.frontmatter.get("decision_type").map(String::as_str))
                    .unwrap_or("general");
                let scope = rule
                    .scope
                    .as_deref()
                    .or_else(|| note.frontmatter.get("scope").map(String::as_str))
                    .unwrap_or("global");
                let metadata = match markdown::validate_executable_note(note, decision_type, scope)
                {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        if markdown::executable_rule_type_is_control(&note.note_type) {
                            bail!(
                                "invalid executable control policy {}: {error}",
                                note.path.display()
                            );
                        }
                        eprintln!(
                            "warning: excluding invalid executable rule {}: {error}",
                            note.path.display()
                        );
                        continue;
                    }
                };
                if let Some((previous_path, previous_is_control)) = executable_ids.get(&note.id) {
                    let error = format!(
                        "duplicate executable rule id {:?}; first declared by {previous_path}",
                        note.id
                    );
                    if metadata.is_control || *previous_is_control {
                        let control_path = if metadata.is_control {
                            rule_path.as_str()
                        } else {
                            previous_path.as_str()
                        };
                        bail!("invalid executable control policy {control_path}: {error}");
                    }
                    eprintln!(
                        "warning: excluding invalid executable rule {}: {error}",
                        note.path.display()
                    );
                    continue;
                }
                executable_ids.insert(note.id.clone(), (rule_path.clone(), metadata.is_control));
                ensure_decision_rule_capacity(executable_ids.len())?;
                let priority = metadata.priority;
                let base_confidence = confidence_value(&note.confidence);
                let evidence_count = note
                    .frontmatter
                    .get("evidence_count")
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or(1);
                let rule_terms = decision_tokens(&situation_normalized);
                let token_count = rule_terms.len() as i64;
                let rule_sequence = decision_token_sequence(&situation_normalized);
                let rule_anchors = decision_anchors(&rule_sequence);
                tx.execute(
                    "insert into decision_rules (rule_id,path,situation,situation_normalized,decision_type,scope,options,chosen,chosen_normalized,rejected,kind,priority,base_confidence,evidence_count,status,operation_anchor,domain_anchor) values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                    params![
                        note.id,
                        rule_path,
                        rule.situation,
                        situation_normalized,
                        decision_type,
                        scope,
                        serde_json::to_string(&rule.options)?,
                        rule.chosen,
                        chosen_normalized,
                        serde_json::to_string(&rule.rejected)?,
                        note.note_type,
                        priority,
                        base_confidence,
                        evidence_count,
                        metadata.status.as_str(),
                        rule_anchors.operation,
                        rule_anchors.domain,
                    ],
                )?;
                tx.execute(
                    "insert or ignore into decision_rule_choice_keys (situation_normalized,chosen_normalized,decision_type,scope,status) values (?1,?2,?3,?4,?5)",
                    params![
                        situation_normalized,
                        chosen_normalized,
                        decision_type,
                        scope,
                        metadata.status.as_str()
                    ],
                )?;
                for term in rule_terms {
                    tx.execute(
                        "insert into decision_rule_terms (term,chosen_normalized,decision_type,scope,status,token_count,priority,path) values (?1,?2,?3,?4,?5,?6,?7,?8)",
                        params![
                            term,
                            chosen_normalized,
                            decision_type,
                            scope,
                            metadata.status.as_str(),
                            token_count,
                            priority,
                            rule_path
                        ],
                    )?;
                }
            }
        }
        for note in &notes {
            for link in &note.links {
                let target = link_index
                    .get(link)
                    .cloned()
                    .unwrap_or_else(|| link.clone());
                tx.execute(
                    "insert into graph_edges (from_id,to_id,kind) values (?1,?2,'related')",
                    params![note.id, target],
                )?;
            }
        }
        tx.execute(
            "insert into index_manifest (key,value) values ('valid','true'),('createdAt',?1),('schemaVersion',?2)",
            params![util::now_iso(), COMPILED_SCHEMA_VERSION],
        )?;
        tx.commit()?;
    }
    let tmp_file = fs::File::open(&tmp)?;
    tmp_file.sync_all()?;
    drop(tmp_file);
    util::replace_file_atomic(&tmp, &final_path)?;
    #[cfg(unix)]
    fs::File::open(root.join(".brainmap"))?.sync_all()?;
    util::write_atomic(
        &root.join(".brainmap/index-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "valid": true,
            "createdAt": util::now_iso(),
            "schemaVersion": COMPILED_SCHEMA_VERSION,
            "notes": notes.len()
        }))?
        .as_slice(),
    )?;
    Ok(())
}

fn create_schema(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        pragma journal_mode = delete;
        create table schema_migrations (version integer primary key, applied_at text not null);
        insert into schema_migrations (version, applied_at) values (4, datetime('now'));
        create table notes (
            id text not null,
            path text primary key,
            title text not null,
            note_type text not null,
            status text not null,
            confidence text not null,
            risk_tier text not null,
            sensitivity text not null,
            body text not null
        );
        create table policies as select * from notes where 0;
        create table tradeoff_rules as select * from notes where 0;
        create table hard_restrictions as select * from notes where 0;
        create table soft_preferences as select * from notes where 0;
        create table approval_rules as select * from notes where 0;
        create table ask_triggers as select * from notes where 0;
        create table decision_examples as select * from notes where 0;
        create table counterexamples as select * from notes where 0;
        create table wrong_decisions as select * from notes where 0;
        create table corrected_decisions as select * from notes where 0;
        create table calibration_questions as select * from notes where 0;
        create table decision_ledger (id text primary key, created_at text not null, payload text not null);
        create table update_packets (id text primary key, created_at text not null, status text not null, payload text not null);
        create table decision_rules (
            rule_id text not null unique,
            path text primary key,
            situation text not null,
            situation_normalized text not null,
            decision_type text not null,
            scope text not null,
            options text not null,
            chosen text not null,
            chosen_normalized text not null,
            rejected text not null,
            kind text not null,
            priority integer not null,
            base_confidence real not null,
            evidence_count integer not null,
            status text not null,
            operation_anchor text,
            domain_anchor text
        );
        create index decision_rules_situation_idx on decision_rules(situation_normalized,chosen_normalized,decision_type,scope,status,priority desc,path desc);
        create index decision_rules_choice_idx on decision_rules(chosen_normalized,decision_type,scope,status,path);
        create table decision_rule_choice_keys (
            situation_normalized text not null,
            chosen_normalized text not null,
            decision_type text not null,
            scope text not null,
            status text not null,
            primary key (situation_normalized,chosen_normalized,decision_type,scope,status)
        ) without rowid;
        create table decision_rule_terms (
            term text not null,
            chosen_normalized text not null,
            decision_type text not null,
            scope text not null,
            status text not null,
            token_count integer not null,
            priority integer not null,
            path text not null,
            primary key (term,chosen_normalized,decision_type,scope,status,token_count,priority desc,path desc)
        ) without rowid;
        create table graph_nodes (id text not null, path text primary key, kind text not null, title text not null);
        create table graph_edges (from_id text not null, to_id text not null, kind text not null);
        create virtual table fts_notes using fts5(path, title, body);
        create table vector_embeddings (id text primary key, path text not null, model text not null, dimension integer not null, embedding blob not null);
        create table imports (id text primary key, created_at text not null, payload text not null);
        create table exports (id text primary key, created_at text not null, payload text not null);
        create table index_manifest (key text primary key, value text not null);
        "#,
    )?;
    Ok(())
}

pub fn status_cmd(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let status = status(&root)?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

pub fn verify_cmd(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let status = status(&root)?;
    if status.valid {
        println!("index verify ok: {} notes", status.notes);
        Ok(())
    } else {
        bail!("index invalid: {}", status.message)
    }
}

#[derive(Debug, Serialize)]
pub struct IndexStatus {
    pub valid: bool,
    pub path: String,
    pub notes: usize,
    pub message: String,
}

pub fn status(root: &Path) -> Result<IndexStatus> {
    let path = db_path(root);
    if !path.exists() {
        return Ok(IndexStatus {
            valid: false,
            path: path.display().to_string(),
            notes: 0,
            message: "missing index".into(),
        });
    }
    let conn = Connection::open(&path)?;
    let integrity: String = conn.query_row("pragma quick_check(1)", [], |row| row.get(0))?;
    let required_tables: i64 = conn.query_row(
        "select count(*) from sqlite_master where type='table' and name in ('notes','schema_migrations','decision_rules','decision_rule_choice_keys','decision_rule_terms','fts_notes','index_manifest')",
        [],
        |row| row.get(0),
    )?;
    if integrity != "ok" || required_tables != 7 {
        return Ok(IndexStatus {
            valid: false,
            path: path.display().to_string(),
            notes: 0,
            message: format!(
                "invalid index integrity or schema: quick_check={integrity}, required_tables={required_tables}/7"
            ),
        });
    }
    let notes: i64 = conn.query_row("select count(*) from notes", [], |row| row.get(0))?;
    let migration: Option<i64> =
        conn.query_row("select max(version) from schema_migrations", [], |row| {
            row.get(0)
        })?;
    let compiled_schema: Option<String> = conn
        .query_row(
            "select value from index_manifest where key='schemaVersion'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let compiled_valid: Option<String> = conn
        .query_row(
            "select value from index_manifest where key='valid'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let disk_manifest_valid = fs::read(root.join(".brainmap/index-manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|manifest| {
            manifest.get("valid").and_then(|value| value.as_bool()) == Some(true)
                && manifest
                    .get("schemaVersion")
                    .and_then(|value| value.as_str())
                    == Some(COMPILED_SCHEMA_VERSION)
                && manifest.get("notes").and_then(|value| value.as_i64()) == Some(notes)
        });
    let valid = migration == Some(COMPILED_SCHEMA_MIGRATION)
        && compiled_schema.as_deref() == Some(COMPILED_SCHEMA_VERSION)
        && compiled_valid.as_deref() == Some("true")
        && disk_manifest_valid;
    Ok(IndexStatus {
        valid,
        path: path.display().to_string(),
        notes: notes as usize,
        message: if valid {
            "valid decision-engine-v4 index".into()
        } else {
            "invalid index schema v4 metadata or disk manifest".into()
        },
    })
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub path: String,
    pub title: String,
    pub snippet: String,
}

pub fn search_text(root: &Path, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let conn = connection(root)?;
    let mut stmt = conn.prepare(
        "select fts_notes.path,fts_notes.title,snippet(fts_notes, 2, '[', ']', '...', 12) \
         from fts_notes join notes on notes.path = fts_notes.path \
         where fts_notes match ?1 and lower(notes.sensitivity) != 'secret' limit ?2",
    )?;
    let rows = stmt.query_map(params![query, limit as i64], |row| {
        Ok(SearchResult {
            path: row.get(0)?,
            title: row.get(1)?,
            snippet: row.get(2)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn search_cmd(args: SearchArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    if let Some(query) = args.text {
        let results = search_text(&root, &query, 20)?;
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }
    if let Some(query) = args.vector {
        let results = model::search_vector(&root, &query, 20)?;
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }
    if let Some(query) = args.hybrid {
        let text = search_text(&root, &query, 20)?;
        let vector = model::search_vector(&root, &query, 20)?;
        let graph_paths = text
            .iter()
            .map(|result| result.path.clone())
            .chain(vector.iter().map(|result| result.path.clone()))
            .take(10)
            .collect::<Vec<_>>();
        let graph = graph_neighborhood_for_paths(&root, &graph_paths, 30)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "text": text,
                "vector": vector,
                "graph": graph
            }))?
        );
        return Ok(());
    }
    bail!("provide --text, --vector, or --hybrid")
}

pub fn graph_neighbors_cmd(vault: Option<PathBuf>, id: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let conn = connection(&root)?;
    let mut stmt = conn.prepare(
        "select from_id,to_id,kind from graph_edges where from_id = ?1 or to_id = ?1 order by kind, to_id limit 50",
    )?;
    let rows = stmt.query_map(params![id], |row| {
        Ok(json!({
            "from": row.get::<_, String>(0)?,
            "to": row.get::<_, String>(1)?,
            "kind": row.get::<_, String>(2)?,
        }))
    })?;
    let values = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    println!("{}", serde_json::to_string_pretty(&values)?);
    Ok(())
}

pub fn graph_path_cmd(vault: Option<PathBuf>, from: &str, to: &str) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let conn = connection(&root)?;
    let path = graph_path(&conn, from, to)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "path": path }))?
    );
    Ok(())
}

fn graph_path(conn: &Connection, from: &str, to: &str) -> Result<Vec<String>> {
    if from == to {
        return Ok(vec![from.to_string()]);
    }
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut stmt = conn.prepare("select from_id,to_id from graph_edges")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (left, right) = row?;
        adjacency
            .entry(left.clone())
            .or_default()
            .push(right.clone());
        adjacency.entry(right).or_default().push(left);
    }
    let mut queue = VecDeque::from([from.to_string()]);
    let mut seen = HashSet::from([from.to_string()]);
    let mut previous: HashMap<String, String> = HashMap::new();
    while let Some(node) = queue.pop_front() {
        for next in adjacency.get(&node).into_iter().flatten() {
            if !seen.insert(next.clone()) {
                continue;
            }
            previous.insert(next.clone(), node.clone());
            if next == to {
                return Ok(reconstruct_path(from, to, &previous));
            }
            queue.push_back(next.clone());
        }
    }
    Ok(Vec::new())
}

fn reconstruct_path(from: &str, to: &str, previous: &HashMap<String, String>) -> Vec<String> {
    let mut path = vec![to.to_string()];
    let mut current = to;
    while current != from {
        let Some(prev) = previous.get(current) else {
            return Vec::new();
        };
        path.push(prev.clone());
        current = prev;
    }
    path.reverse();
    path
}

fn graph_neighborhood_for_paths(
    root: &Path,
    paths: &[String],
    limit: usize,
) -> Result<Vec<serde_json::Value>> {
    let conn = connection(root)?;
    let mut values = Vec::new();
    let mut seen_paths = HashSet::new();
    for path in paths {
        if !seen_paths.insert(path) {
            continue;
        }
        let Some(id) = conn
            .query_row(
                "select id from graph_nodes where path=?1",
                params![path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            continue;
        };
        let mut stmt = conn.prepare(
            "select from_id,to_id,kind from graph_edges where from_id = ?1 or to_id = ?1 order by kind, to_id limit ?2",
        )?;
        let rows = stmt.query_map(params![id, limit as i64], |row| {
            Ok(json!({
                "sourcePath": path,
                "from": row.get::<_, String>(0)?,
                "to": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
            }))
        })?;
        for row in rows {
            values.push(row?);
            if values.len() >= limit {
                return Ok(values);
            }
        }
    }
    Ok(values)
}

pub fn graph_orphans_cmd(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let conn = connection(&root)?;
    let mut stmt = conn.prepare(
        "select n.id,n.path from graph_nodes n where not exists (select 1 from graph_edges e where e.from_id=n.id or e.to_id=n.id) order by n.path",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "path": row.get::<_, String>(1)?,
        }))
    })?;
    let values = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    println!("{}", serde_json::to_string_pretty(&values)?);
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DecisionRuleMatch {
    pub rule_id: String,
    pub path: String,
    pub chosen: String,
    pub rejected: Vec<String>,
    pub score: f64,
    pub priority: i64,
    pub decision_type: String,
    pub scope: String,
    pub scope_exact: bool,
    pub base_confidence: f64,
    pub evidence_count: i64,
    pub match_kind: &'static str,
    pub margin: Option<f64>,
}

impl DecisionRuleMatch {
    pub fn calibrated_confidence(&self) -> f64 {
        let exact_bonus = if self.match_kind == "exact" {
            0.05
        } else {
            0.0
        };
        let evidence_bonus = (self.evidence_count.clamp(1, 4) as f64) * 0.01;
        let correction_bonus = if self.priority >= 300 { 0.02 } else { 0.0 };
        let scope_bonus = if self.scope_exact { 0.02 } else { 0.0 };
        let margin_bonus = self.margin.unwrap_or_default().clamp(0.0, 0.2) * 0.1;
        (0.4 + self.score * 0.45
            + self.base_confidence * 0.08
            + exact_bonus
            + evidence_bonus
            + correction_bonus
            + scope_bonus
            + margin_bonus)
            .min(0.97)
    }
}

#[derive(Debug, Clone)]
pub enum DecisionRuleResolution {
    NoMatch,
    Applicable(DecisionRuleMatch),
    Ambiguous {
        best: Box<DecisionRuleMatch>,
        alternative: Box<DecisionRuleMatch>,
    },
    OptionMismatch(DecisionRuleMatch),
}

type CompiledDecisionRuleRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    f64,
    i64,
    String,
);

fn read_compiled_decision_rule(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<CompiledDecisionRuleRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn compiled_match(
    row: CompiledDecisionRuleRow,
    score: f64,
    match_kind: &'static str,
    request_scope: &str,
) -> Result<DecisionRuleMatch> {
    let (
        rule_id,
        path,
        _situation,
        chosen,
        rejected,
        priority,
        rule_scope,
        base_confidence,
        evidence_count,
        decision_type,
    ) = row;
    Ok(DecisionRuleMatch {
        rule_id,
        path,
        chosen,
        rejected: serde_json::from_str(&rejected)?,
        score,
        priority,
        decision_type,
        scope_exact: rule_scope == request_scope,
        scope: rule_scope,
        base_confidence,
        evidence_count,
        match_kind,
        margin: None,
    })
}

pub fn resolve_decision_rule(
    root: &Path,
    situation: &str,
    decision_type: &str,
    scope: &str,
    options: &[String],
) -> Result<DecisionRuleResolution> {
    if options.len() > MAX_DECISION_OPTIONS {
        bail!("decision gate supports at most {MAX_DECISION_OPTIONS} options");
    }
    let conn = connection(root)?;
    let compiled_tables: i64 = conn.query_row(
        "select count(*) from sqlite_master where type='table' and name in ('decision_rules','decision_rule_terms')",
        [],
        |row| row.get(0),
    )?;
    if compiled_tables != 2 {
        return Ok(DecisionRuleResolution::NoMatch);
    }
    let normalized = normalize_decision_text(situation);
    if normalized.is_empty() {
        return Ok(DecisionRuleResolution::NoMatch);
    }
    let mut normalized_options = options
        .iter()
        .map(|option| normalize_decision_text(option))
        .collect::<Vec<_>>();
    normalized_options.sort();
    normalized_options.dedup();
    let options_json = serde_json::to_string(&normalized_options)?;

    let mut candidates = Vec::new();
    let exact_columns = "rule_id,path,situation,chosen,rejected,priority,scope,base_confidence,evidence_count,decision_type";
    let mut exact_available_stmt = conn.prepare(&format!(
        "select {exact_columns}
         from decision_rules
         where situation_normalized=?1
           and chosen_normalized=?2
           and (decision_type=?3 or decision_type='general')
           and (scope=?4 or scope='global')
           and status in (?5,?6,?7)
         order by (scope=?4) desc,(decision_type=?3) desc,priority desc,path desc
         limit 1"
    ))?;
    for choice in &normalized_options {
        if let Some(row) = exact_available_stmt
            .query_row(
                params![
                    &normalized,
                    choice,
                    decision_type,
                    scope,
                    markdown::ACTIVE_EXECUTABLE_STATUSES[0].as_str(),
                    markdown::ACTIVE_EXECUTABLE_STATUSES[1].as_str(),
                    markdown::ACTIVE_EXECUTABLE_STATUSES[2].as_str(),
                ],
                read_compiled_decision_rule,
            )
            .optional()?
        {
            candidates.push(compiled_match(row, 1.0, "exact", scope)?);
        }
    }
    if candidates.is_empty() {
        let mut exact_unavailable_choice_stmt = conn.prepare(
            "select chosen_normalized
         from decision_rule_choice_keys
         where situation_normalized=?1
           and chosen_normalized not in (select value from json_each(?2))
           and (decision_type=?3 or decision_type='general')
           and (scope=?4 or scope='global')
           and status in (?5,?6,?7)
         order by (scope=?4) desc,(decision_type=?3) desc,chosen_normalized
         limit 1",
        )?;
        if let Some(choice) = exact_unavailable_choice_stmt
            .query_row(
                params![
                    &normalized,
                    &options_json,
                    decision_type,
                    scope,
                    markdown::ACTIVE_EXECUTABLE_STATUSES[0].as_str(),
                    markdown::ACTIVE_EXECUTABLE_STATUSES[1].as_str(),
                    markdown::ACTIVE_EXECUTABLE_STATUSES[2].as_str(),
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            && let Some(row) = exact_available_stmt
                .query_row(
                    params![
                        &normalized,
                        choice,
                        decision_type,
                        scope,
                        markdown::ACTIVE_EXECUTABLE_STATUSES[0].as_str(),
                        markdown::ACTIVE_EXECUTABLE_STATUSES[1].as_str(),
                        markdown::ACTIVE_EXECUTABLE_STATUSES[2].as_str(),
                    ],
                    read_compiled_decision_rule,
                )
                .optional()?
        {
            candidates.push(compiled_match(row, 1.0, "exact", scope)?);
        }
    }
    debug_assert!(candidates.len() <= MAX_DECISION_EXACT_ROWS);
    if !candidates.is_empty() {
        return Ok(resolve_candidates(
            candidates,
            decision_type,
            scope,
            options,
        ));
    }

    let terms = bounded_decision_terms(&normalized);
    if terms.is_empty() {
        return Ok(DecisionRuleResolution::NoMatch);
    }
    let query_terms_json = serde_json::to_string(&terms)?;
    let query_term_count = terms.len() as i64;
    let input_sequence = decision_token_sequence(&normalized);
    let input_anchors = decision_anchors(&input_sequence);
    let mut fuzzy_stmt = conn.prepare(
        "with overlap_counts as (
             select r.rule_id,r.path,r.situation,r.chosen,r.rejected,r.priority,r.scope,
                    r.base_confidence,r.evidence_count,r.decision_type,r.chosen_normalized,
                    t.token_count,count(*) as overlap
             from decision_rule_terms t
             join decision_rules r on r.path=t.path
             where t.term in (select value from json_each(?1))
               and (r.decision_type=?2 or r.decision_type='general')
               and (r.scope=?3 or r.scope='global')
               and (?4 is null or r.operation_anchor is null or r.operation_anchor=?4)
               and (?5 is null or r.domain_anchor is null or r.domain_anchor=?5)
               and r.status in (?6,?7,?8)
             group by r.path
         ), scored as (
             select *,cast(overlap as real)/max(1,token_count+?9-overlap) as score
             from overlap_counts
             where overlap >= case when token_count=1 then 1 else 2 end
         ), eligible as (
             select * from scored where score>=?10
         ), choice_best as (
             select *,row_number() over (
                 partition by chosen_normalized
                 order by score desc,(scope=?3) desc,(decision_type=?2) desc,
                          priority desc,path desc
             ) as choice_rank
             from eligible
         ), available_choice_best as (
             select * from choice_best
             where choice_rank=1
               and chosen_normalized in (select value from json_each(?11))
         ), unavailable_choice_best as (
             select * from choice_best
             where choice_rank=1
               and chosen_normalized not in (select value from json_each(?11))
               and score>=?12
             order by score desc,(scope=?3) desc,(decision_type=?2) desc,
                      priority desc,chosen_normalized,path desc
             limit ?13
         ), selected_choice_best as (
             select * from available_choice_best
             union all
             select * from unavailable_choice_best
         )
         select rule_id,path,situation,chosen,rejected,priority,scope,base_confidence,
                evidence_count,decision_type,score
         from selected_choice_best
         order by score desc,(scope=?3) desc,(decision_type=?2) desc,
                  (chosen_normalized in (select value from json_each(?11))) desc,
                  priority desc,path desc
         limit ?14",
    )?;
    let rows = fuzzy_stmt.query_map(
        params![
            &query_terms_json,
            decision_type,
            scope,
            input_anchors.operation,
            input_anchors.domain,
            markdown::ACTIVE_EXECUTABLE_STATUSES[0].as_str(),
            markdown::ACTIVE_EXECUTABLE_STATUSES[1].as_str(),
            markdown::ACTIVE_EXECUTABLE_STATUSES[2].as_str(),
            query_term_count,
            MIN_DECISION_RULE_SCORE,
            &options_json,
            MIN_DECISION_OPTION_MISMATCH_SCORE,
            MAX_DECISION_UNAVAILABLE_CHOICES as i64,
            MAX_DECISION_FUZZY_ROWS as i64,
        ],
        |row| Ok((read_compiled_decision_rule(row)?, row.get::<_, f64>(10)?)),
    )?;
    for row in rows {
        let (row, query_score) = row?;
        let Some(score) = decision_rule_score(&row.2, situation) else {
            continue;
        };
        debug_assert!((query_score - score).abs() < f64::EPSILON * 8.0);
        candidates.push(compiled_match(row, score, "fuzzy", scope)?);
    }
    debug_assert!(candidates.len() <= MAX_DECISION_FUZZY_ROWS);
    Ok(resolve_candidates(
        candidates,
        decision_type,
        scope,
        options,
    ))
}

fn resolve_candidates(
    mut candidates: Vec<DecisionRuleMatch>,
    request_decision_type: &str,
    request_scope: &str,
    options: &[String],
) -> DecisionRuleResolution {
    if candidates.is_empty() {
        return DecisionRuleResolution::NoMatch;
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| (right.scope == request_scope).cmp(&(left.scope == request_scope)))
            .then_with(|| {
                (right.decision_type == request_decision_type)
                    .cmp(&(left.decision_type == request_decision_type))
            })
            .then_with(|| {
                choice_is_available(options, &right.chosen)
                    .cmp(&choice_is_available(options, &left.chosen))
            })
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| right.path.cmp(&left.path))
    });

    let mut best = candidates.remove(0);
    let best_choice = normalize_decision_text(&best.chosen);
    let alternative = candidates
        .iter()
        .find(|candidate| normalize_decision_text(&candidate.chosen) != best_choice)
        .cloned();
    best.margin = alternative
        .as_ref()
        .map(|candidate| (best.score - candidate.score).max(0.0));
    if let Some(alternative) = alternative
        && (best.score - alternative.score).abs() < MAX_AMBIGUOUS_MATCH_MARGIN
        && best.priority == alternative.priority
        && (best.scope == request_scope) == (alternative.scope == request_scope)
        && (best.decision_type == request_decision_type)
            == (alternative.decision_type == request_decision_type)
        && choice_is_available(options, &best.chosen)
            == choice_is_available(options, &alternative.chosen)
        && normalize_decision_text(&best.chosen) != normalize_decision_text(&alternative.chosen)
    {
        return DecisionRuleResolution::Ambiguous {
            best: Box::new(best),
            alternative: Box::new(alternative),
        };
    }

    if choice_is_available(options, &best.chosen) {
        DecisionRuleResolution::Applicable(best)
    } else if best.match_kind == "exact" || best.score >= MIN_DECISION_OPTION_MISMATCH_SCORE {
        DecisionRuleResolution::OptionMismatch(best)
    } else {
        DecisionRuleResolution::NoMatch
    }
}

fn choice_is_available(options: &[String], chosen: &str) -> bool {
    let chosen = normalize_decision_text(chosen);
    options
        .iter()
        .any(|option| normalize_decision_text(option) == chosen)
}

fn confidence_value(confidence: &str) -> f64 {
    match confidence.to_ascii_lowercase().as_str() {
        "high" | "very-strong" => 0.9,
        "low" | "weak" => 0.55,
        _ => 0.7,
    }
}

fn ensure_decision_rule_capacity(count: usize) -> Result<()> {
    if count > MAX_DECISION_RULES {
        bail!(
            "schema v4 supports at most {MAX_DECISION_RULES} executable rules; found at least {count}"
        );
    }
    Ok(())
}

fn decision_rule_score(rule_situation: &str, input_situation: &str) -> Option<f64> {
    let rule_normalized = normalize_decision_text(rule_situation);
    let input_normalized = normalize_decision_text(input_situation);
    if rule_normalized.is_empty() || input_normalized.is_empty() {
        return None;
    }
    let rule_sequence = decision_token_sequence(&rule_normalized);
    let input_sequence = decision_token_sequence(&input_normalized);
    let rule_anchors = decision_anchors(&rule_sequence);
    let input_anchors = decision_anchors(&input_sequence);
    if rule_anchors
        .operation
        .zip(input_anchors.operation)
        .is_some_and(|(rule, input)| rule != input)
        || rule_anchors
            .domain
            .zip(input_anchors.domain)
            .is_some_and(|(rule, input)| rule != input)
    {
        return None;
    }
    let rule_tokens = decision_tokens(&rule_normalized);
    let input_tokens = bounded_decision_terms(&input_normalized)
        .into_iter()
        .collect::<HashSet<_>>();
    let overlap = rule_tokens.intersection(&input_tokens).count();
    let minimum_overlap = if rule_tokens.len() == 1 { 1 } else { 2 };
    let union = rule_tokens.union(&input_tokens).count();
    let score = overlap as f64 / union.max(1) as f64;
    (overlap >= minimum_overlap && score >= MIN_DECISION_RULE_SCORE).then_some(score)
}

fn normalize_decision_text(text: &str) -> String {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn decision_tokens(text: &str) -> HashSet<String> {
    decision_token_sequence(text).into_iter().collect()
}

fn bounded_decision_terms(text: &str) -> Vec<String> {
    let mut terms = decision_tokens(text).into_iter().collect::<Vec<_>>();
    terms.sort();
    terms.truncate(MAX_DECISION_QUERY_TERMS);
    terms
}

fn decision_token_sequence(text: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "the", "and", "for", "with", "from", "into", "when", "then", "that", "this", "user",
        "what", "which", "should", "could", "would", "please", "tool",
    ];
    text.split_whitespace()
        .filter(|token| token.len() >= 3 && !STOPWORDS.contains(token))
        .map(|token| match token {
            "pick" | "select" | "use" | "using" | "decide" | "deciding" | "choosing" => "choose",
            "format" | "formatting" | "formatter" => "formatter",
            "repo" | "codebase" | "repository" => "repository",
            "rename" | "renaming" => "rename",
            "publish" | "publishing" => "publish",
            other => other,
        })
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct DecisionAnchors<'a> {
    operation: Option<&'a str>,
    domain: Option<&'a str>,
}

fn decision_anchors(tokens: &[String]) -> DecisionAnchors<'_> {
    const OPERATIONS: &[&str] = &[
        "choose",
        "rename",
        "publish",
        "delete",
        "deploy",
        "install",
        "configure",
        "upgrade",
        "migrate",
    ];
    const DOMAINS: &[&str] = &[
        "formatter",
        "database",
        "logging",
        "package",
        "test",
        "build",
        "linter",
        "deployment",
        "cache",
        "message",
        "serializer",
        "storage",
    ];
    DecisionAnchors {
        operation: tokens
            .iter()
            .map(String::as_str)
            .find(|token| *token != "choose" && OPERATIONS.contains(token))
            .or_else(|| {
                tokens
                    .iter()
                    .map(String::as_str)
                    .find(|token| *token == "choose")
            }),
        domain: tokens
            .iter()
            .map(String::as_str)
            .find(|token| DOMAINS.contains(token)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_index() {
        let tmp = tempfile::tempdir().unwrap();
        vault::init_vault(Some(tmp.path().join("BrainMap")), false, true).unwrap();
        let root = tmp.path().join("BrainMap");
        rebuild(&root).unwrap();
        let status = status(&root).unwrap();
        assert!(status.valid);
        assert!(status.notes > 20);
        assert!(!search_text(&root, "local", 5).unwrap().is_empty());
    }

    #[test]
    fn text_search_never_returns_secret_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        fs::write(
            root.join("secret-note.md"),
            "---\nid: secret-note\ntype: reference\nstatus: tested\nconfidence: high\nrisk_tier: suggest-only\nsensitivity: secret\n---\n# Secret\nultrasecretneedle\n",
        )
        .unwrap();
        rebuild(&root).unwrap();

        assert!(
            search_text(&root, "ultrasecretneedle", 20)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn graph_path_finds_multi_hop_paths() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_schema(&mut conn).unwrap();
        conn.execute(
            "insert into graph_edges (from_id,to_id,kind) values ('a','b','related'),('b','c','related')",
            [],
        )
        .unwrap();

        assert_eq!(graph_path(&conn, "a", "c").unwrap(), vec!["a", "b", "c"]);
        assert!(graph_path(&conn, "a", "missing").unwrap().is_empty());
    }

    #[test]
    fn schema_contains_compiled_decision_rules_table() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_schema(&mut conn).unwrap();

        let tables: i64 = conn
            .query_row(
                "select count(*) from sqlite_master where type='table' and name in ('decision_rules','decision_rule_choice_keys','decision_rule_terms')",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(tables, 3);

        let columns = conn
            .prepare("select name from pragma_table_info('decision_rules') order by cid")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for required in [
            "rule_id",
            "decision_type",
            "scope",
            "chosen_normalized",
            "base_confidence",
            "evidence_count",
            "status",
            "operation_anchor",
            "domain_anchor",
        ] {
            assert!(
                columns.iter().any(|column| column == required),
                "missing decision_rules.{required}: {columns:?}"
            );
        }

        let term_columns = conn
            .prepare("select name from pragma_table_info('decision_rule_terms') order by cid")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for required in [
            "term",
            "chosen_normalized",
            "decision_type",
            "scope",
            "status",
            "token_count",
            "priority",
            "path",
        ] {
            assert!(
                term_columns.iter().any(|column| column == required),
                "missing decision_rule_terms.{required}: {term_columns:?}"
            );
        }

        let version: i64 = conn
            .query_row("select max(version) from schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, COMPILED_SCHEMA_MIGRATION);
    }

    #[test]
    fn exact_lookup_plan_is_keyed_by_situation_and_choice() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_schema(&mut conn).unwrap();
        let details = conn
            .prepare(
                "explain query plan
                 select path from decision_rules
                 where situation_normalized=?1
                   and chosen_normalized=?2
                   and (decision_type=?3 or decision_type='general')
                   and (scope=?4 or scope='global')
                   and status in ('seed','tested','reliable')
                 order by (scope=?4) desc,(decision_type=?3) desc,priority desc,path desc
                 limit 1",
            )
            .unwrap()
            .query_map(
                params!["choose formatter", "biome", "tooling", "project:alpha"],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert!(
            details
                .iter()
                .any(|detail| detail.contains("situation_normalized")
                    && detail.contains("chosen_normalized")),
            "exact lookup is not keyed by situation and choice: {details:?}"
        );
    }

    #[test]
    fn fuzzy_lookup_plan_uses_term_postings_before_top_k() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_schema(&mut conn).unwrap();
        let details = conn
            .prepare(
                "explain query plan
                 select r.path
                 from decision_rule_terms t
                 join decision_rules r on r.path=t.path
                 where t.term in (select value from json_each(?1))
                   and (r.decision_type=?2 or r.decision_type='general')
                   and (r.scope=?3 or r.scope='global')
                   and r.status in ('seed','tested','reliable')
                 group by r.path
                 order by count(*) desc,r.priority desc,r.path desc
                 limit 40",
            )
            .unwrap()
            .query_map(
                params![r#"["formatter","repository"]"#, "tooling", "project:alpha"],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert!(
            details.iter().any(|detail| {
                detail.contains("SEARCH t USING PRIMARY KEY") && detail.contains("term=?)")
            }),
            "bounded fuzzy lookup did not use the term posting index: {details:?}"
        );
    }

    #[test]
    fn rebuild_stably_compiles_actual_rule_postings_and_anchors() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let examples = root.join("60-decision-examples");
        fs::write(
            examples.join("choice-postings-alpha.md"),
            r#"---
id: choice-postings-alpha
type: decision-example
status: tested
confidence: high
risk_tier: reversible-auto
sensitivity: personal
---
# Alpha formatter

<!-- brainmap-decision-rule:v1 {"situation":"Choose formatter for alpha repository","decision_type":"tooling","scope":"project:alpha","options":["rustfmt","zeta formatter"],"chosen":"zeta formatter","rejected":["rustfmt"]} -->
"#,
        )
        .unwrap();
        fs::write(
            examples.join("choice-postings-beta.md"),
            r#"---
id: choice-postings-beta
type: decision-example
status: tested
confidence: high
risk_tier: reversible-auto
sensitivity: personal
---
# Beta formatter

<!-- brainmap-decision-rule:v1 {"situation":"Select formatter for beta codebase","decision_type":"tooling","scope":"project:alpha","options":["rustfmt","zeta formatter"],"chosen":"zeta formatter","rejected":["rustfmt"]} -->
"#,
        )
        .unwrap();

        let read_compiled = || {
            let conn = connection(&root).unwrap();
            let postings = conn
                .prepare(
                    "select path,term from decision_rule_terms
                     where path like '60-decision-examples/choice-postings-%'
                     order by path,term",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            let anchors = conn
                .prepare(
                    "select path,operation_anchor,domain_anchor from decision_rules
                     where path like '60-decision-examples/choice-postings-%'
                     order by path",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (postings, anchors)
        };

        rebuild(&root).unwrap();
        let first = read_compiled();
        rebuild(&root).unwrap();
        let second = read_compiled();

        assert_eq!(
            first.0,
            [
                (
                    "60-decision-examples/choice-postings-alpha.md".into(),
                    "alpha".into()
                ),
                (
                    "60-decision-examples/choice-postings-alpha.md".into(),
                    "choose".into()
                ),
                (
                    "60-decision-examples/choice-postings-alpha.md".into(),
                    "formatter".into()
                ),
                (
                    "60-decision-examples/choice-postings-alpha.md".into(),
                    "repository".into()
                ),
                (
                    "60-decision-examples/choice-postings-beta.md".into(),
                    "beta".into()
                ),
                (
                    "60-decision-examples/choice-postings-beta.md".into(),
                    "choose".into()
                ),
                (
                    "60-decision-examples/choice-postings-beta.md".into(),
                    "formatter".into()
                ),
                (
                    "60-decision-examples/choice-postings-beta.md".into(),
                    "repository".into()
                ),
            ]
        );
        assert_eq!(
            first.1,
            [
                (
                    "60-decision-examples/choice-postings-alpha.md".into(),
                    Some("choose".into()),
                    Some("formatter".into())
                ),
                (
                    "60-decision-examples/choice-postings-beta.md".into(),
                    Some("choose".into()),
                    Some("formatter".into())
                ),
            ]
        );
        assert_eq!(second, first);
    }

    #[test]
    fn status_rejects_an_incomplete_database_that_only_has_notes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        fs::create_dir_all(root.join(".brainmap")).unwrap();
        let connection = Connection::open(db_path(&root)).unwrap();
        connection
            .execute_batch("create table notes (id text); insert into notes values ('fake');")
            .unwrap();
        drop(connection);

        let status = status(&root).unwrap();
        assert!(!status.valid);
        assert!(status.message.contains("schema"));
    }

    #[test]
    fn status_rejects_a_legacy_v3_compiled_index() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        fs::create_dir_all(root.join(".brainmap")).unwrap();
        let mut connection = Connection::open(db_path(&root)).unwrap();
        create_schema(&mut connection).unwrap();
        connection
            .execute("update schema_migrations set version=3", [])
            .unwrap();
        connection
            .execute(
                "insert into index_manifest (key,value) values ('valid','true'),('schemaVersion','decision-engine-v3')",
                [],
            )
            .unwrap();
        drop(connection);
        util::write_atomic(
            &root.join(".brainmap/index-manifest.json"),
            br#"{"valid":true,"schemaVersion":"decision-engine-v3","notes":0}"#,
        )
        .unwrap();

        let status = status(&root).unwrap();

        assert!(!status.valid);
        assert!(status.message.contains("schema v4"));
    }

    #[test]
    fn short_generic_input_does_not_match_longer_rule() {
        assert_eq!(
            decision_rule_score("renaming local temporary notes", "local"),
            None
        );
    }

    #[test]
    fn decision_rule_resolution_rejects_unbounded_option_sets() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        rebuild(&root).unwrap();
        let options = (0..=MAX_DECISION_OPTIONS)
            .map(|index| format!("option-{index}"))
            .collect::<Vec<_>>();

        let error = resolve_decision_rule(
            &root,
            "Choose bounded option",
            "tooling",
            "project:bounded",
            &options,
        )
        .unwrap_err();

        assert!(error.to_string().contains("at most 32 options"));
    }

    #[test]
    fn schema_v4_candidate_bounds_cover_the_supported_envelope() {
        assert_eq!(MAX_DECISION_RULES, 5_000);
        assert_eq!(MAX_DECISION_ROWS_PER_TERM, 5_000);
        assert_eq!(MAX_DECISION_FUZZY_ROWS, 40);
        ensure_decision_rule_capacity(MAX_DECISION_RULES).unwrap();
        let error = ensure_decision_rule_capacity(MAX_DECISION_RULES + 1).unwrap_err();
        assert!(error.to_string().contains("at most 5000 executable rules"));

        let input = (0..20)
            .map(|index| format!("term{index:02}"))
            .collect::<Vec<_>>()
            .join(" ");
        let terms = bounded_decision_terms(&input);
        assert_eq!(terms.len(), MAX_DECISION_QUERY_TERMS);
        assert!(terms.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(terms.first().map(String::as_str), Some("term00"));
        assert_eq!(terms.last().map(String::as_str), Some("term15"));
    }

    #[test]
    fn domain_anchor_mismatch_blocks_high_overlap() {
        assert_eq!(
            decision_rule_score(
                "Choose formatter for Rust repository alpha beta gamma",
                "Choose database for Rust repository alpha beta gamma",
            ),
            None
        );
    }

    #[test]
    fn operation_anchor_mismatch_blocks_high_overlap() {
        assert_eq!(
            decision_rule_score(
                "Rename formatter config in Rust repository alpha beta gamma",
                "Choose formatter config in Rust repository alpha beta gamma",
            ),
            None
        );
    }

    #[test]
    fn confidence_uses_scope_compatibility_and_match_margin() {
        let baseline = DecisionRuleMatch {
            rule_id: "baseline".into(),
            path: "baseline.md".into(),
            chosen: "biome".into(),
            rejected: vec!["prettier".into()],
            score: MIN_DECISION_RULE_SCORE,
            priority: 100,
            decision_type: "tooling".into(),
            scope: "global".into(),
            scope_exact: false,
            base_confidence: 0.7,
            evidence_count: 1,
            match_kind: "fuzzy",
            margin: Some(0.0),
        };
        let scoped = DecisionRuleMatch {
            scope_exact: true,
            ..baseline.clone()
        };
        let separated = DecisionRuleMatch {
            margin: Some(0.2),
            ..scoped.clone()
        };

        assert!(scoped.calibrated_confidence() > baseline.calibrated_confidence());
        assert!(separated.calibrated_confidence() > scoped.calibrated_confidence());
        assert!(separated.calibrated_confidence() < 0.9);
    }

    #[test]
    fn malformed_executable_control_policy_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let path = root.join("40-restrictions/malformed-control.md");
        let body = format!(
            "{}# Malformed Control\n\n<!-- brainmap-decision-rule:v1 {{not-json}} -->\n",
            markdown::frontmatter(
                "malformed-control",
                "hard-constraint",
                "never-auto",
                "private",
            )
        );
        util::write_atomic(&path, body.as_bytes()).unwrap();

        let error = rebuild(&root).unwrap_err();
        assert!(error.to_string().contains("malformed-control.md"));
        assert!(
            error
                .to_string()
                .contains("invalid executable control policy")
        );
    }

    #[test]
    fn malformed_non_control_rule_is_excluded_without_disabling_the_index() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let path = root.join("60-decision-examples/malformed-example.md");
        let body = format!(
            "{}# Malformed Example\n\n<!-- brainmap-decision-rule:v1 {{not-json}} -->\n",
            markdown::frontmatter(
                "malformed-example",
                "decision-example",
                "ask-before-action",
                "personal",
            )
        );
        util::write_atomic(&path, body.as_bytes()).unwrap();

        rebuild(&root).unwrap();
        assert!(matches!(
            resolve_decision_rule(
                &root,
                "Malformed Example",
                "general",
                "global",
                &["A".into(), "B".into()],
            )
            .unwrap(),
            DecisionRuleResolution::NoMatch
        ));
    }

    #[test]
    fn unknown_control_status_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let path = root.join("40-restrictions/unknown-status-control.md");
        let frontmatter = markdown::frontmatter(
            "unknown-status-control",
            "hard-constraint",
            "never-auto",
            "private",
        )
        .replace("status: seed", "status: experimental");
        let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
            situation: "Publish private credentials".into(),
            decision_type: Some("workflow".into()),
            scope: Some("global".into()),
            options: vec!["publish".into(), "block".into()],
            chosen: "block".into(),
            rejected: vec!["publish".into()],
        })
        .unwrap();
        util::write_atomic(
            &path,
            format!("{frontmatter}# Unknown Status Control\n\n{marker}\n").as_bytes(),
        )
        .unwrap();

        let error = rebuild(&root).unwrap_err();
        assert!(error.to_string().contains("unknown-status-control.md"));
        assert!(error.to_string().contains("unsupported executable status"));
    }

    #[test]
    fn unsupported_non_control_rule_type_is_excluded() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let path = root.join("60-decision-examples/plugin-rule.md");
        let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
            situation: "Choose plugin formatter".into(),
            decision_type: Some("tooling".into()),
            scope: Some("project:plugin".into()),
            options: vec!["biome".into(), "prettier".into()],
            chosen: "biome".into(),
            rejected: vec!["prettier".into()],
        })
        .unwrap();
        util::write_atomic(
            &path,
            format!(
                "{}# Plugin Rule\n\n{marker}\n",
                markdown::frontmatter(
                    "plugin-rule",
                    "unsupported-plugin-rule",
                    "ask-before-action",
                    "personal",
                )
            )
            .as_bytes(),
        )
        .unwrap();

        rebuild(&root).unwrap();
        assert!(matches!(
            resolve_decision_rule(
                &root,
                "Choose plugin formatter",
                "tooling",
                "project:plugin",
                &["biome".into(), "prettier".into()],
            )
            .unwrap(),
            DecisionRuleResolution::NoMatch
        ));
    }

    #[test]
    fn executable_control_policy_requires_explicit_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
            situation: "Choose whether to publish credentials".into(),
            decision_type: Some("workflow".into()),
            scope: Some("global".into()),
            options: vec!["publish".into(), "block".into()],
            chosen: "block".into(),
            rejected: vec!["publish".into()],
        })
        .unwrap();
        util::write_atomic(
            &root.join("40-restrictions/missing-frontmatter.md"),
            format!("---\nid: missing-frontmatter\n---\n# Missing Fields\n\n{marker}\n").as_bytes(),
        )
        .unwrap();

        let error = rebuild(&root).unwrap_err();
        assert!(error.to_string().contains("missing-frontmatter.md"));
        assert!(
            error
                .to_string()
                .contains("missing required executable frontmatter field")
        );
    }

    #[test]
    fn duplicate_executable_control_ids_fail_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
            situation: "Choose whether to publish duplicate credentials".into(),
            decision_type: Some("workflow".into()),
            scope: Some("global".into()),
            options: vec!["publish".into(), "block".into()],
            chosen: "block".into(),
            rejected: vec!["publish".into()],
        })
        .unwrap();
        for name in ["duplicate-control-a.md", "duplicate-control-b.md"] {
            util::write_atomic(
                &root.join("40-restrictions").join(name),
                format!(
                    "{}# Duplicate Control\n\n{marker}\n",
                    markdown::frontmatter(
                        "duplicate-control-id",
                        "hard-constraint",
                        "never-auto",
                        "private"
                    )
                )
                .as_bytes(),
            )
            .unwrap();
        }

        let error = rebuild(&root).unwrap_err();
        assert!(error.to_string().contains("duplicate executable rule id"));
        assert!(error.to_string().contains("duplicate-control-id"));
    }

    #[test]
    fn mixed_duplicate_ids_fail_closed_regardless_of_path_order() {
        let marker = markdown::decision_rule_marker(&markdown::DecisionRule {
            situation: "Choose whether to publish mixed duplicate credentials".into(),
            decision_type: Some("workflow".into()),
            scope: Some("global".into()),
            options: vec!["publish".into(), "block".into()],
            chosen: "block".into(),
            rejected: vec!["publish".into()],
        })
        .unwrap();

        for control_first in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("BrainMap");
            vault::init_vault(Some(root.clone()), false, true).unwrap();
            let (control_path, example_path) = if control_first {
                (
                    "40-restrictions/a-mixed-control.md",
                    "60-decision-examples/z-mixed-example.md",
                )
            } else {
                (
                    "40-restrictions/z-mixed-control.md",
                    "20-decision-frames/a-mixed-example.md",
                )
            };
            for (path, note_type, risk_tier) in [
                (control_path, "hard-constraint", "never-auto"),
                (example_path, "decision-example", "ask-before-action"),
            ] {
                util::write_atomic(
                    &root.join(path),
                    format!(
                        "{}# Mixed Duplicate\n\n{marker}\n",
                        markdown::frontmatter(
                            "mixed-duplicate-id",
                            note_type,
                            risk_tier,
                            "private"
                        )
                    )
                    .as_bytes(),
                )
                .unwrap();
            }

            let error = rebuild(&root).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("invalid executable control policy")
            );
            assert!(error.to_string().contains("mixed-duplicate-id"));
        }
    }
}
