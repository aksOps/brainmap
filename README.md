# Brainmap Decision Engine

Brainmap is a local, deterministic-first personal decision engine. It helps agent harnesses decide in the user's style before asking the user.

It is not a knowledge base, transcript archive, vector-RAG chatbot, or remote memory SaaS.

## Quick Start

```bash
cargo run -- brainmap init --dry-run
cargo run -- brainmap init-vault --vault ./tmp/BrainMap --yes
cargo run -- brainmap index rebuild --vault ./tmp/BrainMap
cargo run -- brainmap gate --intent would-ask-user --situation "Choose v1 storage" --options "Markdown+JSONL|SQLite|External Vector DB" --risk low --reversible true --vault ./tmp/BrainMap --json
cargo run -- brainmap context --fast --json --vault ./tmp/BrainMap
```

Markdown is canonical. SQLite is a rebuildable compiled index. The hot path never calls an LLM, AgentMemory, network, or embedding generator.

## Install

```bash
cargo install --path crates/brainmap-cli
```

Versioned Linux releases are created from `v*.*.*` tags and include a tarball plus `SHA256SUMS`.

## Slow Path

Use `build-decision-engine --mode interview` from zero. AgentMemory is optional; failures fall back to interview mode.

```bash
cargo run -- brainmap build-decision-engine --mode interview --vault ./tmp/BrainMap --questions 7 --dry-run
```

## Harness Contract

Harnesses call `brainmap gate --json` before asking the user or doing meaningful work. They follow `outcome`, not prose, and call `record-decision` or `learn-feedback` afterward.

Harnesses can call `brainmap context --fast --json` for a compact SQLite-only context pack.

MCP:

```bash
cargo run -- brainmap mcp serve --vault ./tmp/BrainMap
```

The server speaks stdio JSON-RPC with MCP-shaped `initialize`, `tools/list`, and `tools/call`. Exposed tools are allowlisted; no shell tool exists.

## Web UI

```bash
cargo run -- brainmap web --vault ./tmp/BrainMap --host 127.0.0.1 --port 8777
cargo run -- brainmap web export-static --vault ./tmp/BrainMap --out ./tmp/brainmap-web
```

The UI is read-only, local, dark-mode first, and has no CDN, analytics, remote fonts, or write endpoints.

## Import/Export/Restore

```bash
cargo run -- brainmap export --mode portable --vault ./tmp/BrainMap --out ./tmp/brainmap.brainmap.tar.zst
cargo run -- brainmap verify-export ./tmp/brainmap.brainmap.tar.zst
cargo run -- brainmap import --file ./tmp/brainmap.brainmap.tar.zst --to ./tmp/Imported --dry-run
cargo run -- brainmap restore --file ./tmp/brainmap.brainmap.tar.zst --to ./tmp/Restored
cargo run -- brainmap export --mode portable --encrypt --recipient age1... --vault ./tmp/BrainMap --out ./tmp/brainmap.brainmap.tar.zst.age
cargo run -- brainmap verify-export ./tmp/brainmap.brainmap.tar.zst.age --identity ./identity.txt
```

## Offline Embeddings

The default model is embedded `minishlab/potion-base-8M`. `models materialize`, `models verify`, `embed rebuild`, and vector search run locally with no runtime downloads or external embedding providers; see `docs/model-packaging.md`.

## Verification

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo audit
cargo deny check
cargo cyclonedx
```

Scale check:

```bash
brainmap bench --vault /tmp/brainmap-scale-5000 --scale 5000 --embeddings
```

Production checklist: `docs/production-readiness.md`.
