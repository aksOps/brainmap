# Brainmap MVP Implementation Plan

Goal: build a working local decision engine with Markdown vault, SQLite index, deterministic gate, learning packets, import/export, read-only web UI, harness install dry-runs, security docs, tests, and benchmarks.

1. Create Rust 2024 workspace and docs.
2. Build canonical vault generator with safe starter notes.
3. Parse Brainmap frontmatter and wikilinks.
4. Compile Markdown/JSONL into SQLite + FTS + graph tables.
5. Implement deterministic gate and wrappers.
6. Add capture, update packets, interview, apply, calibration.
7. Add text search, graph, embedded model pack, local embeddings, and vector search.
8. Add portable export/import/restore with checksums and path traversal rejection.
9. Add read-only local web UI and static export.
10. Add harness installers, MCP allowlist surface, bench, eval, snapshots.
11. Run fmt, clippy, tests, acceptance commands.
