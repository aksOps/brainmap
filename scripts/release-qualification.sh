#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
brainmap="${BRAINMAP_BIN:-${root}/target/release/brainmap}"
brainmapd="${BRAINMAPD_BIN:-${root}/target/release/brainmapd}"
suite="${BRAINMAP_EVAL_SUITE:-${root}/fixtures/decision-bench}"

if [[ "${brainmap}" != /* ]]; then
  brainmap="${root}/${brainmap}"
fi
if [[ "${brainmapd}" != /* ]]; then
  brainmapd="${root}/${brainmapd}"
fi
for executable in "${brainmap}" "${brainmapd}"; do
  if [[ ! -x "${executable}" ]]; then
    echo "missing release executable: ${executable}" >&2
    exit 1
  fi
done
for command in jq sha256sum; do
  if ! command -v "${command}" >/dev/null; then
    echo "release qualification requires ${command}" >&2
    exit 1
  fi
done

temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT
evidence="${BRAINMAP_QUALIFICATION_OUT:-${temporary}/evidence}"
if [[ "${evidence}" != /* ]]; then
  evidence="${root}/${evidence}"
fi
mkdir -p "${evidence}"
qualification_started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
source_commit="$(git -C "${root}" rev-parse HEAD)"
if [[ -n "$(git -C "${root}" status --porcelain --untracked-files=all)" ]]; then
  source_tree_dirty=true
else
  source_tree_dirty=false
fi

canonical_tree_hash() {
  local vault_root="$1"
  (
    cd "${vault_root}"
    find . -type f \
      ! -path './.brainmap/brainmap.sqlite' \
      ! -path './.brainmap/index-manifest.json' \
      ! -path './.brainmap/locks/*' \
      -print0 |
      sort -z |
      while IFS= read -r -d '' path; do
        printf '%s\0' "${path}"
        sha256sum "${path}"
      done
  ) | sha256sum | cut -d ' ' -f 1
}

"${brainmap}" --help >/dev/null
"${brainmapd}" --help >/dev/null

eval_vault="${temporary}/EvalVault"
"${brainmap}" init-vault --vault "${eval_vault}" --yes >/dev/null
"${brainmap}" index rebuild --vault "${eval_vault}" >/dev/null
"${brainmap}" eval --vault "${eval_vault}" --suite "${suite}" >"${evidence}/eval.json"
jq -e '
  .cases >= 100
  and .falseProceed == 0
  and .falseAsk == 0
  and .falseBlock == 0
  and .wrongChoice == 0
  and .wrongRule == 0
  and .wrongMetadata == 0
  and .learnedRuleRecall.exact == 1
  and .learnedRuleRecall.supportedParaphrase >= 0.95
  and .learnedRuleRecall.negativeExpected >= 100
  and .learnedRuleRecall.negativeSpecificity == 1
' "${evidence}/eval.json" >/dev/null

for scale in 1000 5000; do
  report="${evidence}/bench-${scale}.json"
  "${brainmap}" bench --vault "${temporary}/Bench-${scale}" --scale "${scale}" >"${report}"
  jq --arg vault "<qualification-temp>/Bench-${scale}" '.vault = $vault' \
    "${report}" >"${report}.sanitized"
  mv "${report}.sanitized" "${report}"
  if [[ "${scale}" == "1000" ]]; then
    jq -e '
      .scaleRequested == 1000
      and .executableRules == 1000
      and .gateProbe.outcome == "ask_user"
      and .gateProbe.matchKind == "ambiguous"
      and .gateProbe.candidateCollision == true
      and .unavailableChoiceProbe.outcome == "ask_user"
      and .unavailableChoiceProbe.matchKind == "fuzzy"
      and .unavailableChoiceProbe.candidateCollision == false
      and (.unavailableChoiceProbe.matchedPolicies | any(contains("bench-decision-00000")))
      and .candidateBounds.maximumFuzzyRowsScored == 40
      and .candidateBounds.rowsPerTerm == 5000
      and .candidateBounds.executableRules == 5000
      and .candidateBounds.retrieval == "actual-rule-term-postings"
      and .gateP95Ms < 10
    ' "${report}" >/dev/null
  else
    jq -e '
      .scaleRequested == 5000
      and .executableRules == 5000
      and .gateProbe.outcome == "ask_user"
      and .gateProbe.matchKind == "ambiguous"
      and .gateProbe.candidateCollision == true
      and .unavailableChoiceProbe.outcome == "ask_user"
      and .unavailableChoiceProbe.matchKind == "fuzzy"
      and .unavailableChoiceProbe.candidateCollision == false
      and (.unavailableChoiceProbe.matchedPolicies | any(contains("bench-decision-00000")))
      and .candidateBounds.maximumFuzzyRowsScored == 40
      and .candidateBounds.rowsPerTerm == 5000
      and .candidateBounds.executableRules == 5000
      and .candidateBounds.retrieval == "actual-rule-term-postings"
      and .gateP95Ms < 25
      and .indexRebuildMs < 1000
    ' "${report}" >/dev/null
  fi
done

source_vault="${temporary}/SourceVault"
restored_vault="${temporary}/RestoredVault"
archive="${temporary}/release.brainmap.tar.zst"
"${brainmap}" init-vault --vault "${source_vault}" --yes >/dev/null
"${brainmap}" index rebuild --vault "${source_vault}" >/dev/null
"${brainmap}" learn-decision \
  --situation "Choose test runner for release qualification" \
  --options "cargo test|cargo nextest" \
  --chosen "cargo nextest" \
  --rejected "cargo test" \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${source_vault}" >/dev/null
"${brainmap}" learn-decision \
  --situation "Choose formatter for release qualification" \
  --options "biome|prettier" \
  --chosen "biome" \
  --rejected "prettier" \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${source_vault}" >/dev/null
"${brainmap}" learn-decision \
  --situation "Choose formatter for restore fault qualification" \
  --options "biome|prettier" \
  --chosen "biome" \
  --rejected "prettier" \
  --decision-type tooling \
  --scope project:restore-fault-qualification \
  --vault "${source_vault}" >/dev/null
"${brainmap}" apply --pending --yes --vault "${source_vault}" >/dev/null

"${brainmap}" gate --json \
  --situation "Choose test runner for release qualification" \
  --options "cargo test|cargo nextest" \
  --risk low \
  --reversible true \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${source_vault}" >"${temporary}/learned-before-correction.json"
jq -e '.outcome == "proceed" and .selectedOption == "cargo nextest"' \
  "${temporary}/learned-before-correction.json" >/dev/null
decision_id="$(jq -r '.decisionId' "${temporary}/learned-before-correction.json")"
"${brainmap}" learn-feedback \
  --decision-id "${decision_id}" \
  --chosen "cargo test" \
  --rejected "cargo nextest" \
  --vault "${source_vault}" >/dev/null
"${brainmap}" apply --pending --yes --vault "${source_vault}" >/dev/null

"${brainmap}" gate --json --dry-run \
  --situation "Choose test runner for release qualification" \
  --options "cargo test|cargo nextest" \
  --risk low \
  --reversible true \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${source_vault}" >"${evidence}/source-corrected-gate.json"
jq -e '.outcome == "proceed" and .selectedOption == "cargo test"' \
  "${evidence}/source-corrected-gate.json" >/dev/null
"${brainmap}" gate --json --dry-run \
  --situation "Choose formatter for release qualification" \
  --options "biome|prettier" \
  --risk low \
  --reversible true \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${source_vault}" >"${evidence}/source-learned-gate.json"
jq -e '.outcome == "proceed" and .selectedOption == "biome"' \
  "${evidence}/source-learned-gate.json" >/dev/null
"${brainmap}" gate --json --dry-run \
  --situation "Choose v1 storage" \
  --options "Markdown+JSONL|SQLite|External Vector DB" \
  --risk low \
  --reversible true \
  --decision-type architecture \
  --scope global \
  --vault "${source_vault}" >"${evidence}/source-policy-gate.json"
jq -e '.outcome == "proceed" and .selectedOption == "Markdown+JSONL"' \
  "${evidence}/source-policy-gate.json" >/dev/null

"${brainmap}" export --mode portable --vault "${source_vault}" --out "${archive}" >/dev/null
"${brainmap}" verify-export "${archive}" >/dev/null
"${brainmap}" import --file "${archive}" --to "${temporary}/DryRunImport" --dry-run >/dev/null
(
  cd "${temporary}"
  sha256sum "$(basename "${archive}")" >archive.sha256
  sha256sum -c archive.sha256 >/dev/null
)
"${brainmap}" restore --file "${archive}" --to "${restored_vault}" >/dev/null
"${brainmap}" index verify --vault "${restored_vault}" >/dev/null
"${brainmap}" link-check --vault "${restored_vault}" >/dev/null

"${brainmap}" gate --json --dry-run \
  --situation "Choose test runner for release qualification" \
  --options "cargo test|cargo nextest" \
  --risk low \
  --reversible true \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${restored_vault}" >"${evidence}/restored-corrected-gate.json"
"${brainmap}" gate --json --dry-run \
  --situation "Choose formatter for release qualification" \
  --options "biome|prettier" \
  --risk low \
  --reversible true \
  --decision-type tooling \
  --scope project:release-qualification \
  --vault "${restored_vault}" >"${evidence}/restored-learned-gate.json"
"${brainmap}" gate --json --dry-run \
  --situation "Choose v1 storage" \
  --options "Markdown+JSONL|SQLite|External Vector DB" \
  --risk low \
  --reversible true \
  --decision-type architecture \
  --scope global \
  --vault "${restored_vault}" >"${evidence}/restored-policy-gate.json"

for behavior in learned corrected policy; do
  jq -S '{outcome, selectedOption, ruleId, ruleScope, matchKind, appliedPolicies, restrictionsApplied}' \
    "${evidence}/source-${behavior}-gate.json" >"${temporary}/source-${behavior}.normalized.json"
  jq -S '{outcome, selectedOption, ruleId, ruleScope, matchKind, appliedPolicies, restrictionsApplied}' \
    "${evidence}/restored-${behavior}-gate.json" >"${temporary}/restored-${behavior}.normalized.json"
  cmp "${temporary}/source-${behavior}.normalized.json" \
    "${temporary}/restored-${behavior}.normalized.json"
done

old_vault="${temporary}/OldFaultVault"
old_archive="${temporary}/old-fault.brainmap.tar.zst"
"${brainmap}" init-vault --vault "${old_vault}" --yes >/dev/null
"${brainmap}" learn-decision \
  --situation "Choose formatter for restore fault qualification" \
  --options "biome|prettier" \
  --chosen "prettier" \
  --rejected "biome" \
  --decision-type tooling \
  --scope project:restore-fault-qualification \
  --vault "${old_vault}" >/dev/null
"${brainmap}" apply --pending --yes --vault "${old_vault}" >/dev/null
"${brainmap}" export --mode portable --vault "${old_vault}" --out "${old_archive}" >/dev/null
"${brainmap}" verify-export "${old_archive}" >/dev/null

old_complete="${temporary}/OldCompleteBaseline"
new_complete="${temporary}/NewCompleteBaseline"
"${brainmap}" restore --file "${old_archive}" --to "${old_complete}" >/dev/null
"${brainmap}" restore --file "${archive}" --to "${new_complete}" >/dev/null
old_complete_hash="$(canonical_tree_hash "${old_complete}")"
new_complete_hash="$(canonical_tree_hash "${new_complete}")"

for phase in \
  verified \
  staging-created \
  files-written \
  index-rebuilt \
  links-checked \
  gate-checked \
  existing-backed-up \
  staging-activated; do
  fault_target="${temporary}/FaultTarget-${phase}"
  "${brainmap}" restore --file "${old_archive}" --to "${fault_target}" >/dev/null
  if "${brainmap}" restore \
    --file "${archive}" \
    --to "${fault_target}" \
    --fault-phase "${phase}" >/dev/null 2>&1; then
    echo "restore fault phase unexpectedly succeeded: ${phase}" >&2
    exit 1
  fi
  "${brainmap}" index verify --vault "${fault_target}" >/dev/null
  "${brainmap}" link-check --vault "${fault_target}" >/dev/null
  fault_hash="$(canonical_tree_hash "${fault_target}")"
  if [[ "${fault_hash}" = "${old_complete_hash}" ]]; then
    complete_state=old
  elif [[ "${fault_hash}" = "${new_complete_hash}" ]]; then
    complete_state=new
  else
    echo "restore fault phase left a noncanonical tree: ${phase}" >&2
    exit 1
  fi
  jq -n \
    --arg phase "${phase}" \
    --arg state "${complete_state}" \
    --arg treeHash "${fault_hash}" \
    --arg oldTreeHash "${old_complete_hash}" \
    --arg newTreeHash "${new_complete_hash}" \
    '{phase: $phase, completeState: $state, treeHash: $treeHash, oldTreeHash: $oldTreeHash, newTreeHash: $newTreeHash}' \
    >"${evidence}/restore-fault-${phase}-state.json"
  "${brainmap}" gate --json --dry-run \
    --situation "Choose formatter for restore fault qualification" \
    --options "biome|prettier" \
    --risk low \
    --reversible true \
    --decision-type tooling \
    --scope project:restore-fault-qualification \
    --vault "${fault_target}" >"${evidence}/restore-fault-${phase}.json"
  jq -e '
    .outcome == "proceed"
    and (.selectedOption == "biome" or .selectedOption == "prettier")
  ' "${evidence}/restore-fault-${phase}.json" >/dev/null
done

brainmap_sha256="$(sha256sum "${brainmap}" | cut -d ' ' -f 1)"
brainmapd_sha256="$(sha256sum "${brainmapd}" | cut -d ' ' -f 1)"
archive_sha256="$(sha256sum "${archive}" | cut -d ' ' -f 1)"
jq -n \
  --arg schemaVersion "brainmap-release-qualification-v1" \
  --arg sourceCommit "${source_commit}" \
  --argjson sourceTreeDirty "${source_tree_dirty}" \
  --arg startedAt "${qualification_started_at}" \
  --arg completedAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg host "$(uname -srm)" \
  --arg rustc "$(rustc --version)" \
  --arg cargo "$(cargo --version)" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg archiveSha256 "${archive_sha256}" \
  '{
    schemaVersion: $schemaVersion,
    sourceCommit: $sourceCommit,
    sourceTreeDirty: $sourceTreeDirty,
    startedAt: $startedAt,
    completedAt: $completedAt,
    host: $host,
    toolchain: {rustc: $rustc, cargo: $cargo},
    binaries: {brainmapSha256: $brainmapSha256, brainmapdSha256: $brainmapdSha256},
    portableArchiveSha256: $archiveSha256
  }' >"${evidence}/qualification-manifest.json"

printf 'release qualification passed; evidence: %s\n' "${evidence}"
