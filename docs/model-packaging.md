# Model Packaging

Default target: `minishlab/potion-base-8M`.

Embedded pack:

- `assets/models/default.brainmap-model.tar.zst` contains real `potion-base-8M` Model2Vec assets plus `model-manifest.json`.
- Cargo registry packages split that same compressed pack into `crates/brainmap-model-potion-base-8m-part-*/data/part.bin`.
- Source model license: MIT.
- Embedded pack SHA-256: `ba53d6d59a6de57cb415f2229b7ce1e2ad26f5f9e19eca08bb1446f324d4a39e`.
- `brainmap models materialize` writes it under `.brainmap/models/minishlab_potion-base-8M/<hash>/`.
- `brainmap models verify` verifies extracted file checksums.
- `brainmap embed rebuild` uses the materialized pack through `model2vec-rs` local-only inference and stores 256-dimensional vectors in SQLite.
- No runtime download command exists.

Update steps:

1. Download new model assets outside Brainmap runtime.
2. Include model files, tokenizer/config, license/source metadata, model manifest, and SHA-256 checksums.
3. Compress as `assets/models/default.brainmap-model.tar.zst`.
4. Regenerate the cargo model chunk files with `scripts/prepare-model-crates.sh`.
5. Update `PACK_SHA256` and `PACK_LEN` in `crates/brainmap-cli/src/model.rs`.
6. Run `brainmap models verify`, `brainmap embed rebuild`, and `cargo test`.
