# Architecture

Brainmap has three durable surfaces:

- Markdown vault: canonical user-reviewable policies and packets.
- SQLite index: rebuildable compiled search/gate structure.
- JSONL ledgers: append-only decision/capture records.

Hot path commands (`gate`, `should-ask-user`, `capture`, `context --fast`) use only local files and the compiled index. They never call an LLM, AgentMemory, network, runtime downloads, or embedding generation.

Slow path commands perform vault builds, interview packets, index rebuilds, archive import/export, model materialization, local embedding generation, vector search, review, and eval.
