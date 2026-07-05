# Production Readiness

Target: single-user local Brainmap with embedded SQLite and bundled local model.

## Install

```bash
cargo install --path crates/brainmap-cli
brainmap init --dry-run
brainmap init-vault --vault ~/BrainMap --yes
brainmap index rebuild --vault ~/BrainMap
brainmap models materialize --vault ~/BrainMap
brainmap models verify --vault ~/BrainMap
brainmap embed rebuild --vault ~/BrainMap
```

## Autonomous Checks

CI runs on Linux, macOS, and Windows:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Linux security/SBOM gate:

```bash
cargo audit
cargo deny check
cargo cyclonedx --format json --override-filename brainmap
```

`cargo test` includes a CLI production smoke covering vault init, index verify, hot-path gate/context, model materialization, local embeddings, vector/hybrid search, export/import/restore, tamper rejection, snapshots, and eval.

## Backup Drill

```bash
brainmap export --mode portable --vault ~/BrainMap --out ./brainmap.brainmap.tar.zst
brainmap verify-export ./brainmap.brainmap.tar.zst
brainmap restore --file ./brainmap.brainmap.tar.zst --to ./BrainMap-restored
brainmap snapshot create --vault ~/BrainMap
brainmap snapshot list --vault ~/BrainMap
```

Use encrypted export when backups leave the machine:

```bash
brainmap export --mode portable --encrypt --recipient age1... --vault ~/BrainMap --out ./brainmap.brainmap.tar.zst.age
brainmap verify-export ./brainmap.brainmap.tar.zst.age --identity ./identity.txt
```

## Known Ceilings

- Vector search scans SQLite-stored vectors in-process. Good for personal vault scale; add sqlite-vec/ANN only after measured latency.
- MCP server is the narrow stdio tool surface Brainmap needs, not a broad general-purpose server.
- Canonical data is Markdown; SQLite and embeddings are rebuildable.
