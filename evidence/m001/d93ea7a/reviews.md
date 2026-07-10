# Independent review record

Three independent review passes examined the stable M1–M7 candidate and its
readiness documents on 2026-07-10.

## Specification review

No remaining high-confidence M1–M7 functional mismatch was found after the
final fixes. In particular, the reviewer confirmed that:

- unavailable-choice retrieval scores actual rules rather than combining terms
  from unrelated rules;
- candidate, option, query-term, posting, and executable-rule bounds are
  explicit;
- the crowding fixture preserves the relevant ninth unavailable choice;
- `expectedChoice` distinguishes omitted, `null`, and concrete selections;
- M8 real-host and dogfood requirements remain evidence gaps rather than code
  defects.

## Repository-standards review

The review confirmed the deterministic local gate constraints, schema-v4 test
coverage, current-rule posting source, relevance-before-priority ordering,
presence-aware eval expectations, qualification packaging order, and bounded
retrieval. One documentation mismatch concerning the three checksummed release
artifacts was found and fixed in `d93ea7a`.

Non-blocking maintainability observations were retained for later work: SQL and
Rust both encode precedence ordering and therefore need parity coverage, and the
eval mismatch counters could eventually be centralized in a typed collector.

## Documentation review

The production/readiness documentation was corrected to:

- state the single-user local scope and qualified Linux x86_64 reference host;
- use accurate Codex `enforced`, `best-effort`, and `instruction-only` labels;
- distinguish historical reports from current evidence;
- document all release artifacts and checksum entries;
- bind performance claims to the retained `d93ea7a` benchmark JSON;
- keep FIA-5 and the 7–14 day dogfood interval explicitly open.

These reviews support targeted M1–M7 acceptance only. They do not substitute for
the clean-machine FIA sequence, a real Codex-host FIA-5 run, or dogfood.
