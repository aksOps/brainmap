# M001 final pre-dogfood evidence — `c49d41a`

This directory is the exact-clean M1–M7 qualification bundle for the selected
pre-dogfood runtime candidate. It supersedes `d93ea7a` for release, security,
package, recovery, and performance evidence while retaining the earlier direct
M4–M6 preflight transcripts as supporting evidence.

It does **not** complete M8. A real Codex-host FIA-5 run, the clean-machine FIA
sequence, and the 7–14 day shadow-mode dogfood interval remain required.

## Provenance

- Source commit: `c49d41ac5326a71712ab5d5ecd4ec48a402d935e`
- Source tree: clean before and after the captured verification run
- Host: Linux `6.8.0-134-generic`, x86_64
- Toolchain: Rust/Cargo 1.95.0
- `brainmap` SHA-256:
  `cfda4b4eb5ecb4385632b1f3c7e13402b4870399ad1bf58db1632445d4d16563`
- `brainmapd` SHA-256:
  `72119cea3e554648f88dfb51049dda068c6ab144ffc7f46d1772073187f67c05`
- Source snapshot SHA-256:
  `e19133c4aa31b7418a298f41727f32f0cdfed5279dbe28fca798e0fc7c88f76f`

[`manifest.json`](manifest.json) is the compact machine-readable index. The
top-level [`SHA256SUMS`](SHA256SUMS) verifies the 65 generated payload
artifacts; this README is the human index added by the evidence checkpoint.

## Reproducible release artifacts

`scripts/verify-release-reproducibility.sh` built the candidate in two isolated
source roots with compiler wrappers disabled. For both binaries:

- build A equals build B byte-for-byte;
- the verified installed `target/release` file equals build A;
- the npm-packaged file equals the verified installed file.

The complete path-sanitized transcript is in
[`reproducibility.log`](reproducibility.log). The verified source snapshot is
retained as `source.tar`.

The fix uses the MIT-licensed `i18n-embed-fl` 0.9.4 source with the upstream
deterministic named-argument ordering patch. Its focused test graph is locked,
2/2 tests pass, and both root and vendor audit gates report no vulnerabilities.
The existing `proc-macro-error2` unmaintained advisory remains informational and
allowed; upstream 0.10 still uses it.

## Functional qualification

[`qualification/eval.json`](qualification/eval.json) records 238 deterministic
cases:

| Contract | Result |
| --- | ---: |
| Exact learned recall | 6/6 (100%) |
| Supported paraphrase recall | 10/10 (100%) |
| Negative specificity | 207/207 (100%) |
| False proceed / ask / block | 0 / 0 / 0 |
| Choice / rule / metadata mismatches | 0 / 0 / 0 |

## Performance qualification

Each optimized benchmark uses 10 warm-ups and 200 timed gate calls.

| Executable rules | Rebuild | Gate p50 | Gate p95 | Gate max | Budget |
| ---: | ---: | ---: | ---: | ---: | --- |
| 1,000 | 135 ms | 2.558 ms | 2.809 ms | 3.954 ms | p95 < 10 ms |
| 5,000 | 497 ms | 9.892 ms | 10.696 ms | 15.172 ms | p95 < 25 ms; rebuild < 1 s |

Both runs retain the expected ambiguity and unavailable-choice probes. Candidate
retrieval remains bounded to 16 query terms, 32 request options, 5,000 rules,
5,000 postings per term, 33 exact candidates, and 40 fuzzy candidates. All
prohibited gate hot-path operations are false.

## Recovery qualification

- Learned, corrected, and policy-driven source/restored results are
  substantively identical (3/3 pairs).
- Eight restore failure phases leave a canonical complete vault: seven retain
  the complete old state, and failure after staging activation retains the
  complete new state.
- All eight recovered gates are usable and exact.

The raw state and gate files are under `qualification/`; the aggregation is in
`qualification-summary.json` and `manifest.json`.

## Captured gates

All 16 exact-HEAD verification gates exited zero:

- formatting, all-target/all-feature Clippy, and 152/152 workspace tests;
- 2/2 locked vendor tests plus root and vendor audits;
- dependency policy, Cargo package verification, and SBOM byte identity;
- npm preparation, npm tests, and npm pack dry-run;
- Bash syntax, ShellCheck, and Actionlint.

Complete path-sanitized logs and exit records are under `verification/`.

## Remaining M8 work

Before the whole M001 goal can be declared complete:

1. Run FIA-1 through FIA-8 as an integrated clean-machine sequence against the
   final optimized candidate, including FIA-5 in a real Codex host.
2. Complete 7–14 days of real local shadow-mode dogfood with the required
   aggregate metrics and no safety false-proceed or cross-domain application.
3. Add the dogfood report, rerun the final release gate at the final commit, and
   leave the worktree clean.
