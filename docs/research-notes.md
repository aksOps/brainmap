# Research Notes

Date: 2026-07-05.

Subagents:

- Broad research agents for dependencies, model/vector, and harnesses were attempted first. All three failed with context-window errors.
- Bounded retry probes completed for Rust/SQLite/vector/model and harness integrations.
- Fallback: main coordinator verified primary sources directly and kept dependency choices conservative.

## Rust

- Rust 2024 edition stabilized in Rust 1.85.0. Source: https://blog.rust-lang.org/2025/02/20/Rust-1.85.0/ and https://doc.rust-lang.org/edition-guide/rust-2024/
- Local environment: `rustc 1.95.0`, `cargo 1.95.0`, sufficient for Rust 2024.

## SQLite

- `rusqlite` is the default wrapper. Source: https://crates.io/crates/rusqlite
- Use `bundled` to avoid platform SQLite drift and enable deterministic FTS5 behavior through bundled SQLite.
- SQLite FTS5 is used for text search.

## Vector and Embeddings

- `sqlite-vec` is the preferred embedded SQLite vector path when full vector search is enabled. Source: https://crates.io/crates/sqlite-vec and https://alexgarcia.xyz/sqlite-vec/rust.html
- `model2vec-rs` exists for Rust local inference. Source: https://github.com/MinishLab/model2vec-rs and https://docs.rs/model2vec/
- `minishlab/potion-base-8M` is the default target model. Source: https://huggingface.co/minishlab/potion-base-8M
- The model card/config report a static Model2Vec model with hidden dimension 256. This repo embeds the real model pack and uses SQLite-stored vectors plus in-process cosine scanning for MVP vector search.

## Archive, Encryption, Security

- Use `tar` + `zstd` for `.brainmap.tar.zst` portable archives. Sources: https://crates.io/crates/tar and https://crates.io/crates/zstd
- `age`/`rage` are suitable for encrypted export. Source: https://crates.io/crates/age and https://crates.io/crates/rage
- `age` 0.11.3 is integrated for recipient-based `.age` archive encryption/decryption. Context7 was not exposed in this session; crates.io/docs.rs/source docs were used as fallback.
- Security tooling: `cargo-audit`, `cargo-deny`, `cargo-cyclonedx`. Sources: https://crates.io/crates/cargo-audit, https://crates.io/crates/cargo-deny, https://crates.io/crates/cargo-cyclonedx

## MCP and Harnesses

- Official Rust MCP SDK: https://github.com/modelcontextprotocol/rust-sdk and https://docs.rs/rmcp
- Claude Code official docs cover skills, hooks, MCP, subagents. Sources: https://docs.anthropic.com/en/docs/claude-code/skills, https://docs.anthropic.com/en/docs/claude-code/hooks, https://docs.anthropic.com/en/docs/claude-code/mcp
- Codex official docs cover AGENTS.md, hooks, subagents, MCP. Sources: https://developers.openai.com/codex/guides/agents-md, https://developers.openai.com/codex/hooks, https://developers.openai.com/codex/subagents, https://developers.openai.com/codex/mcp
- OpenCode official docs cover rules/plugins/MCP. Sources: https://opencode.ai/docs/rules/, https://opencode.ai/docs/plugins/, https://opencode.ai/docs/config/
- GitHub Copilot official docs cover custom instructions, MCP, custom agents. Sources: https://docs.github.com/copilot/customizing-copilot/adding-custom-instructions-for-github-copilot, https://docs.github.com/en/copilot/concepts/context/mcp, https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/create-custom-agents

## Web UI

MVP uses Rust-served static HTML/CSS/JS with no external assets. This avoids build-chain and CDN risk while satisfying read-only local UI requirements.
