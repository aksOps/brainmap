use crate::{index, util, vault};
use anyhow::{Context, Result, bail};
use model2vec_rs::model::StaticModel;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use std::cmp::Ordering;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const MODEL_ID: &str = "minishlab/potion-base-8M";
pub(crate) const DIMENSION: usize = 256;
const PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/default.brainmap-model.tar.zst"));
const PACK_SHA256: &str = env!("BRAINMAP_MODEL_PACK_SHA256");

pub fn models_status(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let hash = pack_hash();
    let dir = model_dir(&root, hash);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "model": MODEL_ID,
            "dimension": DIMENSION,
            "embeddedPackSha256": hash,
            "materialized": dir.exists(),
            "runtimeDownloadAllowed": false,
            "externalProvidersAllowed": false
        }))?
    );
    Ok(())
}

pub fn models_materialize(vault: Option<PathBuf>, force: bool) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let (dir, changed) = materialize_model(&root, force)?;
    if changed {
        println!("materialized model {}", dir.display());
    } else {
        println!("model already materialized: {}", dir.display());
    }
    Ok(())
}

pub(crate) fn materialize_model(root: &Path, force: bool) -> Result<(PathBuf, bool)> {
    let hash = pack_hash();
    let dir = model_dir(root, hash);
    if dir.exists() && !force && verify_materialized_dir(&dir, hash).is_ok() {
        return Ok((dir, false));
    }
    let tmp = root.join(".brainmap/models/.tmp-default-model");
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp)?;
    extract_pack(&tmp)?;
    let extracted = tmp.join("potion-base-8M");
    verify_materialized_dir(&extracted, hash)?;
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(dir.parent().context("model dir has no parent")?)?;
    fs::rename(&extracted, &dir)?;
    let _ = fs::remove_dir_all(&tmp);
    Ok((dir, true))
}

pub fn models_verify(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let hash = pack_hash();
    let dir = model_dir(&root, hash);
    if !dir.exists() {
        bail!("model not materialized; run models materialize")
    }
    verify_materialized_dir(&dir, hash)?;
    println!("model verify ok: {MODEL_ID} {hash}");
    Ok(())
}

pub fn models_info() -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "model": MODEL_ID,
            "type": "Model2Vec static embedding model",
            "dimension": DIMENSION,
            "packPath": "build-time downloaded pack embedded in binary",
            "packKind": "build-time-download",
            "source": "https://huggingface.co/minishlab/potion-base-8M",
            "runtimeDownloadAllowed": false
        }))?
    );
    Ok(())
}

pub fn embed_rebuild(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let count = embed_notes(&root, false)?;
    println!("embedded {count} note(s) with local Model2Vec pack");
    Ok(())
}

pub fn embed_process(vault: Option<PathBuf>, missing_only: bool) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let count = embed_notes(&root, missing_only)?;
    println!("processed {count} note embedding(s); no external providers");
    Ok(())
}

pub fn embed_status(vault: Option<PathBuf>) -> Result<()> {
    let root = vault::resolve_vault(vault);
    let embedded_notes = embedding_count(&root).unwrap_or(0);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "enabled": true,
            "provider": "embedded-model2vec",
            "model": MODEL_ID,
            "dimension": DIMENSION,
            "generateInHotPath": false,
            "materialized": model_dir(&root, pack_hash()).exists(),
            "embeddedNotes": embedded_notes
        }))?
    );
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct VectorSearchResult {
    pub path: String,
    pub title: String,
    pub score: f32,
}

pub fn search_vector(root: &Path, query: &str, limit: usize) -> Result<Vec<VectorSearchResult>> {
    let model = load_materialized_model(root)?;
    let query_embedding = model.encode_single(query);
    ensure_dimension(&query_embedding)?;
    let conn = index::connection(root)?;
    let mut stmt = conn.prepare(
        "select v.path,n.title,v.embedding from vector_embeddings v join notes n on n.path=v.path where v.model=?1 and v.dimension=?2",
    )?;
    let rows = stmt.query_map(params![MODEL_ID, DIMENSION as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    let mut results = Vec::new();
    for row in rows {
        let (path, title, blob) = row?;
        let embedding = blob_to_embedding(&blob)?;
        ensure_dimension(&embedding)?;
        let score = cosine_similarity(&query_embedding, &embedding);
        if score.is_finite() {
            results.push(VectorSearchResult { path, title, score });
        }
    }
    if results.is_empty() {
        bail!("no vector embeddings found; run brainmap embed rebuild")
    }
    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    results.truncate(limit);
    Ok(results)
}

pub(crate) fn embed_notes(root: &Path, missing_only: bool) -> Result<usize> {
    let model = load_materialized_model(root)?;
    if !index::db_path(root).exists() {
        index::rebuild(root)?;
    }
    let mut conn = index::connection(root)?;
    let rows = note_rows_for_embedding(&conn, missing_only)?;
    if rows.is_empty() {
        return Ok(0);
    }
    let texts = rows
        .iter()
        .map(|(_, text)| text.clone())
        .collect::<Vec<_>>();
    let embeddings = model.encode(&texts);
    if embeddings.len() != rows.len() {
        bail!("embedding count mismatch")
    }
    let tx = conn.transaction()?;
    if !missing_only {
        tx.execute(
            "delete from vector_embeddings where model=?1",
            params![MODEL_ID],
        )?;
    }
    {
        let mut insert = tx.prepare(
            "insert or replace into vector_embeddings (id,path,model,dimension,embedding) values (?1,?2,?3,?4,?5)",
        )?;
        for ((path, _), embedding) in rows.iter().zip(embeddings.iter()) {
            ensure_dimension(embedding)?;
            insert.execute(params![
                format!("{MODEL_ID}:{path}"),
                path,
                MODEL_ID,
                DIMENSION as i64,
                embedding_to_blob(embedding)
            ])?;
        }
    }
    tx.commit()?;
    Ok(rows.len())
}

fn note_rows_for_embedding(
    conn: &rusqlite::Connection,
    missing_only: bool,
) -> Result<Vec<(String, String)>> {
    if missing_only {
        let mut stmt = conn.prepare(
            "select n.path,n.title || char(10) || n.body from notes n where not exists (select 1 from vector_embeddings v where v.path=n.path and v.model=?1) order by n.path",
        )?;
        let rows = stmt.query_map(params![MODEL_ID], |row| Ok((row.get(0)?, row.get(1)?)))?;
        return rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into);
    }
    let mut stmt =
        conn.prepare("select path,title || char(10) || body from notes order by path")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub(crate) fn embedding_count(root: &Path) -> Result<usize> {
    if !index::db_path(root).exists() {
        return Ok(0);
    }
    let conn = index::connection(root)?;
    let count: i64 = conn.query_row(
        "select count(*) from vector_embeddings where model=?1",
        params![MODEL_ID],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

fn load_materialized_model(root: &Path) -> Result<StaticModel> {
    let dir = materialized_model_dir(root)?;
    StaticModel::from_pretrained(&dir, None, None, None)
        .with_context(|| format!("load materialized model {}", dir.display()))
}

fn materialized_model_dir(root: &Path) -> Result<PathBuf> {
    let hash = pack_hash();
    let dir = model_dir(root, hash);
    if !dir.exists() {
        bail!("embedded model pack not materialized; run brainmap models materialize")
    }
    verify_materialized_dir(&dir, hash)?;
    Ok(dir)
}

fn ensure_dimension(embedding: &[f32]) -> Result<()> {
    if embedding.len() != DIMENSION {
        bail!(
            "embedding dimension mismatch: got {}, expected {DIMENSION}",
            embedding.len()
        );
    }
    Ok(())
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn blob_to_embedding(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(std::mem::size_of::<f32>()) {
        bail!("stored embedding blob has invalid length")
    }
    Ok(bytes
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("chunk length checked")))
        .collect())
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (&left_value, &right_value) in left.iter().zip(right.iter()) {
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }
    dot / (left_norm.sqrt().max(1e-12) * right_norm.sqrt().max(1e-12))
}

fn model_dir(root: &std::path::Path, hash: &str) -> PathBuf {
    root.join(".brainmap/models/minishlab_potion-base-8M")
        .join(hash)
}

fn pack_hash() -> &'static str {
    PACK_SHA256
}

fn extract_pack(out: &Path) -> Result<()> {
    let decoder = zstd::Decoder::new(Cursor::new(PACK))?;
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let rel = entry.path()?.to_path_buf();
        util::safe_archive_path(&rel)?;
        entry.unpack(out.join(rel))?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ModelManifest {
    #[serde(rename = "modelId")]
    model_id: String,
    dimension: usize,
    files: Vec<ModelFile>,
}

#[derive(Debug, Deserialize)]
struct ModelFile {
    path: PathBuf,
    sha256: String,
    size: u64,
}

fn verify_materialized_dir(dir: &Path, pack_hash: &str) -> Result<()> {
    let manifest_path = dir.join("model-manifest.json");
    let manifest: ModelManifest = serde_json::from_slice(
        &fs::read(&manifest_path).with_context(|| format!("read {}", manifest_path.display()))?,
    )?;
    if manifest.model_id != MODEL_ID {
        bail!("model id mismatch: {}", manifest.model_id);
    }
    if manifest.dimension != DIMENSION {
        bail!("model dimension mismatch: {}", manifest.dimension);
    }
    for file in manifest.files {
        util::safe_archive_path(&file.path)?;
        let path = dir.join(&file.path);
        let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        if bytes.len() as u64 != file.size {
            bail!("model file size mismatch: {}", path.display());
        }
        let got = util::sha256_hex(&bytes);
        if got != file.sha256 {
            bail!("model file checksum mismatch: {}", path.display());
        }
    }
    util::write_atomic(&dir.join("pack.sha256"), pack_hash.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn embedded_model_pack_matches_build_hash() {
        let mut hasher = Sha256::new();
        hasher.update(PACK);
        assert_eq!(PACK.len().to_string(), env!("BRAINMAP_MODEL_PACK_LEN"));
        assert_eq!(hex::encode(hasher.finalize()), PACK_SHA256);
    }

    #[test]
    fn materializes_real_pack_and_verifies_checksums() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        models_materialize(Some(root.clone()), false).unwrap();
        models_verify(Some(root.clone())).unwrap();
        let hash = pack_hash();
        assert!(model_dir(&root, hash).join("model.safetensors").exists());
        assert!(model_dir(&root, hash).join("tokenizer.json").exists());
    }

    #[test]
    fn embed_rebuild_writes_vectors_and_searches() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BrainMap");
        vault::init_vault(Some(root.clone()), false, true).unwrap();
        models_materialize(Some(root.clone()), false).unwrap();
        index::rebuild(&root).unwrap();
        assert!(embed_notes(&root, false).unwrap() > 20);

        let conn = index::connection(&root).unwrap();
        let (count, dimension): (i64, i64) = conn
            .query_row(
                "select count(*),min(dimension) from vector_embeddings where model=?1",
                params![MODEL_ID],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(count > 20);
        assert_eq!(dimension, DIMENSION as i64);

        let results = search_vector(&root, "local first decisions", 5).unwrap();
        assert!(!results.is_empty());
    }
}
