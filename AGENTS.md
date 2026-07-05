# Agent Instructions

This repo builds Brainmap Decision Engine: a local decision engine, not an information engine.

- Markdown is canonical.
- SQLite is a rebuildable compiled index.
- Hot path must stay deterministic and fast.
- No LLM in `gate`.
- No AgentMemory in `gate`.
- No embedding generation in `gate`.
- No external embedding providers.
- No runtime model downloads.
- Default embedding target is embedded `minishlab/potion-base-8M`.
- Use update packets for learning.
- Preserve wikilinks.
- Never store secrets.
- Every schema change needs tests.
- Every installer needs dry-run and backup.
- Every import/export path needs checksum validation.
- Every gate change needs eval tests.
- Run tests before final answer.

Use `rtk` before shell commands in this workspace.

