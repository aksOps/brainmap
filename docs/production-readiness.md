# Production Readiness

Target: single-user local Brainmap on Linux-family systems with embedded SQLite and bundled local model.

Supported release shape:

- Local-only runtime; no network service required.
- Cargo registry source install downloads the model at build time and embeds it into the binary.
- Linux x86_64 npm binary package from `v*.*.*` Git tags.
- Linux x86_64 GitHub release tarball plus `SHA256SUMS`.
- Ubuntu, Fedora, RHEL, and UBI-style hosts install by cargo, npm package, release tarball, or source `cargo install`.
- Markdown is canonical. SQLite, embeddings, snapshots, and exports are rebuildable.

## Install

```bash
npm install -g @aksops/brainmap
cargo install brainmap-cli
brainmap init --dry-run
brainmap init-vault --vault ~/BrainMap --yes
brainmap index rebuild --vault ~/BrainMap
brainmap models materialize --vault ~/BrainMap
brainmap models verify --vault ~/BrainMap
brainmap embed rebuild --vault ~/BrainMap
```

Source install:

```bash
cargo install --path crates/brainmap-cli
brainmap init --dry-run
brainmap init-vault --vault ~/BrainMap --yes
brainmap index rebuild --vault ~/BrainMap
brainmap models materialize --vault ~/BrainMap
brainmap models verify --vault ~/BrainMap
brainmap embed rebuild --vault ~/BrainMap
```

Versioned release:

```bash
git tag -a v0.1.0 -m "v0.1.0"
git push origin v0.1.0
sha256sum -c SHA256SUMS
```

The release workflow publishes `brainmap-vX.Y.Z-linux-x86_64.tar.gz`, `SHA256SUMS`, cargo crates when `CARGO_REGISTRY_TOKEN` is configured, and `@aksops/brainmap` when `NPM_TOKEN` is configured.

Cargo registry shape:

- `brainmap-cli` publishes without model bytes in the crate package.
- `build.rs` downloads and checksum-verifies `minishlab/potion-base-8M`, then embeds the generated pack.
- `cargo install brainmap-cli` needs network and `curl` at build/install time. No runtime model download is introduced.

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

## Scale Envelope

Primary supported target:

- 1k-5k curated Markdown files.
- One embedded SQLite index.
- One `minishlab/potion-base-8M` 256-dimensional `f32` embedding per note.
- Current vector search is SQLite read plus in-process cosine scan.

Stretch target:

- 10k-25k files only after benchmark proof on the release target.

Not a target:

- 100k+ files.
- Millions of chunks.
- General document archive or RAG behavior.

Run scale checks on the Linux release binary:

```bash
brainmap bench --vault /tmp/brainmap-scale-1000 --scale 1000
brainmap bench --vault /tmp/brainmap-scale-5000 --scale 5000 --embeddings
brainmap bench --vault /tmp/brainmap-scale-10000 --scale 10000
```

`--scale` writes deterministic benchmark notes under `90-calibration/scale-bench` and replaces only that directory. `--embeddings` includes model materialization, embedding rebuild, and vector search timing.

Observed on this container with the optimized Linux binary:

| Files | Index rebuild | Gate | Embed rebuild | Vector search | Raw vectors |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1k | 76 ms | 2 ms | 166 ms | 79 ms | 1.0 MB |
| 5k | 231 ms | 9 ms | 335 ms | 100 ms | 5.1 MB |
| 10k | 418 ms | 18 ms | 577 ms | 137 ms | 10.2 MB |
| 25k | 1046 ms | 44 ms | 1179 ms | 141 ms | 25.6 MB |

Treat these as a sanity baseline, not a hardware guarantee.

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

- Vector search scans SQLite-stored vectors in-process. Good for the 1k-5k personal decision scale; add sqlite-vec/ANN only after measured latency.
- MCP server is the narrow stdio tool surface Brainmap needs, not a broad general-purpose server.
- Canonical data is Markdown; SQLite and embeddings are rebuildable.
