# Harness Contract

Before asking the user a decision question, the harness must ask Brainmap:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

Harnesses must follow `outcome`, not prose.

- `proceed`: proceed with `selectedOption`; do not ask the user.
- `ask_user`: ask only `askUserQuestion`.
- `needs_more_context`: gather context or ask a narrower question.
- `block`: do not proceed.
- `defer`/`no_action`: delay or do nothing.

Before meaningful actions, call `gate`. After action, call `record-decision`. After user correction, call `learn-feedback`.

For context packs:

```bash
brainmap context --fast --json --vault ./tmp/BrainMap
```

This command reads the compiled SQLite index only. It does not scan the full vault, call network, load models, generate embeddings, call AgentMemory, or call an LLM.

Integration enforcement labels:

- enforced: host/runtime can block action.
- best-effort: hook/tool participates but host can bypass.
- instruction-only: model receives guidance only.

Generic stdio enforcement:

```bash
printf '%s\n' '{"situation":"Choose v1 storage","options":["Markdown+JSONL","External Vector DB"],"risk":"low","reversible":true}' \
  | brainmap harness stdio --vault ./tmp/BrainMap --fail-on-block
```
