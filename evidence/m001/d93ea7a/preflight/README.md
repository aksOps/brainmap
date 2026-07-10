# Optimized policy, learning, and adapter preflight

These synthetic drills were run with a locked optimized build from source commit
`d93ea7ada09043a1422478b60d5f3223037ff8ad`. All paths are replaced with stable
placeholders and all inputs are secret-safe synthetic data.

The build used for these drills has SHA-256
`d22e00e5d4681fbcd68c6ca9717f3e82e4e984c28cf2802685ccce86f3796cb2`.
A second build root produced a CLI binary differing by only 32 bytes (GNU build
ID plus equivalent ordering in upstream `age` formatting code). That does not
invalidate these behavioral results, but it does mean this build is not final
M8 reproducible-artifact evidence. Release builds need the planned source-root
remapping fix and a two-root comparison before dogfood begins.

## M4 policy compiler

The `m4` directory records:

- no applicable policy: `ask_user`;
- active structured policy: `proceed` with `violet`, the expected rule ID, and
  only the causal policy in `appliedPolicies`;
- prose-only edit: identical substantive decision;
- retired policy: back to `ask_user` with no applied policy;
- malformed control policy: compilation fails closed and names the note/field;
- malformed non-control rule: excluded with an actionable warning while rebuild
  succeeds;
- wikilink round trip: note hash unchanged and the link preserved.

[`m4/summary.json`](m4/summary.json) contains the main assertions.

## M5 onboarding and learning

The `m5` directory records:

- clean interactive onboarding asks three questions and applies three exact
  previews;
- versioned answer-file onboarding dry-run leaves the complete tree hash
  unchanged, then explicit approval applies one update;
- a free-text ambiguous answer remains `pending-clarification` and its gate asks;
- onboarding learns scoped `pnpm`, then recorded feedback previews and applies a
  correction to `npm` while an unrelated scope still asks;
- the second packet application applies zero packets;
- preview leaves canonical Markdown/JSONL/packet hashes unchanged (only the
  rebuildable process-lock file is touched);
- secret-like onboarding and feedback are rejected without a pending event or
  packet;
- secret-safe ledger and packet schema samples are retained.

[`m5/behavior-comparison.json`](m5/behavior-comparison.json),
[`m5/onboarding-drill.json`](m5/onboarding-drill.json), and
[`m5/secret-rejection.json`](m5/secret-rejection.json) contain the primary
assertions.

## M6 stdio and Codex preflight

The `m6` directory records:

- a complete structured stdio request/response with exact learned selection and
  causal metadata;
- strict rejection of an unknown stdio field;
- Codex installer dry-run, backed-up installation, idempotent second install,
  protected unmanaged-file refusal, and a healthy integration doctor;
- accurate `instruction-only`, `best-effort`, and `enforced` labels;
- hook preflight that allows routine work, blocks a hard-no action, and keeps an
  unobservable personal prompt advisory rather than falsely enforced.

[`m6/stdio-transcript.json`](m6/stdio-transcript.json),
[`m6/codex-installer-doctor.json`](m6/codex-installer-doctor.json), and
[`m6/codex-hook-preflight.json`](m6/codex-hook-preflight.json) contain the main
assertions. This is adapter preflight, not a live Codex-host FIA-5 transcript.
