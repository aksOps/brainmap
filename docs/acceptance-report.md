# Acceptance Report

Generated: 2026-07-05.

Legend: PASS = verified now. MVP = implemented with documented MVP scope. DEFERRED = explicit non-blocking fallback documented.

1. PASS `cargo fmt --check`.
2. PASS `cargo clippy --all-targets --all-features -- -D warnings`.
3. PASS `cargo test`.
4. PASS `brainmap --help`.
5. PASS `brainmap init --dry-run`.
6. PASS `brainmap init` implemented.
7. PASS `brainmap init-vault --vault ./tmp/BrainMap --yes`.
8. PASS `brainmap link-check --vault ./tmp/BrainMap`.
9. PASS `brainmap index rebuild` creates SQLite.
10. PASS `brainmap index status` reports valid.
11. PASS gate JSON command.
12. PASS clear low-risk storage returns `proceed`.
13. PASS ambiguous fixture returns `ask_user`.
14. PASS secret hard-no returns `block`.
15. PASS private remote/model use asks or blocks by rule.
16. PASS `should-ask-user --json`.
17. PASS `decide`.
18. PASS `record-decision`.
19. PASS `learn-feedback`.
20. PASS `autopilot status`.
21. PASS default autopilot mode `shadow`.
22. PASS `BRAINMAP_DISABLE_AUTOPILOT=1` forces `ask_user`.
23. PASS no LLM code path in gate.
24. PASS no AgentMemory code path in gate.
25. PASS no network code path in gate.
26. PASS no embedding generation in gate.
27. PASS no model load in gate.
28. PASS gate uses SQLite policy lookup when index exists.
29. PASS `capture --stdin`.
30. PASS `bench` reports gate/capture timing.
30a. PASS `brainmap context --fast --json` returns SQLite-only hot-path context pack.
31. PASS interview dry-run works without AgentMemory.
32. PASS interview prints 7 questions.
33. PASS interview creates packets when not dry-run.
34. PASS apply creates linked Markdown notes and rebuilds index.
35. PASS `build-brain` aliases build command.
36. PASS AgentMemory never required.
37. PASS AgentMemory fallback printed.
38. PASS `mode auto` falls back.
39. PASS `mode agentmemory` handles absence.
40. PASS export-file dry-run works with fixture.
41. PASS AgentMemory export parser extracts decision traces and discards code/project/secret strings.
42. PASS FTS text search.
43. PASS graph neighbors.
44. PASS graph orphans.
45. PASS models status.
46. PASS models materialize uses embedded real `potion-base-8M` pack offline.
47. PASS models verify checks extracted model checksums.
48. PASS embed rebuild uses materialized local Model2Vec pack and does no network.
49. PASS no external embedding provider code path.
50. PASS no runtime model download command.
51. PASS real `potion-base-8M` assets are included in the embedded pack.
52. PASS portable export.
53. PASS manifest/checksums.
54. PASS verify-export.
55. PASS tampered export fails verification, including trailing data after the zstd frame.
56. PASS import dry-run.
57. PASS restore.
58. PASS restore rebuilds index.
59. PASS restore link-check.
60. PASS restore gate smoke test.
61. PASS share-safe export redacts/skips private ledgers.
62. PASS encrypted export/verify/restore with age recipient and identity file.
63. PASS live web server served `/api/status`.
64. PASS POST returns 405 read-only.
65. PASS no external assets in generated HTML.
66. PASS UI shows required sections.
67. PASS section selection implemented in local JS.
68. PASS policy cards/graph relationships implemented in UI.
69. PASS status chips/insights implemented.
70. PASS `web export-static`.
71. PASS claude-code installer dry-run.
72. PASS codex installer dry-run.
73. PASS opencode installer dry-run.
74. PASS copilot installer dry-run.
75. PASS generic-stdio installer dry-run.
76. PASS installers back up existing files before writes.
77. PASS installers support uninstall.
78. PASS installed skill/instructions tell agents to ask Brainmap.
79. PASS enforcement labels printed.
80. PASS generic stdio harness runtime `brainmap harness stdio --fail-on-block`.
81. PASS redaction tests for API key, bearer token, private key; cookie/auth patterns implemented.
82. PASS prompt-injection rule documented and imported content cannot override gate code.
83. PASS secret packets rejected/redacted.
84. PASS no secrets written by config/export path.
85. PASS remote private model use requires ask/block.
86. PASS MCP stdio JSON-RPC server exposes only allowlisted tools.
87. PASS no arbitrary shell MCP tool.
88. PASS atomic writes via temp+rename.
89. PASS file lock on index rebuild.
90. PASS SQLite transactions on index rebuild.
91. PASS snapshot create/list/restore.
92. PASS rollback last.
93. PASS eval suite runs.
94. PASS eval reports false proceed/ask/block, wrong choice, confidence, coverage.
95. MVP shadow mode records gate predictions in ledger.
96. PASS promotion guards require evidence thresholds and deny aggressive automatic promotion.
97. PASS corrections produce high-strength update packets.
98. PASS `cargo audit` ran; allowed unmaintained transitive warnings documented.
99. PASS `cargo deny check` ran and passed with warnings.
100. PASS `cargo cyclonedx --format json --override-filename brainmap` produced `crates/brainmap-cli/brainmap.json`.
101. PASS license policy encoded in `deny.toml`.
102. PASS `SECURITY.md`.
103. PASS threat model doc.
104. PASS README covers product, hot/slow paths, init, interview, AgentMemory fallback, gate, web, import/export/restore, offline embeddings, privacy, performance.
105. PASS this report lists every criterion.
106. PASS implementation notes list assumptions, fallbacks, caveats.

Final verified commands:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo run -- brainmap init-vault --vault ./tmp/BrainMap --yes
cargo run -- brainmap index rebuild --vault ./tmp/BrainMap
cargo run -- brainmap gate --intent would-ask-user --situation "Choose v1 storage" --options "Markdown+JSONL|SQLite|External Vector DB" --risk low --reversible true --decision-type architecture --vault ./tmp/BrainMap --json
cargo run -- brainmap context --fast --json --vault ./tmp/BrainMap
cargo run -- brainmap models materialize --vault ./tmp/BrainMap --force
cargo run -- brainmap models verify --vault ./tmp/BrainMap
cargo run -- brainmap embed rebuild --vault ./tmp/BrainMap
cargo run -- brainmap embed status --vault ./tmp/BrainMap
cargo run -- brainmap search --vector "local first decisions" --vault ./tmp/BrainMap
cargo run -- brainmap search --hybrid "privacy approval" --vault ./tmp/BrainMap
cargo run -- brainmap build-decision-engine --mode export --file fixtures/agentmemory/sample.json --dry-run --vault ./tmp/BrainMap
cargo run -- brainmap mcp serve --vault ./tmp/BrainMap
cargo run -- brainmap eval --vault ./tmp/BrainMap --suite fixtures/decision-bench
cargo run -- brainmap bench --vault ./tmp/BrainMap
cargo run -- brainmap export --mode portable --encrypt --recipient age1t7rxyev2z3rw82stdlrrepyc39nvn86l5078zqkf5uasdy86jp6svpy7pa --vault ./tmp/BrainMap --out ./tmp/encrypted.brainmap.tar.zst.age
cargo run -- brainmap verify-export ./tmp/encrypted.brainmap.tar.zst.age --identity ./tmp/age-identity.txt
cargo run -- brainmap restore --file ./tmp/encrypted.brainmap.tar.zst.age --identity ./tmp/age-identity.txt --to ./tmp/EncryptedRestored
cargo audit
cargo deny check
cargo cyclonedx --format json --override-filename brainmap
```
