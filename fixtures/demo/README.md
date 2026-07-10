# Demo

Run:

```bash
cargo test
cargo run -- brainmap init-vault --vault ./tmp/DemoBrainMap --yes
cargo run -- brainmap index rebuild --vault ./tmp/DemoBrainMap
cargo run -- brainmap gate --intent would-ask-user --situation "Choose v1 storage" --options "Markdown+JSONL|SQLite|External Vector DB" --risk low --reversible true --decision-type architecture --vault ./tmp/DemoBrainMap --json
cargo run -- brainmap build-decision-engine --mode interview --vault ./tmp/DemoBrainMap --questions 7 --dry-run
cargo run -- brainmap search --text "local first" --vault ./tmp/DemoBrainMap
cargo run -- brainmap export --mode portable --vault ./tmp/DemoBrainMap --out ./tmp/demo.brainmap.tar.zst
cargo run -- brainmap verify-export ./tmp/demo.brainmap.tar.zst
cargo run -- brainmap web --vault ./tmp/DemoBrainMap --host 127.0.0.1 --port 8777
```
