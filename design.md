
Build a production-quality MVP called:

Brainmap Decision Engine

This is not a knowledge base, not a second-brain archive, and not an information retrieval app.

It is a fast, local, deterministic-first personal decision engine that helps agent harnesses decide like the user, ask the user fewer and better questions, and learn from corrections over time.

The system must be fully testable, local-first, file-based, secure, open-source, importable/exportable/restorable, and designed for low CPU, low RAM, and snappy harness use.

Use subagent-driven development. Use the most efficient model for each subtask:
- Main coordinator / final architect: GPT-5.5 xhigh fast.
- Research subagents: GPT-5.3 Spark or the fastest capable available model.
- Implementation subagents: GPT-5.4 or GPT-5.5 depending complexity.
- Test/security/audit subagents: GPT-5.5 for critical paths, GPT-5.4 for routine coverage.
- UI/design subagent: GPT-5.4 or GPT-5.5 depending availability.
- If exact model aliases are unavailable in the environment, use the closest available model and document the fallback in `docs/implementation-notes.md`.

Do not ask me clarifying questions. Make strong defaults, implement the system, and document assumptions.

The final answer must include:
1. What was built.
2. Files and modules created.
3. Commands run.
4. Test results.
5. Bench/security results.
6. Remaining limitations.
7. Exact next commands for me to run.

Do not stop after scaffolding. Build working vertical slices and keep iterating until the acceptance criteria pass or until you have made the strongest possible implementation with honest caveats.

---

# 0. Mandatory research phase

Before coding, run a research phase using subagents.

Research current official docs and latest secure open-source libraries for:

- Rust stable version and Rust 2024 edition support.
- SQLite embedded use in Rust.
- SQLite FTS5.
- sqlite-vec or latest secure embedded vector alternative.
- Model2Vec Rust support.
- `minishlab/potion-base-8M` model packaging, license, files, dimensions, and local inference requirements.
- Embedded model packaging in Rust.
- Rust MCP server libraries.
- Claude Code skills/hooks/MCP/instructions.
- Codex skills/hooks/AGENTS.md/subagents/MCP.
- OpenCode plugins/hooks/MCP/instructions.
- GitHub Copilot instructions/skills/custom agents/MCP support.
- Secure archive export/import libraries.
- Zstandard archive handling.
- age/rage encryption libraries.
- Rust security tooling: cargo-audit, cargo-deny, cargo-cyclonedx, cargo-vet if practical.
- Frontend stack for a read-only modern local web UI.

Use latest stable, secure, actively maintained, permissively licensed, open-source libraries.

Do not use abandoned or archived projects as default dependencies.

If a library appears risky, unmaintained, GPL/AGPL-only, or insecure, choose a safer alternative and document the decision.

Create:

docs/research-notes.md
docs/dependency-decisions.md
docs/implementation-plan.md

Research must happen before final dependency choices are locked.

---

# 1. Product definition

Build:

Brainmap Decision Engine

Purpose:

Help agent harnesses decide in the user's style.

The system learns:

- choices
- rejected options
- tradeoffs
- hard constraints
- soft preferences
- default priorities
- ask triggers
- approval boundaries
- examples
- counterexamples
- uncertainty behavior
- correction patterns
- risk posture
- local/system/tool/model preferences

The system must not become:

- project archive
- code archive
- raw transcript storage
- general knowledge base
- fact database
- vector-RAG chatbot
- external memory SaaS
- remote embedding client
- always-on LLM service

Core sentence:

Brainmap is a portable local decision engine whose canonical brain is Markdown, whose working index is SQLite, whose semantic retrieval is embedded local vectors, whose topology is a graph, whose hot path is deterministic, and whose harness gate asks Brainmap before asking the user.

---

# 2. Architectural doctrine

Use this separation:

## Hot path

Used constantly by harnesses.

Commands:

- `brainmap gate`
- `brainmap should-ask-user`
- `brainmap record-decision`
- `brainmap capture`
- `brainmap context --fast`

Hot path rules:

- no LLM calls
- no AgentMemory calls
- no network
- no external embeddings
- no embedding generation
- no model loading
- no full Markdown vault scan
- no index rebuild
- no long graph analytics
- no slow historical enrichment
- no remote API

Hot path may use:

- compiled SQLite index
- policy tables
- restrictions
- approval rules
- FTS5 if already indexed
- sqlite-vec if vectors already exist
- graph edge tables
- compact in-memory cache
- deterministic scoring
- JSONL append

## Slow path

Used for learning, import, baseline, review, dreaming, embeddings, and enrichment.

Commands:

- `brainmap build-decision-engine`
- `brainmap build-brain`
- `brainmap calibrate`
- `brainmap extract`
- `brainmap review-decisions`
- `brainmap dream`
- `brainmap embed rebuild`
- `brainmap baseline`
- `brainmap import`
- `brainmap export`
- `brainmap restore`

Slow path may use:

- local embedded embeddings
- optional AgentMemory adapter
- optional historical import
- optional Fable/heavy model through harness approval
- broader scans
- review/refactor
- batch processing

---

# 3. Primary technology stack

Use Rust as the core implementation language.

Use current stable Rust and Rust 2024 edition if supported by the environment.

Create a workspace or well-structured single Rust project. Prefer workspace crates if it improves maintainability without slowing MVP delivery.

Suggested crates after research, subject to replacement by newer/safer choices:

- CLI: `clap`
- Serialization: `serde`, `serde_json`, `serde_yaml`, `toml`
- Errors: `anyhow`, `thiserror`
- Async/daemon: `tokio`
- Local HTTP API if needed: `axum`
- SQLite: `rusqlite` with bundled SQLite if appropriate
- Full-text search: SQLite FTS5
- Vector search: `sqlite-vec` or latest secure embedded SQLite vector extension
- Embeddings: `model2vec` / Model2Vec Rust support
- Graph: SQLite edge tables + `petgraph` for in-memory traversal if useful
- Markdown/frontmatter: choose secure maintained parser or implement minimal safe parser
- Glob/ignore: `ignore`, `globset`
- Regex/redaction: `regex`
- Archive: `tar`, `zstd`
- Encryption: `age` or rage-compatible crate
- Hashing: SHA-256 crate with maintained ecosystem
- MCP: latest secure maintained Rust MCP SDK
- Web UI: Rust-served static UI or small TypeScript frontend built with Vite/Svelte/React, but keep read-only and lightweight
- Tests: `cargo test`, `tempfile`, `insta` if useful
- Bench: lightweight timing command plus optional criterion if practical
- Security: `cargo-audit`, `cargo-deny`, `cargo-cyclonedx`

Do not use:

- external vector DB
- external graph DB
- Redis
- Postgres
- cloud DB
- cloud embeddings
- Hugging Face runtime download
- remote embedding APIs
- GPL/AGPL-only dependencies
- archived libraries as defaults
- arbitrary command execution in MCP tools

---

# 4. Repository structure

Create approximately:

brainmap/
  Cargo.toml
  Cargo.lock
  rust-toolchain.toml
  deny.toml
  README.md
  AGENTS.md
  SECURITY.md
  LICENSE-MIT
  LICENSE-APACHE
  .gitignore
  .brainmapignore.example

  crates/
    brainmap-cli/
    brainmapd/
    brainmap-core/
    brainmap-vault/
    brainmap-index/
    brainmap-gate/
    brainmap-search/
    brainmap-vector/
    brainmap-graph/
    brainmap-model/
    brainmap-import-export/
    brainmap-harness/
    brainmap-mcp/
    brainmap-sources/
    brainmap-privacy/
    brainmap-web/

  assets/
    models/
      default/
        README.md
        model-manifest.template.json

  integrations/
    claude-code/
      skills/
      hooks/
      README.md
    codex/
      skills/
      hooks/
      AGENTS.md.fragment
      README.md
    opencode/
      plugin/
      README.md
    copilot/
      github/
      README.md
    generic-harness/
      README.md

  docs/
    architecture.md
    implementation-plan.md
    implementation-notes.md
    research-notes.md
    dependency-decisions.md
    decision-ontology.md
    harness-contract.md
    hot-path-vs-slow-path.md
    privacy-threat-model.md
    import-export.md
    model-packaging.md
    web-ui.md
    acceptance-report.md

  fixtures/
    interview/
    decisions/
    agentmemory/
    exports/
    vaults/
    web/

  tests/
    integration/

If workspace complexity slows the MVP, use a smaller crate layout but preserve module boundaries.

---

# 5. Canonical vault structure

Markdown is canonical.

SQLite is rebuildable.

Create vault at `~/BrainMap` by default or provided `--vault`.

Vault:

BrainMap/
  README.md
  INDEX.md
  DECISIONMAP.md

  00-control/
    _index.md
    engine-contract.md
    privacy-rules.md
    no-knowledge-archive-rule.md
    no-project-archive-rule.md
    decision-boundaries.md
    approval-policy.md
    update-protocol.md
    prompt-injection-policy.md
    policy-precedence.md

  10-decision-identity/
    _index.md
    default-operating-principles.md
    priority-stack.md
    taste-profile.md
    risk-posture.md
    uncertainty-style.md
    context-scopes.md

  20-decision-frames/
    _index.md
    architecture-decisions.md
    tooling-decisions.md
    model-selection-decisions.md
    workflow-decisions.md
    communication-decisions.md
    privacy-decisions.md
    time-decisions.md
    learning-decisions.md

  30-tradeoff-models/
    _index.md
    speed-vs-quality.md
    simplicity-vs-power.md
    local-first-vs-cloud.md
    automation-vs-control.md
    flexibility-vs-maintenance.md
    cost-vs-capability.md
    novelty-vs-reliability.md
    reversible-vs-irreversible.md

  40-restrictions/
    _index.md
    hard-no-rules.md
    approval-required.md
    privacy-boundaries.md
    non-goals.md
    anti-patterns.md
    never-auto.md

  50-choice-patterns/
    _index.md
    preferred-defaults.md
    recurring-choices.md
    rejection-patterns.md
    escalation-patterns.md
    reversible-decisions.md
    irreversible-decisions.md

  60-decision-examples/
    _index.md
    examples.md
    counterexamples.md
    pairwise-comparisons.md
    explained-decisions.md
    wrong-decisions.md
    corrected-decisions.md

  70-question-triggers/
    _index.md
    ask-before-deciding.md
    ask-when-uncertain.md
    batch-questions.md
    clarification-patterns.md
    suppress-redundant-questions.md

  80-agent-interface/
    _index.md
    decide-command.md
    gate-contract.md
    context-pack-template.md
    lightweight-decision-mode.md
    decision-output-format.md
    feedback-protocol.md

  90-calibration/
    _index.md
    decision-ledger.jsonl
    pending-feedback.md
    calibration-questions.md
    interview-state.md
    shadow-mode-report.md
    evaluation-report.md

  95-reviews/
    _index.md
    daily-micro-review.md
    weekly-decision-review.md
    monthly-map-refactor.md
    dream-lite-report.md

  99-meta/
    _index.md
    schema.md
    tags.md
    changelog.md
    source-policy.md
    audit-checklist.md
    pending-update-packets/
    backups/

  .brainmap/
    brainmap.sqlite
    capture-queue.jsonl
    embed-queue.jsonl
    index-manifest.json
    models/
    exports/
    web-cache/
    locks/

Every generated Markdown note must have YAML frontmatter.

Do not include fake personal facts in starter notes.

Starter notes should contain schema, instructions, and empty/default policy placeholders only.

---

# 6. Decision note schema

Implement schemas in Rust and validate them.

Decision policy note:

---
id: local-first-before-infrastructure
type: decision-policy
status: seed
confidence: medium
risk_tier: reversible-auto
sensitivity: personal
created: 2026-07-05
updated: 2026-07-05
last_confirmed:
decay: normal
scope:
  domains:
    - personal-tools
    - agent-systems
  applies_to:
    - v1 systems
    - local automation
  does_not_apply_to:
    - irreversible production migrations without review
tags:
  - brainmap
  - decision-policy
  - architecture
links:
  parent: "[[20-decision-frames/architecture-decisions]]"
  tradeoffs:
    - "[[30-tradeoff-models/simplicity-vs-power]]"
    - "[[30-tradeoff-models/local-first-vs-cloud]]"
  restrictions:
    - "[[40-restrictions/approval-required]]"
sources: []
---

# Local-first before infrastructure

## Policy

Prefer local-first, inspectable, low-dependency systems before adding heavier infrastructure.

## Applies when

- Building v1 personal systems
- Designing local agent tooling
- Choosing storage or integration defaults

## Default decision

Start with local files, Markdown, JSONL, and embedded SQLite unless scale pressure or reliability requirements justify more.

## Tradeoff rule

Prefer simplicity and portability over power unless the heavier option clearly removes repeated friction.

## Ask before deciding if

- The decision affects privacy
- The change is hard to reverse
- The lightweight option has already failed
- The system becomes multi-user or production-critical

## Examples

- Situation: Choosing Brainmap v1 storage.
- Chosen: Markdown canonical vault + SQLite compiled index.
- Rejected: External vector database.
- Rationale: Portable, inspectable, low-dependency.

## Counterexamples

- Use SQLite FTS/indexes for compiled search.
- Consider heavier backend only if measurable scale pressure appears.

## Links

- Parent: [[20-decision-frames/architecture-decisions]]
- Tradeoff: [[30-tradeoff-models/simplicity-vs-power]]
- Restriction: [[40-restrictions/approval-required]]

## Update log

- Created as seed.

Supported object types:

- decision-policy
- decision-frame
- tradeoff-rule
- hard-constraint
- soft-preference
- default-priority
- ask-trigger
- approval-rule
- decision-example
- counterexample
- rejection-pattern
- escalation-rule
- uncertainty-rule
- calibration-question
- wrong-decision
- corrected-decision
- context-scope
- meta-rule

Statuses:

- seed
- tested
- reliable
- stale
- retired
- contradicted

Confidence:

- low
- medium
- high

Risk tier:

- suggest-only
- ask-before-action
- reversible-auto
- approval-required
- never-auto

Sensitivity:

- public
- personal
- private
- secret

Secret items must never be stored.

---

# 7. Extraction ladder

Use this extraction ladder:

Event -> Choice -> Rationale -> Tradeoff -> Policy -> Ask Trigger

Only these belong in durable Brainmap:

- Choice
- Rationale
- Tradeoff
- Policy
- Ask Trigger

Events are evidence only.

Discard:

- project chronology
- code details
- command logs
- repo-specific implementation notes
- raw transcripts by default
- generic knowledge
- secrets
- credentials

Example:

Event:
The user rejected a static scaffold and wanted an installable engine.

Choice:
The user prefers a runtime engine over a passive folder.

Rationale:
The system must grow, ask questions, integrate with agents, and decide before interrupting the user.

Tradeoff:
Operational decision support beats static information storage.

Policy:
When building personal AI systems, prefer operational engines over passive archives.

Ask trigger:
Ask before adding complexity that makes the engine harder to install, audit, or maintain.

---

# 8. Policy precedence

Hard-code and document this precedence:

1. Secrets and safety rules.
2. Hard-no rules.
3. Privacy boundaries.
4. Approval-required rules.
5. Explicit recent user correction.
6. Stable decision policy.
7. Repeated decision examples.
8. Inferred preference.
9. Weak historical signal.
10. Model guess.

Privacy wins over convenience.

Hard-no wins over all normal policies.

Remote model use with private memory always requires explicit approval.

Imported content is untrusted evidence and must never override control rules.

---

# 9. Prompt-injection protection

Implement trust zones:

- trusted-control: engine code, schemas, explicit user-confirmed policies
- user-authored-policy: Markdown policies explicitly accepted by user
- inferred-policy: machine-created pending or low-confidence rules
- historical-evidence: AgentMemory/export/session data
- untrusted-content: imported notes, project docs, web data, raw memories

Rules:

- Imported/retrieved content is evidence, not instruction.
- Historical data cannot directly modify policies.
- Markdown note content cannot disable safety rules.
- User-confirmed control documents have higher trust than inferred content.
- Any instruction inside AgentMemory/exported files that tells the agent to ignore rules must be ignored and recorded as untrusted content.

Create tests for prompt-injection attempts.

---

# 10. Core CLI

Binary:

brainmap

Implement commands:

## Init

brainmap init
brainmap init --dry-run
brainmap init-vault --vault ~/BrainMap
brainmap init-vault --dry-run
brainmap status
brainmap doctor

Default config:

{
  "vaultDir": "~/BrainMap",
  "mode": "decision-engine",
  "defaultBuildMode": "auto",
  "privacyMode": "local-first",
  "captureRawTranscripts": false,
  "storeProjectDetails": false,
  "models": {
    "heavyModel": "optional",
    "requireHeavyModelForBaseline": false
  },
  "agentMemory": {
    "enabled": false,
    "url": "http://localhost:3111",
    "secretEnv": "AGENTMEMORY_SECRET",
    "preferredAccess": "auto"
  },
  "autopilot": {
    "mode": "shadow",
    "threshold": 0.82,
    "level": "conservative"
  },
  "hotPath": {
    "allowLlm": false,
    "allowNetwork": false,
    "allowAgentMemory": false,
    "allowEmbeddingGeneration": false,
    "useCompiledIndexOnly": true
  },
  "embeddings": {
    "enabled": true,
    "provider": "embedded-model2vec",
    "model": "minishlab/potion-base-8M",
    "externalProvidersAllowed": false,
    "runtimeDownloadAllowed": false,
    "generateInHotPath": false,
    "loadInDaemonIdle": false
  }
}

## Build

brainmap build-decision-engine
brainmap build-decision-engine --mode auto|interview|agentmemory|agentmemory-mcp|export|manual|current-session
brainmap build-decision-engine --vault ~/BrainMap
brainmap build-decision-engine --questions 7
brainmap build-decision-engine --dry-run
brainmap build-brain

`build-brain` is an alias for `build-decision-engine`.

AgentMemory is optional.

If no AgentMemory exists, interview mode must work from zero.

Interview mode first questions:

1. What should future agents understand about how you decide that they usually miss?
2. What should this decision engine help with: coding choices, design choices, model/tool choices, workflow choices, privacy boundaries, time decisions, or something else?
3. What should never be stored or inferred?
4. When should an agent ask immediately, batch questions, or make a reversible guess?
5. What kinds of details should be treated only as evidence and discarded after extracting the decision pattern?
6. What makes a system feel useful instead of heavy or noisy?
7. Which decisions can lightweight models/harnesses make automatically, and which require your approval?

Answers create update packets first.

Do not mutate stable notes without approval.

## Gate

brainmap gate --json
brainmap gate --intent would-ask-user|act|plan|tool-use|write-file|delete-file|external-call|model-call|privacy|unknown
brainmap gate --situation "..."
brainmap gate --options "A|B|C"
brainmap gate --proposed-action "..."
brainmap gate --risk low|medium|high|critical
brainmap gate --reversible true|false
brainmap gate --decision-type architecture|tooling|workflow|model|communication|privacy|file-change|external-action|time|general
brainmap gate --agent-confidence 0.75
brainmap gate --vault ~/BrainMap
brainmap gate --dry-run

Gate output JSON:

{
  "decisionId": "dec_...",
  "outcome": "proceed|ask_user|block|needs_more_context|defer|no_action",
  "recommendation": "string",
  "selectedOption": "string|null",
  "rejectedOptions": ["string"],
  "confidence": 0.0,
  "riskTier": "suggest_only|ask_before_action|reversible_auto|approval_required|never_auto",
  "reasoningSummary": ["short user-readable summary"],
  "matchedPolicies": ["[[path/to/policy]]"],
  "restrictionsApplied": ["[[path/to/restriction]]"],
  "askUserQuestion": "string|null",
  "defaultIfNoAnswer": "string|null",
  "learningEvent": {
    "shouldRecord": true,
    "kind": "decision-gate",
    "situation": "string",
    "chosen": "string|null",
    "confidence": 0.0
  }
}

Do not expose hidden chain-of-thought.

Harnesses must use `outcome`, not prose.

## Should ask user

brainmap should-ask-user --question "..." --situation "..." --json

This is a convenience wrapper around gate.

It determines whether the harness should ask the user or whether Brainmap can decide.

## Decide

brainmap decide "<situation>"
brainmap decide "<situation>" --options "A|B|C"
brainmap decide "<situation>" --risk low --reversible true
brainmap decide "<situation>" --json

Human-readable decision explanation.

## Record and learn

brainmap record-decision
brainmap record-decision --decision-id dec_123 --chosen "..." --was-asked false --vault ~/BrainMap
brainmap learn-feedback --decision-id dec_123 --correction "..." --vault ~/BrainMap
brainmap learn-decision --situation "..." --options "A|B" --chosen "A" --rejected "B" --rationale "..."

Rules:

- Silence after automatic decision is weak evidence.
- Explicit approval is medium-high evidence.
- Explicit correction is strong evidence.
- Repeated choices become stable policy only after review/promotion.

## Calibration

brainmap calibrate --vault ~/BrainMap --n 7
brainmap calibrate --topic privacy|architecture|tooling|workflow|model|time|all

Ask pairwise decision questions.

Example:

When building a v1 local personal tool, choose one:
A. Simple files plus SQLite index first.
B. Full database/vector system first.

Which feels more like your decision style and why?

## Autopilot

brainmap autopilot status
brainmap autopilot enable --level conservative|balanced|aggressive
brainmap autopilot disable
brainmap autopilot promote --to shadow|conservative|balanced|aggressive
brainmap autopilot demote --to shadow
brainmap autopilot set-threshold --confidence 0.82
brainmap gate-mode ask-always|suggest-only|shadow|active

Default: shadow.

Promotion rules:

Shadow -> Conservative:
- at least 30 shadow decisions
- fewer than 2 serious mismatches
- zero privacy/hard-rule violations
- explicit user command

Conservative -> Balanced:
- at least 100 decisions
- false-proceed rate below configured threshold
- explicit user approval

Balanced -> Aggressive:
- never automatic
- explicit user command only

Global kill switches:

- `BRAINMAP_DISABLE_AUTOPILOT=1`
- `BRAINMAP_GATE_MODE=ask-always`

## Capture

brainmap capture --stdin
brainmap capture --text "..."
brainmap capture --source claude-code|codex|opencode|copilot|manual

Capture must append compact events to JSONL and return quickly.

Hooks capture only. They do not analyze.

## Extract and apply

brainmap extract
brainmap extract --from-queue
brainmap apply
brainmap apply --pending
brainmap apply --yes
brainmap apply --dry-run

Update packet schema:

{
  "id": "upd_...",
  "createdAt": "ISO",
  "source": {
    "kind": "interactive|agentmemory-rest|agentmemory-mcp|agentmemory-export|manual|current-session|hook|harness",
    "ref": "...",
    "sessionId": "...",
    "confidence": 0.0
  },
  "classification": "decision-policy|tradeoff-rule|hard-constraint|soft-preference|approval-rule|ask-trigger|decision-example|counterexample|wrong-decision|corrected-decision|calibration-question",
  "claim": "short plain statement",
  "evidence": [
    {
      "quoteOrSummary": "...",
      "sourceRef": "...",
      "strength": "weak|medium|strong|very-strong"
    }
  ],
  "targetNotes": ["[[20-decision-frames/architecture-decisions]]"],
  "suggestedLinks": ["[[30-tradeoff-models/simplicity-vs-power]]"],
  "confidence": 0.0,
  "sensitivity": "public|personal|private|secret",
  "action": "create|append|merge|revise|retire|ask",
  "humanQuestion": "optional",
  "status": "pending|applied|rejected|needs-user-answer"
}

Secret packets must be rejected/redacted.

## Review and dreaming

brainmap review-decisions daily|weekly|monthly
brainmap dream --mode lite
brainmap dream --mode embed
brainmap dream --mode deep

Dream-lite is deterministic and default.

Dream-lite:
- detect repeated decisions
- find contradictions
- find stale policies
- detect missing approval rules
- detect too many ask_user outcomes
- merge duplicates
- find orphan graph nodes
- suggest calibration questions
- create update packets

Dream-embed:
- process missing embeddings
- find similar policies/examples
- cluster near-duplicates
- no external network

Dream-deep:
- optional LLM reflection
- requires explicit approval
- creates pending packets only
- never rewrites stable policy directly

## Index

brainmap index rebuild --vault ~/BrainMap
brainmap index status
brainmap link-check
brainmap graph neighbors <id>
brainmap search --text "..."
brainmap search --vector "..."
brainmap embed rebuild
brainmap embed process --missing-only
brainmap embed status

Index must compile Markdown + JSONL ledgers into SQLite.

## Models

brainmap models status
brainmap models materialize
brainmap models verify
brainmap models info

No `models download` command in MVP.

## Import/export/restore

brainmap export --mode portable|full|share-safe|encrypted --vault ~/BrainMap --out file.brainmap.tar.zst
brainmap export --mode portable --encrypt --recipient age1...
brainmap import --file file.brainmap.tar.zst --to ~/BrainMap-Restored --dry-run
brainmap restore --file file.brainmap.tar.zst --to ~/BrainMap
brainmap verify-export file.brainmap.tar.zst

## Web UI

brainmap web
brainmap web --vault ~/BrainMap --open
brainmap web --host 127.0.0.1 --port 8777
brainmap web export-static --out ./brainmap-web

Read-only only.

No write APIs exposed by web UI.

## MCP

brainmap mcp serve

Expose allowlisted tools only:
- brainmap_decision_gate
- brainmap_should_ask_user
- brainmap_record_decision
- brainmap_learn_feedback
- brainmap_context
- brainmap_import
- brainmap_export
- brainmap_restore
- brainmap_autopilot_status

No arbitrary shell command tool.

---

# 11. Decision gate algorithm

Implement deterministic gate algorithm.

1. Normalize request.
2. Classify intent.
3. Classify decision type.
4. Load compiled index.
5. Match policies by:
   - decision type
   - scope
   - tags
   - aliases
   - keywords
   - linked restrictions
   - linked tradeoffs
6. Apply secrets and safety rules.
7. Apply hard-no rules.
8. Apply privacy boundaries.
9. Apply approval rules.
10. Apply recent explicit correction.
11. Score options against tradeoff rules.
12. Check examples/counterexamples.
13. Use graph neighbors for nearby policies.
14. Optionally query FTS/vector if already indexed and cheap.
15. Estimate confidence.
16. Return:
   - proceed
   - ask_user
   - block
   - needs_more_context
   - defer
   - no_action
17. Append decision attempt to ledger if configured.
18. Never call LLM.
19. Never call AgentMemory.
20. Never generate embeddings.

Default outcome rules:

Proceed when:
- confidence >= threshold, default 0.82
- no hard restriction violated
- no unresolved policy contradiction
- low-risk or reversible medium-risk
- approval not required
- privacy/secrets/external actions not involved
- enough context exists

Ask user when:
- confidence between 0.50 and threshold
- policies conflict
- decision affects stable policy
- user has required approval for class
- action is not clearly reversible
- privacy boundary may apply

Block when:
- hard-no violated
- secret involved
- destructive external action without approval
- private memory would be sent remotely without explicit approval
- never-auto tier applies

Needs more context when:
- situation under-specified
- options unclear
- proposed action missing
- decision type cannot be classified

Defer/no_action when:
- decision is not needed yet
- cheaper reversible step exists
- evidence threshold not met
- doing nothing is better than premature choice

---

# 12. Harness contract

Create `docs/harness-contract.md`.

Harness rule:

Before asking the user a decision question, the harness must ask Brainmap.

Flow:

Agent wants to ask user.
Harness calls:
brainmap gate --intent would-ask-user --situation ... --options ... --json

If outcome is:
- proceed: harness proceeds with selected option and does not ask user.
- ask_user: harness asks only returned `askUserQuestion`.
- needs_more_context: harness gathers context or asks a narrower question.
- block: harness does not proceed.
- defer/no_action: harness does nothing or delays.

Before meaningful action, harness must call gate.

Meaningful actions:
- important file write
- config change
- install integration
- delete/rename
- external call
- remote model call with personal data
- changing stable decision policy
- enabling hooks/autopilot
- exporting private data
- restoring over existing vault

After action:
brainmap record-decision

After user correction:
brainmap learn-feedback

Classify integrations as:
- enforced
- best-effort
- instruction-only

Do not claim universal enforcement where host does not support it.

---

# 13. Agent integrations

Implement installers:

brainmap install harness --target claude-code --global
brainmap install harness --target claude-code --project .
brainmap install harness --target codex --global
brainmap install harness --target codex --project .
brainmap install harness --target opencode --global
brainmap install harness --target opencode --project .
brainmap install harness --target copilot --project .
brainmap install harness --target generic-stdio
brainmap install harness --dry-run

Installer rules:
- detect existing files
- show plan
- backup before writing
- never overwrite without backup
- insert managed blocks if modifying existing file
- support dry-run
- support uninstall
- label enforcement level

Install skills/instructions for:

- build-decision-engine
- brainmap-context
- brainmap-ask
- brainmap-review
- brainmap-gate

Primary skill:

build-decision-engine

Skill behavior:

- AgentMemory optional.
- Ask whether to build from historical decision traces, direct calibration questions, current session, export file, or hybrid.
- If AgentMemory absent, start interview mode.
- Learn how user decides, not what user knows.
- Use update packets.
- Ask high-leverage calibration questions.
- Do not store project archive.
- Do not store raw code.
- Do not store secrets.
- Use Brainmap gate before asking user whenever possible.

---

# 14. Optional AgentMemory source

AgentMemory is optional.

Implement source adapters:

src or crate modules:
- interactive-source
- current-session-source
- manual-markdown-source
- agentmemory-rest-source
- agentmemory-mcp-source
- agentmemory-export-source

All produce generic `BrainmapSignal`.

AgentMemory adapter must:
- never be required
- fail gracefully
- offer interview fallback
- ask before broad memory access
- use secret env var name, not secret values
- support REST if available
- support MCP if available
- support export file
- extract only decision evidence

Search historical data for:
- user chose
- user rejected
- user corrected
- user preferred
- user refused
- user changed direction
- user said should
- user said should not
- ask me
- don't ask me
- approval
- tradeoff
- restriction
- default
- decision

Discard:
- code content
- project chronology
- command logs
- generic facts

---

# 15. Optional heavy LLM use

Brainmap must not require its own LLM.

Harness LLM is enough for conversation, explanation, calibration, and update-packet drafting.

Heavy models like Fable or GPT-5.5 may be used only in slow path with explicit approval.

Rules:
- no LLM in gate
- no LLM in daemon idle
- no hidden background LLM
- no remote model call with private memory without explicit approval
- LLM output creates pending packets only
- stable policy changes require validation/approval

If no LLM API is available:
- write prompt files for manual use
- continue with deterministic/interview mode

---

# 16. Embedded local embedding model

Hard requirement:

Use `minishlab/potion-base-8M` as the default embedding model.

Use Model2Vec Rust inference if safe and maintained.

External embedding providers are not supported in MVP.

Runtime model downloads are not supported in MVP.

The embedding model must be available in the binary or packaged as a compile-time embedded model pack.

Implementation:

- Download model files at build time, verify checksums, create compressed pack in Cargo `OUT_DIR`, then embed with `include_bytes!`.
- Pack contains:
  - model files required by Model2Vec
  - tokenizer/config files if needed
  - license
  - source metadata
  - model-manifest.json
  - SHA-256 checksums

At runtime:

brainmap models materialize

Must:
1. Verify embedded pack SHA-256.
2. Extract to temp dir.
3. Verify every file checksum.
4. Atomically move to:
   BrainMap/.brainmap/models/<model-id>/<hash>/
5. Never overwrite verified model unless `--force`.
6. Never contact network.
7. Fail closed if verification fails.
8. Disable embeddings if verification fails.
9. Never fall back to network.

Hot path:
- must not load embedding model.
- must not generate embeddings.

Slow path:
- `brainmap embed rebuild`
- `brainmap embed process --missing-only`
- `brainmap dream --mode embed`

Use embeddings only for:
- candidate retrieval
- near-duplicate detection
- similar decision examples
- clustering suggestions
- related-policy discovery

Do not use vector result as final decision authority.

Final authority:
- safety rules
- hard restrictions
- approval rules
- decision policies
- tradeoff rules
- confidence threshold

---

# 17. SQLite index

Create:

BrainMap/.brainmap/brainmap.sqlite

Tables:

- schema_migrations
- notes
- policies
- tradeoff_rules
- hard_restrictions
- soft_preferences
- approval_rules
- ask_triggers
- decision_examples
- counterexamples
- wrong_decisions
- corrected_decisions
- calibration_questions
- decision_ledger
- update_packets
- graph_nodes
- graph_edges
- fts_notes
- vector_embeddings
- imports
- exports
- index_manifest

Use SQLite transactions.

The SQLite DB is rebuildable from Markdown + ledgers.

Implement:

brainmap index rebuild
brainmap index status
brainmap index verify

Gate should use index if valid.

If index stale:
- warn
- use last valid index if safe
- never rebuild automatically in hot path unless explicit flag

---

# 18. Graph

Implement graph as embedded file-based data:

Default:
- SQLite graph_nodes
- SQLite graph_edges
- optional petgraph in-memory traversal for local neighborhood scoring

Edge kinds:
- parent
- related
- tradeoff
- restriction
- approval
- asks
- contradicts
- example-of
- counterexample-of
- supersedes
- scope-of

Commands:

brainmap graph neighbors <id>
brainmap graph path <from> <to>
brainmap graph orphans

---

# 19. Search

Implement:

brainmap search --text "query"
brainmap search --vector "query"
brainmap search --hybrid "query"

Text:
- SQLite FTS5

Vector:
- sqlite-vec or latest secure embedded SQLite vector alternative
- requires precomputed embeddings
- if embeddings absent, return helpful error

Hybrid:
- structured + FTS + vector + graph neighborhood

Do not use search result as final gate authority without policy scoring.

---

# 20. Import/export/restore

First-class support.

Export modes:

1. portable
2. full
3. share-safe
4. encrypted

Archive:
- `.brainmap.tar.zst`
- `.brainmap.tar.zst.age` for encrypted

Every export includes:

manifest.json:
{
  "format": "brainmap-export",
  "formatVersion": 1,
  "createdAt": "ISO",
  "brainmapVersion": "0.1.0",
  "exportMode": "portable|full|share-safe|encrypted",
  "schemaVersion": "decision-engine-v1",
  "includesIndexes": false,
  "includesEmbeddings": false,
  "includesPrivateNotes": true,
  "encrypted": false,
  "files": [
    {
      "path": "DECISIONMAP.md",
      "sha256": "..."
    }
  ]
}

Portable includes:
- Markdown vault
- config without secrets
- decision ledger
- pending update packets
- schemas
- manifest
- checksums
- migration metadata

Portable excludes:
- model cache
- raw AgentMemory export
- secrets
- temporary indexes
- large backups by default

Full may include:
- SQLite index
- vectors
- graph tables
- decision ledger
- materialized model directory if explicitly requested

Share-safe:
- redact/exclude private notes
- exclude secret-sensitive data
- exclude raw evidence
- exclude raw AgentMemory refs
- scrub personal identifiers unless allowed

Encrypted:
- age/rage-compatible encryption
- no weak crypto
- verify decrypt/restore path

Import/restore must:
- validate manifest
- verify checksums
- reject path traversal
- never overwrite without backup
- run migrations if needed
- rebuild index unless full restore with valid index
- link-check
- gate smoke test
- write import report

---

# 21. Read-only Web UI

Build a read-only web UI.

Command:

brainmap web --vault ~/BrainMap --host 127.0.0.1 --port 8777
brainmap web --open
brainmap web export-static --out ./brainmap-web

Requirements:
- read-only
- local by default
- no write endpoints
- no remote fonts/assets/CDNs
- no analytics
- no external network
- modern, premium, innovative
- dark mode first
- keyboard/search friendly
- accessible contrast
- fast loading
- works from SQLite index and static assets
- no secrets exposed
- private notes hidden unless allowed by config

Visual concept:
- Brainmap Decision Engine Explorer
- central brain image/visualization with labeled sections:
  - Decision Identity
  - Tradeoff Models
  - Restrictions
  - Choice Patterns
  - Question Triggers
  - Calibration
  - Examples
- clicking a brain section explodes/expands it into graph/map view
- graph shows policies, rules, examples, counterexamples, restrictions
- right side shows selected policy cards
- bottom/side panel shows engine insights:
  - total policies/rules
  - recent decisions
  - confidence trend
  - stale policies
  - coverage
  - calibration score
- status chips:
  - Read-only
  - Shadow Mode
  - Autopilot: Conservative
- sidebar sections mirror vault ontology
- search across policies/examples/tradeoffs

Implement practical MVP version:
- SVG/canvas brain map or stylized section map
- graph visualization using secure open-source library after research
- if no graph library is suitable, implement simple SVG graph
- no write capabilities

Acceptance must include UI tests or snapshot/static route tests where practical.

---

# 22. Daemon

Binary:

brainmapd

Commands:

brainmapd start
brainmapd stop
brainmapd status

Daemon:
- optional
- low-memory
- local-only
- loads compiled index
- watches index manifest/mtime
- exposes local socket or localhost-only HTTP
- no LLM
- no AgentMemory
- no embedding model loaded while idle
- no network except localhost API if enabled
- gate requests return JSON
- direct CLI fallback if daemon absent

Targets:
- warm daemon gate decision under 50 ms target
- direct CLI gate under 200 ms target
- capture append under 20 ms target
- idle daemon under 50 MB target if practical
- document actual observed values

---

# 23. Performance controls

Implement:

brainmap bench --vault ./tmp/BrainMap

Bench:
- gate from SQLite index
- direct local gate
- daemon gate if daemon available
- capture append
- index load
- FTS query
- vector query with precomputed vector
- graph neighbors

The system must document:
- disk use
- memory use
- CPU use
- benchmark environment

Design targets:
- no model call in gate
- no AgentMemory call in gate
- no full vault scan in gate
- no embedding generation in gate
- capture hook under 20 ms target
- daemon idle under 50 MB target if practical
- embedding generation is explicit slow path

---

# 24. Privacy and safety

Implement redaction for:

- API keys
- tokens
- passwords
- private keys
- SSH keys
- .env content
- bearer tokens
- auth headers
- cookies
- payment info
- secrets
- private identifiers where configured

Default privacy mode:
local-first

Modes:
- strict
- local-first
- open

Never store:
- secrets
- credentials
- raw private keys
- raw code dumps
- raw transcripts by default
- project chronology as durable memory
- private memory sent remotely without approval

Autopilot never decides automatically for:
- secrets
- credentials
- private keys
- payment
- legal/medical/financial advice
- irreversible deletion
- external data sharing
- sending private memory to remote models
- changing hard-no rules
- disabling privacy protections
- enabling broad surveillance/capture
- spending money
- identity/account actions

Autopilot may decide automatically for:
- low-risk formatting
- reversible local file organization
- established user defaults
- adding pending update packets
- generating context packs
- suppressing redundant questions
- local-only reversible edits when policy is clear

---

# 25. Transactionality and concurrency

Implement:
- file locking
- SQLite transactions
- atomic writes through temp file + rename
- backups before modifying user files
- rollback

Commands:

brainmap snapshot create
brainmap snapshot list
brainmap snapshot restore <id>
brainmap rollback last

Rules:
- one writer at a time
- many readers allowed
- ledger append atomic
- index rebuild cannot corrupt existing valid index
- daemon reloads only after valid rebuild
- import/restore never overwrites without backup

---

# 26. Staleness and decay

Policies include:
- created
- updated
- last_confirmed
- decay
- status

Decay values:
- never
- slow
- normal
- fast

Monthly review:
- identify stale policies
- identify contradicted policies
- identify unconfirmed assumptions
- ask calibration questions
- never decay hard safety rules like “never store secrets”

---

# 27. Decision benchmark and eval

Create a local decision eval suite.

Fixtures:

fixtures/decision-bench/
  architecture.jsonl
  tooling.jsonl
  privacy.jsonl
  model-use.jsonl
  workflow.jsonl
  ambiguous.jsonl
  hard-no.jsonl

Each line:

{
  "id": "bench_storage_v1",
  "situation": "Choose storage for v1 personal decision engine",
  "options": ["Markdown+JSONL", "SQLite", "External Vector DB"],
  "risk": "low",
  "reversible": true,
  "expectedOutcome": "proceed",
  "expectedChoice": "Markdown+JSONL",
  "mustAskUser": false,
  "reason": "User prefers local-first simple v1 systems"
}

Command:

brainmap eval --vault ~/BrainMap --suite fixtures/decision-bench

Metrics:
- false proceed rate
- false ask rate
- false block rate
- wrong choice rate
- confidence calibration
- policy coverage
- ambiguity detection

Shadow mode should use eval-like reports.

---

# 28. AGENTS.md

Create strong `AGENTS.md`.

Must instruct future coding agents:

- This repo builds a local Brainmap Decision Engine.
- It is a decision engine, not an information engine.
- Markdown is canonical.
- SQLite is a rebuildable compiled index.
- The hot path must remain deterministic and fast.
- No LLM in gate.
- No AgentMemory in gate.
- No embedding generation in gate.
- No external embedding providers.
- No runtime model downloads.
- The default embedding model is embedded `minishlab/potion-base-8M`.
- Use update packets.
- Preserve wikilinks.
- Never store secrets.
- Every schema change needs tests.
- Every installer needs dry-run and backup.
- Every import/export path needs checksum validation.
- Every gate change needs eval tests.
- Run tests before final answer.

---

# 29. Security/supply chain

Add:
- `deny.toml`
- cargo-audit workflow or documented command
- cargo-deny workflow or documented command
- cargo-cyclonedx SBOM command
- SECURITY.md
- threat model doc

License policy:
Allow:
- MIT
- Apache-2.0
- BSD-2-Clause
- BSD-3-Clause
- MPL-2.0
- ISC
- Unicode-3.0

Deny:
- GPL
- AGPL
- unknown licenses
- yanked crates
- critical unresolved advisories
- unmaintained critical dependencies

Commands:
- cargo fmt --check
- cargo clippy --all-targets --all-features -- -D warnings
- cargo test
- cargo audit
- cargo deny check
- cargo cyclonedx

If a security tool cannot run in environment, document why and provide exact command.

---

# 30. Subagent-driven development plan

Use subagents.

Create or simulate these subagents:

1. Research subagent
   - model: GPT-5.3 Spark or fastest capable
   - output: docs/research-notes.md, dependency candidates

2. Architecture subagent
   - model: GPT-5.5
   - output: docs/architecture.md, crate/module plan

3. Core engine subagent
   - model: GPT-5.5 or GPT-5.4
   - output: vault/index/gate implementation

4. Storage/search/vector/graph subagent
   - model: GPT-5.4
   - output: SQLite schema, FTS, vector trait, graph tables

5. Embedded model subagent
   - model: GPT-5.5 for careful packaging/security
   - output: model pack logic, verify/materialize commands

6. Harness/integration subagent
   - model: GPT-5.4
   - output: skills/hooks/installers/contracts

7. Web UI subagent
   - model: GPT-5.4 or GPT-5.5
   - output: read-only UI

8. Security/privacy subagent
   - model: GPT-5.5
   - output: redaction, threat model, audit setup

9. Test/eval subagent
   - model: GPT-5.5
   - output: full test suite, fixtures, eval suite

10. Performance subagent
   - model: GPT-5.4
   - output: bench command and measurements

Main coordinator must reconcile outputs, remove contradictions, and ensure acceptance criteria pass.

---

# 31. Implementation order

Build vertical slices in this order:

## Slice 1: working CLI and vault
- Rust project builds.
- `brainmap init`
- `brainmap init-vault`
- starter decision vault
- frontmatter parser
- wikilink parser
- link-check

## Slice 2: SQLite index and gate
- index schema
- index rebuild
- sample policies indexed
- `brainmap gate --json`
- no LLM/network/AgentMemory in gate
- decision ledger append

## Slice 3: interview and update packets
- `build-decision-engine --mode interview`
- question engine
- update packets
- apply packets
- rebuild index

## Slice 4: search/vector/graph
- FTS search
- embedded model pack interface
- model materialize/verify
- vector trait
- sqlite vector table
- graph nodes/edges
- graph commands

## Slice 5: import/export/restore
- portable export
- verify export
- dry-run import
- restore
- share-safe redaction
- encrypted export if feasible

## Slice 6: harness integrations
- install dry-runs
- skill files
- AGENTS fragments
- generic stdio harness
- MCP tools

## Slice 7: web UI
- local read-only web server
- brain section visualization
- expandable graph view
- search and insights
- no write endpoints

## Slice 8: security/performance/eval
- cargo audit/deny/cyclonedx
- bench
- eval suite
- final docs

---

# 32. Tests

Write tests for:

## Vault and Markdown
- create vault
- note frontmatter parse
- frontmatter preserve/update
- invalid schema rejected
- wikilinks parse:
  - [[foo]]
  - [[foo|bar]]
  - [[dir/foo]]
- broken links detected
- ambiguous links detected
- orphan notes detected
- parent link missing detected

## Gate
- clear low-risk decision returns proceed
- ambiguous decision returns ask_user
- hard-no returns block
- privacy-sensitive action requires ask_user/approval
- irreversible action asks/blocks
- reversible low-risk can proceed
- threshold respected
- conflicting policies ask_user
- no_action/defer valid
- `should-ask-user` suppresses redundant question
- JSON schema stable
- gate does not call LLM
- gate does not call AgentMemory
- gate does not generate embeddings
- gate does not full scan vault when index valid

## Update packets
- valid packet accepted
- secret packet rejected/redacted
- apply append
- apply merge
- reject unsafe overwrite
- preserve human content
- rebuild index after apply

## Interview/calibration
- interview works without AgentMemory
- answers create update packets
- pairwise questions generated
- duplicate questions deduped
- answered questions not repeated
- corrections become high-weight evidence

## Autopilot
- default shadow mode
- conservative promotion requires criteria
- kill switch disables autopilot
- env var disables gate enforcement
- silence is weak evidence
- approval medium/high evidence
- correction strong evidence

## AgentMemory optional
- unavailable fallback to interview
- REST mock works
- MCP mock works if implemented
- export fixture parse
- secret env var handled without storing secret
- project/code details discarded

## Embedded model
- model pack hash verification
- materialize without network
- verify extracted files
- failed verification disables embedding
- no runtime download path exists
- gate never loads model
- embed rebuild uses materialized local model

## Search/vector/graph
- FTS returns expected policy
- vector search uses precomputed vectors
- vector search errors gracefully when no embeddings
- graph neighbors returns linked nodes
- graph orphans detected
- hybrid search combines sources

## Import/export
- portable export creates archive
- manifest includes checksums
- verify-export passes
- tampered archive fails
- path traversal rejected
- dry-run import writes nothing
- restore to new vault works
- restore backs up before overwrite
- share-safe redacts private data
- encrypted export/restore if implemented

## Privacy/security
- API keys redacted
- bearer tokens redacted
- private keys redacted
- .env redacted
- prompt injection in imported content ignored
- untrusted evidence cannot override control docs

## Web UI
- web server binds localhost by default
- read-only API only
- no write endpoints
- static export works
- search endpoint read-only
- graph endpoint read-only
- private data hidden according to config

## Installers
- dry-run writes nothing
- backup existing files
- managed block insert/update/remove
- uninstall removes only managed files/blocks
- each integration labels enforcement level

## Performance
- bench command runs
- capture append timed
- gate direct timed
- index load timed
- daemon gate timed if daemon implemented

---

# 33. Acceptance criteria

The project is acceptable only when these are true or explicitly documented as not possible in the current environment with a strong fallback.

## Build and basic commands

1. `cargo fmt --check` passes.
2. `cargo clippy --all-targets --all-features -- -D warnings` passes or documented warnings are fixed/justified.
3. `cargo test` passes.
4. `brainmap --help` works.
5. `brainmap init --dry-run` works.
6. `brainmap init` creates config.
7. `brainmap init-vault --vault ./tmp/BrainMap --yes` creates vault.
8. `brainmap link-check --vault ./tmp/BrainMap` reports no broken starter links.

## Decision engine

9. `brainmap index rebuild --vault ./tmp/BrainMap` creates `./tmp/BrainMap/.brainmap/brainmap.sqlite`.
10. `brainmap index status --vault ./tmp/BrainMap` reports valid index.
11. `brainmap gate --intent would-ask-user --situation "Choose v1 storage" --options "Markdown+JSONL|SQLite|External Vector DB" --risk low --reversible true --vault ./tmp/BrainMap --json` returns machine-readable JSON.
12. Clear low-risk decisions return `proceed`.
13. Ambiguous decisions return `ask_user` with a focused question.
14. Hard-no decisions return `block`.
15. Privacy-sensitive decisions require approval.
16. `brainmap should-ask-user --json` works.
17. `brainmap decide` produces human-readable decision output.
18. `brainmap record-decision` appends compact decision ledger entry.
19. `brainmap learn-feedback` creates high-strength update packet.
20. `brainmap autopilot status` shows mode, threshold, and level.
21. Default autopilot mode is `shadow`.
22. Kill switch env var disables autopilot/gate enforcement.

## Hot path

23. `brainmap gate --json` does not call an LLM.
24. `brainmap gate --json` does not call AgentMemory.
25. `brainmap gate --json` does not access network.
26. `brainmap gate --json` does not generate embeddings.
27. `brainmap gate --json` does not load embedded model.
28. `brainmap gate --json` does not scan full vault when valid index exists.
29. `brainmap capture --stdin` appends compact JSONL event.
30. `brainmap bench --vault ./tmp/BrainMap` reports gate/capture timing.

## Interview and learning

31. `brainmap build-decision-engine --mode interview --dry-run` works without AgentMemory.
32. `brainmap build-decision-engine --mode interview --vault ./tmp/BrainMap --questions 7` can produce questions.
33. Interview answers create update packets.
34. Applying interview update packets creates linked Markdown notes.
35. `brainmap build-brain` works as alias.
36. The system never says AgentMemory is required.
37. The system offers interview fallback when AgentMemory is absent.

## AgentMemory optional

38. `brainmap build-decision-engine --mode auto` does not fail when AgentMemory is absent.
39. `brainmap build-decision-engine --mode agentmemory` gracefully handles connection failure and offers fallback.
40. `brainmap build-decision-engine --mode export --file fixtures/agentmemory/sample.json --dry-run` works if fixture provided.
41. AgentMemory extraction discards project/code details and keeps decision traces.

## Search/vector/graph

42. `brainmap search --text "local first"` uses FTS.
43. `brainmap graph neighbors <id>` works.
44. `brainmap graph orphans` works.
45. `brainmap models status` works.
46. `brainmap models materialize` succeeds without internet using embedded model pack, if model assets are present.
47. `brainmap models verify` verifies pack/extracted checksums.
48. `brainmap embed rebuild` uses only embedded/materialized model.
49. No external embedding provider code path exists in MVP.
50. No runtime model download path exists in MVP.
51. If model assets cannot be included due environment constraints, implement the full model-pack interface, tests with a tiny fixture model pack, and document exact steps to add `potion-base-8M` assets before release.

## Import/export/restore

52. `brainmap export --mode portable --vault ./tmp/BrainMap --out ./tmp/brainmap.brainmap.tar.zst` creates archive.
53. Export contains manifest and checksums.
54. `brainmap verify-export ./tmp/brainmap.brainmap.tar.zst` passes.
55. Tampered export fails verification.
56. `brainmap import --file ./tmp/brainmap.brainmap.tar.zst --to ./tmp/Imported --dry-run` validates without writing.
57. `brainmap restore --file ./tmp/brainmap.brainmap.tar.zst --to ./tmp/Restored` restores.
58. Restore rebuilds index or verifies included index.
59. Restore runs link-check.
60. Restore runs gate smoke test.
61. Share-safe export redacts private material.
62. Encrypted export/restore works or is stubbed behind documented feature if encryption crate integration cannot be completed in the environment.

## Web UI

63. `brainmap web --vault ./tmp/BrainMap --host 127.0.0.1 --port 8777` serves local read-only UI.
64. Web UI has no write endpoints.
65. Web UI has no external CDN/fonts/assets.
66. Web UI shows brain sections:
    - Decision Identity
    - Tradeoff Models
    - Restrictions
    - Choice Patterns
    - Question Triggers
    - Calibration
    - Examples
67. Clicking/selecting section shows expanded graph/map.
68. Web UI shows policy cards and graph relationships.
69. Web UI shows read-only status, shadow/autopilot status, and insights.
70. `brainmap web export-static` works or is documented if deferred.

## Harness and integrations

71. `brainmap install harness --target claude-code --dry-run` prints planned files.
72. `brainmap install harness --target codex --dry-run` prints planned files.
73. `brainmap install harness --target opencode --dry-run` prints planned files.
74. `brainmap install harness --target copilot --dry-run` prints planned files.
75. `brainmap install harness --target generic-stdio --dry-run` prints planned files.
76. Installers back up existing files before writing.
77. Installers support uninstall.
78. Installed skills tell agents to ask Brainmap before asking user.
79. Integrations label enforcement as enforced/best-effort/instruction-only.
80. Generic stdio harness can enforce decision gate.

## Security/privacy

81. Redaction tests pass for API keys, bearer tokens, private keys, .env content, cookies, auth headers.
82. Prompt injection in imported content cannot override control docs.
83. Secret sensitivity packets are rejected/redacted.
84. No secrets are written to config/export.
85. Remote model use with private memory requires approval.
86. MCP exposes only allowlisted tools.
87. No arbitrary shell execution through MCP.

## Transaction/concurrency

88. Writes are atomic.
89. File locks prevent concurrent corrupting writes.
90. SQLite transactions are used.
91. Snapshot create/list/restore works.
92. Rollback last works for at least vault/index changes.

## Eval/shadow/autopilot

93. `brainmap eval --vault ./tmp/BrainMap --suite fixtures/decision-bench` runs.
94. Eval reports false proceed, false ask, false block, wrong choice, confidence, coverage.
95. Shadow mode records predictions without enforcing.
96. Promotion rules prevent automatic jump to active autopilot.
97. Corrections produce high-weight learning events.

## Supply chain

98. `cargo audit` passes or advisories are documented with risk decision.
99. `cargo deny check` passes or blocked by environment with documentation.
100. `cargo cyclonedx` produces SBOM or exact command is documented.
101. License policy denies GPL/AGPL/unknown critical dependencies.
102. SECURITY.md exists.
103. Threat model exists.

## Documentation

104. README explains:
    - what Brainmap is
    - what it is not
    - hot path vs slow path
    - how to init
    - how to build from interview
    - how to use optional AgentMemory
    - how to run gate
    - how harnesses should integrate
    - how to run web UI
    - how to import/export/restore
    - how embeddings work offline
    - privacy rules
    - performance expectations
105. docs/acceptance-report.md lists every criterion and status.
106. docs/implementation-notes.md lists assumptions, fallbacks, and unresolved caveats.

---

# 34. Demo scenario

Implement fixtures/demo so this works:

cargo test
cargo run -- brainmap init-vault --vault ./tmp/DemoBrainMap --yes
cargo run -- brainmap index rebuild --vault ./tmp/DemoBrainMap
cargo run -- brainmap gate --intent would-ask-user --situation "Choose storage for v1 Brainmap engine" --options "Markdown+JSONL|SQLite|External Vector DB" --risk low --reversible true --vault ./tmp/DemoBrainMap --json
cargo run -- brainmap build-decision-engine --mode interview --vault ./tmp/DemoBrainMap --questions 7 --dry-run
cargo run -- brainmap search --text "local first" --vault ./tmp/DemoBrainMap
cargo run -- brainmap export --mode portable --vault ./tmp/DemoBrainMap --out ./tmp/demo.brainmap.tar.zst
cargo run -- brainmap verify-export ./tmp/demo.brainmap.tar.zst
cargo run -- brainmap web --vault ./tmp/DemoBrainMap --host 127.0.0.1 --port 8777

The demo must show:

- decisions, not information notes
- Markdown canonical vault
- SQLite index
- gate output
- read-only web UI
- export/verify path

---

# 35. Final response requirements

When done, respond with:

## Built

Short summary.

## Architecture

Short summary.

## Commands run

List exact commands and outputs/results.

## Acceptance status

Summarize pass/fail/deferred.

## Security status

cargo audit/deny/SBOM status.

## Performance status

bench results or current observations.

## How I run it

Give exact commands.

## Caveats

Be honest.

Do not claim completion if tests fail. Fix failures if possible before final response.
