# Hot Path vs Slow Path

Hot path:

- `brainmap context --fast`
- no LLM
- no AgentMemory
- no network
- no runtime downloads
- no embedding generation
- no model loading
- no full vault scan when valid index exists

Slow path:

- interview
- index rebuild
- import/export/restore
- model materialization
- local embedding rebuild/process
- vector/hybrid search
- review/dream/eval
