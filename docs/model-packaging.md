# Model Packaging

Default target: `minishlab/potion-base-8M`.

Embedded pack:

- No model bytes are checked into git.
- `crates/brainmap-cli/build.rs` downloads real `potion-base-8M` Model2Vec assets from Hugging Face at build time.
- The build verifies every downloaded file by size and SHA-256, writes `model-manifest.json`, builds `default.brainmap-model.tar.zst` under Cargo `OUT_DIR`, and embeds that generated pack into the binary.
- Build prerequisite: `curl`.
- Source model license: MIT.
- Embedded pack SHA-256 is computed during build and exposed by `models status`.
- `brainmap models materialize` writes it under `.brainmap/models/minishlab_potion-base-8M/<hash>/`.
- `brainmap models verify` verifies extracted file checksums.
- `brainmap embed rebuild` uses the materialized pack through `model2vec-rs` local-only inference and stores 256-dimensional vectors in SQLite.
- No runtime download command exists.

Update steps:

1. Update `MODEL_FILES` in `crates/brainmap-cli/build.rs`.
2. Run `cargo clean -p brainmap-cli && cargo test`.
3. Run `brainmap models verify` and `brainmap embed rebuild` from the rebuilt binary.
