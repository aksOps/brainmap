# Generic Stdio Harness

Enforced harnesses can shell out to:

```bash
brainmap gate --intent would-ask-user --situation "$SITUATION" --options "$OPTIONS" --json
```

Use `outcome` as the control field.

Runtime stdio bridge:

```bash
printf '%s\n' '{"situation":"Choose v1 storage","options":["Markdown+JSONL","External Vector DB"],"risk":"low","reversible":true}' \
  | brainmap harness stdio --vault ./tmp/BrainMap --fail-on-block
```
