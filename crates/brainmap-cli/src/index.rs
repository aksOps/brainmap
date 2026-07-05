use crate::{cli::SearchArgs, model, util, vault};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

pub fn db_path(root: &Path) -> PathBuf {
    root.join(".brainmap/brainmap.sqlite")
}

pub fn connection(root: &Path) -> Result<Connection> {
    let path = db_path(root);
    Connection::open(&path).with_context(|| format!("open {}", path.display()))
}

pub fn rebuild_cmd(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
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
    let mut conn = Connection::open(&tmp)?;
    create_schema(&mut conn)?;
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
    let tx = conn.transaction()?;
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
        "insert into index_manifest (key,value) values ('valid','true'),('createdAt',?1),('schemaVersion','decision-engine-v1')",
        params![util::now_iso()],
    )?;
    tx.commit()?;
    if final_path.exists() {
        fs::remove_file(&final_path)?;
    }
    fs::rename(&tmp, &final_path)?;
    util::write_atomic(
        &root.join(".brainmap/index-manifest.json"),
        serde_json::to_vec_pretty(&json!({
            "valid": true,
            "createdAt": util::now_iso(),
            "schemaVersion": "decision-engine-v1",
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
        insert into schema_migrations (version, applied_at) values (1, datetime('now'));
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
    let notes: i64 = conn.query_row("select count(*) from notes", [], |row| row.get(0))?;
    Ok(IndexStatus {
        valid: true,
        path: path.display().to_string(),
        notes: notes as usize,
        message: "valid index".into(),
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
        "select path,title,snippet(fts_notes, 2, '[', ']', '...', 12) from fts_notes where fts_notes match ?1 limit ?2",
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

pub fn policy_paths_for(root: &Path, words: &[&str]) -> Result<Vec<String>> {
    let conn = connection(root)?;
    let mut out = Vec::new();
    for word in words {
        let like = format!("%{}%", word.to_lowercase());
        let mut stmt = conn.prepare(
            "select path from notes where lower(title || ' ' || body) like ?1 order by path limit 4",
        )?;
        let rows = stmt.query_map(params![like], |row| row.get::<_, String>(0))?;
        for row in rows {
            let path = row?;
            let link = format!("[[{path}]]");
            if !out.contains(&link) {
                out.push(link);
            }
        }
    }
    Ok(out)
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
}
