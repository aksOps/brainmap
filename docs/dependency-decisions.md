# Dependency Decisions

## Chosen

- `clap`: CLI parser, maintained, permissive.
- `serde`/`serde_json`: JSON schemas and machine-readable gate output.
- `rusqlite` with `bundled`: deterministic embedded SQLite and FTS5.
- `regex`: privacy redaction and wikilink parsing.
- `sha2`/`hex`: export/model checksums.
- `tar`/`zstd`: portable archive format.
- `age`: age/rage-compatible recipient encryption for `.brainmap.tar.zst.age`.
- `model2vec-rs`: local Model2Vec inference for embedded `potion-base-8M`; built with `local-only` and no Hugging Face runtime download feature.
- `chrono`: timestamps.
- `tempfile`: tests and restore verification.

## Deferred

- `sqlite-vec`: replaced in MVP by SQLite-stored vectors plus in-process cosine scanning. Revisit only if vault scale requires ANN/vector-index performance.
- `rmcp`: skipped for now. `brainmap mcp serve` implements a minimal stdio JSON-RPC tool server with MCP-shaped `initialize`, `tools/list`, and `tools/call`, while keeping dependency surface smaller.
- `axum`/frontend framework: skipped. Native local HTTP server is smaller and sufficient for read-only MVP.

## Rejected

- External vector DB, graph DB, Redis, Postgres, cloud embeddings, runtime Hugging Face download.
- `serde_yaml`: avoided to keep frontmatter parsing constrained to Brainmap schema fields.
