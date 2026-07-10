# M001 qualification evidence — `d93ea7a`

This directory retains the machine-readable qualification output for the stable
M1–M7 code candidate. It is synthetic local qualification evidence, not the
M8 real-host or dogfood report.

## Provenance

- Source commit: `d93ea7ada09043a1422478b60d5f3223037ff8ad`
- Source tree before qualification: clean (`sourceTreeDirty: false`)
- Host: Linux `6.8.0-134-generic`, x86_64, 6 logical CPUs
- Toolchain: Rust/Cargo 1.95.0
- `brainmap` SHA-256:
  `cb0f6ed0f544880b5c526b619de16af25ff5cd7cfa7db6eae3c6e3c69a8df75d`
- `brainmapd` SHA-256:
  `f8a94343f12f20dc6344686934f114deda15ebea0c4f2d9ebea0e2ad03b2c078`
- Portable archive SHA-256:
  `1373a1226d36fb8c5ad9f314b211bfe6f4c5763869d9373b7185b6b03c3ba421`

The qualification command was:

```bash
BRAINMAP_QUALIFICATION_OUT=target/m001-qualification-d93ea7a \
  scripts/release-qualification.sh
```

The script rebuilt locked optimized binaries, generated a clean synthetic
vault, ran the release-binary eval and scale drills, exercised learned,
corrected, and policy decisions before and after export/restore, and injected
failure at every restore transition. The retained JSON was checked to contain
no workspace or temporary-vault paths.

## Functional result

[`eval.json`](eval.json) records 238 deterministic cases:

| Contract | Result |
| --- | ---: |
| Exact learned recall | 6/6 (100%) |
| Supported paraphrase recall | 10/10 (100%) |
| Negative specificity | 207/207 (100%) |
| False proceed / ask / block | 0 / 0 / 0 |
| Outcome / choice / rule / metadata mismatches | 0 / 0 / 0 / 0 |
| Explicit ambiguity detection | Passed |

The suite includes option membership and order, scope containment, correction
priority, confidence calibration, hard-no/secret/irreversible safety cases, and
the unavailable-choice crowding regression.

## Performance result

Both benchmarks use 10 warm-up calls and 200 timed optimized gate calls.

| Executable rules | Rebuild | Gate p50 | Gate p95 | Gate max | Budget |
| ---: | ---: | ---: | ---: | ---: | --- |
| 1,000 | 133 ms | 2.548 ms | 2.672 ms | 3.432 ms | p95 < 10 ms |
| 5,000 | 503 ms | 9.831 ms | 10.810 ms | 13.323 ms | p95 < 25 ms; rebuild < 1 s |

[`bench-1000.json`](bench-1000.json) and
[`bench-5000.json`](bench-5000.json) also prove the hot path reports no network,
LLM, AgentMemory, embedding generation, or runtime model load. Retrieval is
bounded to 16 query terms, 32 request options, 5,000 executable rules, 5,000
posting rows per term, 33 exact candidates, and 40 fuzzy candidates. Both runs
passed the ambiguity probe and recovered the expected relevant unavailable
choice after the crowding fixture.

## Recovery result

The `restore-fault-*-state.json` and matching `restore-fault-*.json` files cover
all eight injected transitions:

1. verified
2. staging created
3. files written
4. index rebuilt
5. links checked
6. gate checked
7. existing vault backed up
8. staging activated

The first seven leave the complete old vault active; failure after activation
leaves the complete new vault active. Every surviving vault passed its gate
probe. The `source-*` and `restored-*` gate files prove substantive equivalence
for learned, corrected, and policy-driven behavior.

## Other verification at the candidate

The following commands were run at the exact source commit before retaining
this evidence:

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Passed |
| `cargo test --workspace --all-targets --all-features --locked` | 152 passed across four test binaries |
| `cargo build --release --locked --bin brainmap --bin brainmapd` | Passed |
| `cargo audit` | No vulnerabilities; three allowed unmaintained warnings |
| `cargo deny check` | Passed; metadata-only duplicate/no-license warnings |
| `cargo tree -i crossbeam-epoch` | Resolved patched `0.9.20` |
| `scripts/generate-sbom.sh` | Passed; tracked SBOM remained byte-identical |
| `cargo package --locked -p brainmap-cli` | Passed package and verification build |
| `scripts/prepare-npm-package.sh` | Passed |
| `npm test --prefix npm/brainmap` | Passed |
| `npm pack --dry-run ./npm/brainmap` | Passed; four-file package |
| `bash -n`, `shellcheck`, and `actionlint` on release scripts/workflows | Passed |

Focused reruns also passed: 7 harness tests, 7 MCP tests, 9 installer tests,
the Codex integration-doctor smoke, the dynamic skill smoke, 4 onboarding tests,
21 learning tests, and the structured-policy causal/retirement drill.

Optimized-binary policy, onboarding/correction, stdio, installer, doctor, and
hook drills are retained under [`preflight`](preflight/README.md). They provide
direct synthetic M4–M6 evidence while keeping the real Codex-host FIA-5 open.

Complete path-sanitized command output, per-command exit status, timestamps, and
checksums are retained in
[`verification/verification-manifest.json`](verification/verification-manifest.json).
Its 20/20 gates passed at the same clean source commit. The accompanying
`verification/*.log` files contain the complete output; `SHA256SUMS` binds the
bundle. A second qualification run retained inside that directory also stayed
within every budget (1k p95 3.807 ms; 5k p95 10.484 ms; 5k rebuild 479 ms) and
reproduced both optimized binary hashes. The primary table above remains bound
to the top-level benchmark JSON so a single run is used consistently in release
documentation.

## Qualification boundary

This evidence supports targeted M1–M7 acceptance on the named Linux x86_64
reference host. It does **not** satisfy M8 by itself. Still required before the
whole M001 goal can be called complete:

- FIA-1 through FIA-8 as integrated clean-machine drills against the final
  optimized candidate, including FIA-5 through a real Codex host;
- a two-source-root byte-for-byte release-build check after applying deterministic
  source-path remapping (the `d93ea7a` investigation found a 32-byte CLI build
  variation in upstream dependency formatting code);
- a valid 7–14 day local shadow-mode dogfood interval;
- the final post-dogfood release gate and clean-worktree proof.
