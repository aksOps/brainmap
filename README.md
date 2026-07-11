# Brainmap Decision Engine

Brainmap is a local, deterministic-first personal decision engine. It helps agent harnesses decide in the user's style before asking the user.

It is not a knowledge base, transcript archive, vector-RAG chatbot, or remote memory SaaS.

## Quick Start

```bash
cargo run -- brainmap init --dry-run
cargo run -- brainmap init-vault --vault ./tmp/BrainMap --yes
cargo run -- brainmap index rebuild --vault ./tmp/BrainMap
cargo run -- brainmap onboard --vault ./tmp/BrainMap
cargo run -- brainmap gate --intent would-ask-user --situation "Choose v1 storage" --options "Markdown+JSONL|SQLite|External Vector DB" --risk low --reversible true --vault ./tmp/BrainMap --json
cargo run -- brainmap context --fast --json --vault ./tmp/BrainMap
```

Markdown is canonical. SQLite is a rebuildable compiled index. The hot path never calls an LLM, AgentMemory, network, or embedding generator.

Only validated deterministic rule markers are executable; ordinary policy prose remains context. See `docs/executable-policies.md` and `docs/onboarding.md`.

## Install

```bash
npm install -g @aksops/brainmap
cargo install brainmap-cli
```

Source install:

```bash
cargo install --path crates/brainmap-cli
```

Versioned Linux releases are created from `v*.*.*` tags and publish cargo crates, a GitHub tarball, and the npm binary package.

For the qualified M001 candidate, run the exact release `brainmap` beside its
matching `brainmapd`, preview the local installation, then install it into the
PATH-active destination (default `~/.local/bin`):

```bash
/absolute/release/brainmap install candidate \
  --qualification-bundle /absolute/path/qualification --dry-run
/absolute/release/brainmap install candidate \
  --qualification-bundle /absolute/path/qualification
```

The installer verifies the strict bundle and embedded provenance before any
mutation, backs up changed binaries, installs `brainmapd` before `brainmap`,
rolls both back on failed verification, and proves the installed pair is what
`PATH` resolves.

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

Codex integration installs instructions, safety-only hooks, the local MCP
registration, and the Brainmap skill. A global install is the simplest local
setup because it uses `$CODEX_HOME` (default `~/.codex`) and does not depend on
project trust. Preview every path first:

```bash
brainmap install harness --target codex --global --vault ~/BrainMap --dry-run
brainmap install harness --target codex --global --vault ~/BrainMap
brainmap integration doctor --target codex --global --vault ~/BrainMap
```

For a project-local install, replace `--global` with `--project .` and mark the
project trusted in Codex; Codex skips project configuration for untrusted
projects. Codex also asks the user to review new or changed command hooks. The
Brainmap doctor proves local files and the learning contract, and reports that
a real host probe is still required; start Codex normally and approve the exact
hook definition before relying on enforcement. The M001 qualification never
uses Codex's dangerous hook-trust bypass.

The installer pins the running Brainmap executable and an absolute vault path.
Routine MCP tools use Codex's automatic tool mode, while feedback and apply
remain explicit approval points. The MCP path supports gate, context, action
recording, structured feedback, pending preview, and approved activation.
Personal learning defaults to a stable `project:<name>-<hash>` scope derived
from the current directory; use `--scope global` only for an intentional global
rule. The qualified Codex adapter target for M001 is Linux x86_64.

## Dogfood Qualification

Start dogfood only from the strict, recursively checksummed M8 qualification
bundle produced by `scripts/m8-assemble-qualification.sh`. The bundle must bind
the exact full commit and the installed `brainmap` and companion `brainmapd`
binaries; obsolete flat `brainmap-m8-fia-v1` self-attestations are rejected.
Brainmap preserves and re-verifies the complete bundle for the lifetime of the
run and includes it as `qualification/` in final evidence.

During an intensive real-use shadow session, collect at least 30 complete
gate/action pairs spanning at least five distinct decision scenarios and either
at least three distinct scopes or at least three distinct decision types.
Diversity metrics retain aggregate counts only; they never include raw prompt,
situation, scope, or decision-type values.

After the final qualifying ledger event, persist a prompt-free review receipt.
Brainmap supplies the timestamp, exact ledger-prefix checksum, canonical
aggregate-metrics checksum, safety counters, and active run ID; the operator
supplies only the incident state:

```bash
brainmap dogfood review --incident-status clear --vault ~/BrainMap
brainmap dogfood status --vault ~/BrainMap
```

The other states are `investigating`, `resolved-no-violation`, and the terminal
`candidate-failed`. Final qualification requires at least one final covering
receipt in `clear` or `resolved-no-violation` state, plus zero false-proceed,
cross-domain, privacy, or hard-rule violations. Confirmed collisions remain a
required investigation and reported metric, but are not automatically terminal
unless they establish one of those failure classes.
It rejects tampered receipts, unresolved investigations, and a failed
candidate. Qualification is evidence-count based; longer monitoring is
optional, not a completion requirement. See `docs/production-readiness.md` for
the complete start, review, and finalize procedure.

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

## Offline Runtime Embeddings

The default model is downloaded at build time, checksum-verified, and embedded into the `brainmap` binary. `models materialize`, `models verify`, `embed rebuild`, and vector search run locally with no runtime downloads or external embedding providers; see `docs/model-packaging.md`.

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
