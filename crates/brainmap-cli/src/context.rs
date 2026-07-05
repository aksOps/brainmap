use crate::cli::ContextArgs;
use crate::{index, vault};
use anyhow::Result;
use rusqlite::params;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ContextPack {
    mode: &'static str,
    source: &'static str,
    hot_path: HotPath,
    policies: Vec<ContextNote>,
    restrictions: Vec<ContextNote>,
    ask_triggers: Vec<ContextNote>,
}

#[derive(Debug, Serialize)]
struct HotPath {
    llm: bool,
    agent_memory: bool,
    network: bool,
    embedding_generation: bool,
    model_load: bool,
    full_vault_scan: bool,
}

#[derive(Debug, Serialize)]
struct ContextNote {
    path: String,
    title: String,
    note_type: String,
    risk_tier: String,
    sensitivity: String,
}

pub fn cmd_context(args: ContextArgs) -> Result<()> {
    let root = vault::resolve_vault(args.vault);
    let pack = load_fast_context(&root, args.limit)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        println!("Brainmap context pack ({})", pack.mode);
        for note in pack
            .policies
            .iter()
            .chain(pack.restrictions.iter())
            .chain(pack.ask_triggers.iter())
        {
            println!("- {} [{} {}]", note.path, note.note_type, note.risk_tier);
        }
    }
    Ok(())
}

pub(crate) fn load_fast_context(root: &std::path::Path, limit: usize) -> Result<ContextPack> {
    let conn = index::connection(root)?;
    Ok(ContextPack {
        mode: "decision-engine",
        source: "compiled-sqlite-index",
        hot_path: HotPath {
            llm: false,
            agent_memory: false,
            network: false,
            embedding_generation: false,
            model_load: false,
            full_vault_scan: false,
        },
        policies: select_notes(
            &conn,
            "decision-policy','tradeoff-rule','soft-preference','default-priority",
            limit,
        )?,
        restrictions: select_notes(&conn, "hard-constraint','approval-rule", limit)?,
        ask_triggers: select_notes(&conn, "ask-trigger','uncertainty-rule", limit)?,
    })
}

fn select_notes(
    conn: &rusqlite::Connection,
    types_csv: &str,
    limit: usize,
) -> Result<Vec<ContextNote>> {
    let sql = format!(
        "select path,title,note_type,risk_tier,sensitivity from notes where note_type in ('{types_csv}') order by path limit ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(ContextNote {
            path: row.get(0)?,
            title: row.get(1)?,
            note_type: row.get(2)?,
            risk_tier: row.get(3)?,
            sensitivity: row.get(4)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_context_reads_index() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        index::rebuild(&root).unwrap();
        let pack = load_fast_context(&root, 3).unwrap();
        assert_eq!(pack.source, "compiled-sqlite-index");
        assert!(!pack.policies.is_empty());
        assert!(!pack.restrictions.is_empty());
        assert!(!pack.hot_path.network);
    }
}
