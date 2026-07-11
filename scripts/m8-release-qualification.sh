#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

usage() {
  cat <<'EOF'
Run the independent Brainmap FIA-8 release gate and publish strict evidence.

Usage:
  scripts/m8-release-qualification.sh \
    --brainmap PATH --brainmap-sha256 SHA256 \
    --brainmapd PATH --brainmapd-sha256 SHA256 \
    --candidate-commit COMMIT \
    --reproducibility-manifest PATH \
    --out DIR

Required:
  --brainmap PATH                 Exact absolute optimized brainmap path
  --brainmap-sha256 SHA256        Expected SHA-256 of brainmap
  --brainmapd PATH                Exact absolute optimized brainmapd path
  --brainmapd-sha256 SHA256       Expected SHA-256 of brainmapd
  --candidate-commit COMMIT       Full 40-character commit at unchanged HEAD
  --reproducibility-manifest PATH Strict two-root release provenance manifest
  --out DIR                       Absolute path to a new evidence directory

The runner has no test or fake mode. It requires a clean worktree, reruns the
complete FIA-8 gate, rejects any source or binary identity change, sanitizes
retained text, and publishes the evidence directory atomically.
EOF
}

die() {
  echo "m8 release qualification: $*" >&2
  exit 1
}

require_value() {
  [[ -n "${2:-}" ]] || die "$1 requires a value"
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

sha256_file() {
  sha256sum "$1" | awk '{print $1}'
}

canonical_file_path() {
  local path="$1"
  local parent
  parent="$(cd "$(dirname "${path}")" && pwd -P)"
  printf '%s/%s\n' "${parent}" "$(basename "${path}")"
}

sanitize_one_file() {
  local file="$1"
  shift
  local value replacement temporary_file
  while (($#)); do
    value="$1"
    replacement="$2"
    shift 2
    [[ -n "${value}" ]] || continue
    temporary_file="${file}.sanitized"
    awk -v needle="${value}" -v replacement="${replacement}" '
      function replace_literal(text, target, value, position) {
        if (target == "") return text
        while ((position = index(text, target)) != 0) {
          text = substr(text, 1, position - 1) value substr(text, position + length(target))
        }
        return text
      }
      { print replace_literal($0, needle, replacement) }
    ' "${file}" >"${temporary_file}"
    mv "${temporary_file}" "${file}"
  done
}

contains_secret_like_material() {
  local file="$1"

  # Keep this fail-closed detector aligned with privacy::contains_secret in
  # the strict Rust qualification verifier. The AWS form is an additional
  # conservative release-evidence check.
  if grep -Eiq \
    "(^|[^[:alnum:]_])(api[_-]?key|token|secret|password)[[:space:]]*[:=][[:space:]]*[\"']?[A-Za-z0-9_.+/=-]{12,}|(^|[^[:alnum:]_])Bearer[[:space:]]+[A-Za-z0-9_.+/=-]{12,}|-----BEGIN [A-Z ]*PRIVATE KEY-----|(^|[^[:alnum:]_])(cookie|authorization)[[:space:]]*:[[:space:]]*[^[:space:]].{7,}|(^|[^[:alnum:]_])sk-[A-Za-z0-9_-]{16,}|AKIA[0-9A-Z]{16}" \
    "${file}"; then
    return 0
  fi

  if grep -Eiq \
    '(^|[^[:alnum:]._%+-])[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}([^[:alnum:]._-]|$)' \
    "${file}"; then
    return 0
  fi

  return 1
}

brainmap=
brainmap_sha256=
brainmapd=
brainmapd_sha256=
candidate_commit=
reproducibility_manifest=
out=

while (($#)); do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --brainmap|--brainmap-sha256|--brainmapd|--brainmapd-sha256|--candidate-commit|--reproducibility-manifest|--out)
      option="$1"
      require_value "${option}" "${2:-}"
      case "${option}" in
        --brainmap) brainmap="$2" ;;
        --brainmap-sha256) brainmap_sha256="$2" ;;
        --brainmapd) brainmapd="$2" ;;
        --brainmapd-sha256) brainmapd_sha256="$2" ;;
        --candidate-commit) candidate_commit="$2" ;;
        --reproducibility-manifest) reproducibility_manifest="$2" ;;
        --out) out="$2" ;;
      esac
      shift 2
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "${brainmap}" ]] || die "missing required --brainmap PATH"
[[ -n "${brainmap_sha256}" ]] || die "missing required --brainmap-sha256 SHA256"
[[ -n "${brainmapd}" ]] || die "missing required --brainmapd PATH"
[[ -n "${brainmapd_sha256}" ]] || die "missing required --brainmapd-sha256 SHA256"
[[ -n "${candidate_commit}" ]] || die "missing required --candidate-commit COMMIT"
[[ -n "${reproducibility_manifest}" ]] ||
  die "missing required --reproducibility-manifest PATH"
[[ -n "${out}" ]] || die "missing required --out DIR"

[[ "${brainmap}" == /* && "${brainmapd}" == /* ]] ||
  die "binary paths must be absolute"
[[ -f "${brainmap}" && -x "${brainmap}" ]] ||
  die "brainmap is not an executable regular file: ${brainmap}"
[[ -f "${brainmapd}" && -x "${brainmapd}" ]] ||
  die "brainmapd is not an executable regular file: ${brainmapd}"
[[ "${brainmap}" == "$(canonical_file_path "${brainmap}")" &&
   "${brainmapd}" == "$(canonical_file_path "${brainmapd}")" ]] ||
  die "binary paths must be canonical absolute paths"

[[ "${brainmap_sha256}" =~ ^[0-9a-f]{64}$ ]] ||
  die "brainmap SHA-256 must be exactly 64 lowercase hexadecimal characters"
[[ "${brainmapd_sha256}" =~ ^[0-9a-f]{64}$ ]] ||
  die "brainmapd SHA-256 must be exactly 64 lowercase hexadecimal characters"
actual_brainmap_sha256="$(sha256_file "${brainmap}")"
actual_brainmapd_sha256="$(sha256_file "${brainmapd}")"
[[ "${actual_brainmap_sha256}" == "${brainmap_sha256}" ]] ||
  die "brainmap SHA-256 mismatch: expected ${brainmap_sha256}, got ${actual_brainmap_sha256}"
[[ "${actual_brainmapd_sha256}" == "${brainmapd_sha256}" ]] ||
  die "brainmapd SHA-256 mismatch: expected ${brainmapd_sha256}, got ${actual_brainmapd_sha256}"

[[ "${candidate_commit}" =~ ^[0-9a-f]{40}$ ]] ||
  die "candidate commit must be exactly 40 lowercase hexadecimal characters"
git -C "${root}" cat-file -e "${candidate_commit}^{commit}" 2>/dev/null ||
  die "candidate commit does not resolve in this repository"
head_commit="$(git -C "${root}" rev-parse HEAD)"
[[ "${candidate_commit}" == "${head_commit}" ]] ||
  die "candidate commit must equal unchanged HEAD"
[[ -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
  die "qualification requires clean HEAD"

[[ "${reproducibility_manifest}" == /* ]] ||
  die "reproducibility manifest path must be absolute"
[[ -f "${reproducibility_manifest}" ]] ||
  die "reproducibility manifest is not a regular file: ${reproducibility_manifest}"
[[ "${reproducibility_manifest}" == "$(canonical_file_path "${reproducibility_manifest}")" ]] ||
  die "reproducibility manifest path must be canonical"
brainmap_build_info="$("${brainmap}" build-info)" ||
  die "brainmap did not expose embedded build provenance"
brainmapd_build_info="$("${brainmapd}" build-info)" ||
  die "brainmapd did not expose embedded build provenance"
[[ "${brainmap_build_info}" == "${brainmapd_build_info}" ]] ||
  die "brainmap and brainmapd embedded build provenance differs"
jq -e --arg candidateCommit "${candidate_commit}" '
  type == "object"
  and (keys == [
    "candidateCommit", "cargoProfile", "producerDigests", "qualification",
    "schemaVersion"
  ])
  and .schemaVersion == "brainmap-build-info-v1"
  and .candidateCommit == $candidateCommit
  and .cargoProfile == "release"
  and .qualification == {
    eligible: true,
    locked: true,
    marker: "brainmap-clean-locked-two-root-v1",
    release: true,
    twoRootCandidate: true
  }
  and (.producerDigests | keys == [
    "assembleQualificationSha256", "codexFia5Sha256",
    "integratedQualificationSha256", "releaseQualificationSha256"
  ])
  and all(.producerDigests[]; test("^[0-9a-f]{64}$"))
' <<<"${brainmap_build_info}" >/dev/null ||
  die "candidate binaries do not contain qualifying embedded build provenance"
build_info_sha256="$(printf '%s' "${brainmap_build_info}" | sha256sum | cut -d ' ' -f 1)"
producer_digests="$(jq -c '.producerDigests' <<<"${brainmap_build_info}")"
release_producer_sha256="$(sha256_file "${root}/scripts/m8-release-qualification.sh")"
jq -e \
  --arg candidateCommit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg buildInfoSha256 "${build_info_sha256}" \
  --arg releaseProducerSha256 "${release_producer_sha256}" \
  --argjson producerDigests "${producer_digests}" '
  type == "object"
  and (keys == [
    "brainmapSha256", "brainmapdSha256", "buildInfoSha256",
    "candidateCommit", "cleanTree", "locked", "producerDigests", "profile",
    "schemaVersion", "twoRootByteIdentical"
  ])
  and .schemaVersion == "brainmap-release-reproducibility-v2"
  and .candidateCommit == $candidateCommit
  and .profile == "release"
  and .locked == true
  and .twoRootByteIdentical == true
  and .cleanTree == true
  and .brainmapSha256 == $brainmapSha256
  and .brainmapdSha256 == $brainmapdSha256
  and .buildInfoSha256 == $buildInfoSha256
  and .producerDigests == $producerDigests
  and .producerDigests.releaseQualificationSha256 == $releaseProducerSha256
' "${reproducibility_manifest}" >/dev/null ||
  die "invalid strict reproducibility manifest"
reproducibility_manifest_sha256="$(sha256_file "${reproducibility_manifest}")"

[[ "${out}" == /* ]] || die "evidence output path must be absolute"
[[ ! -e "${out}" && ! -L "${out}" ]] ||
  die "evidence directory already exists: ${out}"
out_parent="$(dirname "${out}")"
[[ -d "${out_parent}" ]] || die "evidence output parent does not exist: ${out_parent}"
out_parent="$(cd "${out_parent}" && pwd -P)"
canonical_out="${out_parent}/$(basename "${out}")"
[[ "${out}" == "${canonical_out}" ]] ||
  die "evidence output path must be canonical"
[[ "${out}" != "${root}" && "${out}" != "${root}/"* ]] ||
  die "evidence output path must be outside the repository"

for command in \
  actionlint awk bash cargo cmp cp date find git grep iconv jq mktemp mv node npm \
  rustc sed sha256sum shellcheck sort sync uname xargs; do
  require_command "${command}"
done

tracked_sbom="${root}/crates/brainmap-cli/brainmap.json"
[[ -f "${tracked_sbom}" ]] || die "tracked release SBOM is missing"

staging="$(mktemp -d "${out_parent}/.brainmap-m8-release.XXXXXX")"
raw="${staging}/release"
work="${staging}/work"
cleanup() {
  rm -rf "${staging}"
}
trap cleanup EXIT

mkdir -p \
  "${raw}/gates" \
  "${raw}/qualification" \
  "${raw}/sbom" \
  "${work}"
cp "${tracked_sbom}" "${work}/tracked-sbom.json"
cp "${reproducibility_manifest}" "${raw}/reproducibility-manifest.json"

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
commands_tsv="${work}/commands.tsv"
sequence=0
: >"${commands_tsv}"

verify_input_hashes() {
  local current_brainmap current_brainmapd
  current_brainmap="$(sha256_file "${brainmap}")"
  current_brainmapd="$(sha256_file "${brainmapd}")"
  [[ "${current_brainmap}" == "${brainmap_sha256}" ]] ||
    die "brainmap changed during qualification"
  [[ "${current_brainmapd}" == "${brainmapd_sha256}" ]] ||
    die "brainmapd changed during qualification"
}

assert_candidate_state() {
  [[ "$(git -C "${root}" rev-parse HEAD)" == "${candidate_commit}" ]] ||
    die "candidate commit must equal unchanged HEAD"
  [[ -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
    die "qualification requires clean HEAD"
}

run_gate() {
  local id="$1" display="$2"
  shift 2
  [[ "${id}" =~ ^[a-z0-9-]+$ ]] || die "invalid gate ID: ${id}"
  if awk -F '\t' -v id="${id}" '$2 == id {found = 1} END {exit !found}' \
    "${commands_tsv}"; then
    die "duplicate gate ID: ${id}"
  fi

  sequence=$((sequence + 1))
  local log_relative log_path exit_code passed
  log_relative="gates/${id}.log"
  log_path="${raw}/${log_relative}"
  set +e
  (
    set -euo pipefail
    cd "${root}"
    "$@"
  ) >"${log_path}" 2>&1
  exit_code=$?
  set -e
  passed=false
  [[ "${exit_code}" -eq 0 ]] && passed=true
  printf '%s\t%s\t%s\t0\t%s\t%s\t%s\n' \
    "${sequence}" "${id}" "${display}" "${exit_code}" "${passed}" "${log_relative}" \
    >>"${commands_tsv}"
  [[ "${passed}" == true ]] || die "gate ${id} failed with exit ${exit_code}"
  verify_input_hashes
  assert_candidate_state
}

verify_release_binary_identity() {
  local built_brainmap="${root}/target/release/brainmap"
  local built_brainmapd="${root}/target/release/brainmapd"
  [[ -x "${built_brainmap}" && -x "${built_brainmapd}" ]]
  [[ "$(sha256_file "${built_brainmap}")" == "${brainmap_sha256}" ]]
  [[ "$(sha256_file "${built_brainmapd}")" == "${brainmapd_sha256}" ]]
  cmp --silent "${built_brainmap}" "${brainmap}"
  cmp --silent "${built_brainmapd}" "${brainmapd}"
  echo "locked release binaries match exact supplied identities"
}

verify_npm_binary_identity() {
  local npm_brainmap="${root}/npm/brainmap/bin/brainmap"
  local npm_brainmapd="${root}/npm/brainmap/bin/brainmapd"
  [[ -x "${npm_brainmap}" && -x "${npm_brainmapd}" ]]
  [[ "$(sha256_file "${npm_brainmap}")" == "${brainmap_sha256}" ]]
  [[ "$(sha256_file "${npm_brainmapd}")" == "${brainmapd_sha256}" ]]
  cmp --silent "${npm_brainmap}" "${brainmap}"
  cmp --silent "${npm_brainmapd}" "${brainmapd}"
  echo "npm package binaries match exact supplied identities"
}

run_workspace_tests() {
  cargo test --locked --workspace --all-targets --all-features
  scripts/test-vendored-i18n.sh

  local -a shell_scripts workflows
  mapfile -d '' shell_scripts < <(
    find scripts -maxdepth 1 -type f -name '*.sh' -print0 | sort -z
  )
  ((${#shell_scripts[@]} > 0))
  bash -n "${shell_scripts[@]}"
  shellcheck "${shell_scripts[@]}"

  mapfile -d '' workflows < <(
    find .github/workflows -maxdepth 1 -type f \
      \( -name '*.yml' -o -name '*.yaml' \) -print0 | sort -z
  )
  ((${#workflows[@]} > 0))
  actionlint "${workflows[@]}"
  scripts/test-m8-integrated-qualification.sh
  scripts/test-m8-codex-fia5.sh
  scripts/test-m8-assemble-qualification.sh
  scripts/test-m8-release-qualification.sh
  scripts/test-release-reproducibility.sh
}

run_audits() {
  cargo audit
  cargo audit --file vendor/i18n-embed-fl/Cargo.lock
}

run_sbom_gate() {
  scripts/generate-sbom.sh
  cmp --silent "${work}/tracked-sbom.json" "${tracked_sbom}"
  jq -e '
    .bomFormat == "CycloneDX"
    and (has("authors") or has("metadata"))
  ' "${tracked_sbom}" >/dev/null
  if grep -E 'path\+file://|file://[^.#]|/(home|opt)/[^" ]+' "${tracked_sbom}"; then
    echo "generated SBOM contains a local filesystem path" >&2
    return 1
  fi
  cp "${tracked_sbom}" "${raw}/sbom/brainmap.cdx.json"
  echo "generated SBOM is byte-identical to the tracked sanitized SBOM"
}

run_locked_release_build() {
  env \
    RUSTC_WRAPPER= \
    RUSTC_WORKSPACE_WRAPPER= \
    CARGO_BUILD_RUSTC_WRAPPER= \
    CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER= \
    BRAINMAP_INTERNAL_QUALIFICATION_MARKER=brainmap-clean-locked-two-root-v1 \
    BRAINMAP_INTERNAL_CANDIDATE_COMMIT="${candidate_commit}" \
    BRAINMAP_INTERNAL_SOURCE_CLEAN=true \
    BRAINMAP_INTERNAL_LOCKED=true \
    BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE=true \
    cargo build --release --locked -p brainmap-cli --bin brainmap --bin brainmapd
  verify_release_binary_identity
}

run_package_smoke() {
  cargo package --locked -p brainmap-cli
  scripts/prepare-npm-package.sh
  npm test --prefix npm/brainmap
  npm pack --dry-run ./npm/brainmap
  verify_npm_binary_identity
}

run_release_qualification() {
  env \
    BRAINMAP_BIN="${brainmap}" \
    BRAINMAPD_BIN="${brainmapd}" \
    BRAINMAP_QUALIFICATION_OUT="${raw}/qualification" \
    BRAINMAP_CANDIDATE_COMMIT="${candidate_commit}" \
    BRAINMAP_EXPECTED_SHA256="${brainmap_sha256}" \
    BRAINMAPD_EXPECTED_SHA256="${brainmapd_sha256}" \
    scripts/release-qualification.sh

  jq -e \
    --arg commit "${candidate_commit}" \
    --arg brainmapSha256 "${brainmap_sha256}" \
    --arg brainmapdSha256 "${brainmapd_sha256}" '
    .schemaVersion == "brainmap-release-qualification-v1"
    and .sourceCommit == $commit
    and .sourceTreeDirty == false
    and .binaries.brainmapSha256 == $brainmapSha256
    and .binaries.brainmapdSha256 == $brainmapdSha256
  ' "${raw}/qualification/qualification-manifest.json" >/dev/null
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
  ' "${raw}/qualification/eval.json" >/dev/null
  jq -e '
    .scaleRequested == 1000
    and .executableRules == 1000
    and .candidateBounds.retrieval == "actual-rule-term-postings"
    and .gateP95Ms < 10
  ' "${raw}/qualification/bench-1000.json" >/dev/null
  echo "1k scale qualification and evaluation thresholds passed"
}

verify_scale_5000() {
  jq -e '
    .scaleRequested == 5000
    and .executableRules == 5000
    and .candidateBounds.retrieval == "actual-rule-term-postings"
    and .gateP95Ms < 25
    and .indexRebuildMs < 1000
  ' "${raw}/qualification/bench-5000.json" >/dev/null
  echo "5k scale and rebuild thresholds passed"
}

verify_performance_and_recovery() {
  local expected_phases actual_phases state_count
  expected_phases="$(printf '%s\n' \
    existing-backed-up files-written gate-checked index-rebuilt links-checked \
    staging-activated staging-created verified | sort)"
  actual_phases="$(
    jq -r '.phase' "${raw}"/qualification/restore-fault-*-state.json | sort
  )"
  state_count="$(find "${raw}/qualification" -maxdepth 1 -type f \
    -name 'restore-fault-*-state.json' | wc -l | awk '{print $1}')"
  [[ "${state_count}" -eq 8 ]]
  [[ "${actual_phases}" == "${expected_phases}" ]]
  jq -e '
    (.completeState == "old" or .completeState == "new")
    and .oldTreeHash != .newTreeHash
    and (.treeHash == .oldTreeHash or .treeHash == .newTreeHash)
  ' "${raw}"/qualification/restore-fault-*-state.json >/dev/null
  jq -e '.gateP95Ms < 10' "${raw}/qualification/bench-1000.json" >/dev/null
  jq -e '.gateP95Ms < 25 and .indexRebuildMs < 1000' \
    "${raw}/qualification/bench-5000.json" >/dev/null
  echo "performance budgets and all eight recovery phases passed"
}

verify_clean_final() {
  verify_release_binary_identity
  verify_npm_binary_identity
  cmp --silent "${work}/tracked-sbom.json" "${tracked_sbom}"
  verify_input_hashes
  assert_candidate_state
  echo "candidate HEAD, source tree, binaries, npm payload, and SBOM are unchanged"
}

run_gate format \
  'cargo fmt --all -- --check' \
  cargo fmt --all -- --check
run_gate clippy \
  'cargo clippy --locked --workspace --all-targets --all-features -- -D warnings' \
  cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
run_gate workspace-tests \
  'all-target/all-feature tests; vendored, shell, workflow, and runner interfaces' \
  run_workspace_tests
run_gate audit \
  'root and vendored dependency audits' \
  run_audits
run_gate deny \
  'cargo deny check' \
  cargo deny check
run_gate sbom \
  'regenerate and byte-verify the tracked sanitized CycloneDX SBOM' \
  run_sbom_gate
run_gate locked-release-build \
  'locked optimized build and exact binary identity' \
  run_locked_release_build
run_gate package-smoke \
  'Cargo package plus npm prepare, test, dry-run pack, and binary identity' \
  run_package_smoke
run_gate scale-1000 \
  'release qualification, eval thresholds, and 1k budget' \
  run_release_qualification
run_gate scale-5000 \
  '5k gate and index-rebuild budgets' \
  verify_scale_5000
run_gate performance \
  'independent 1k/5k budget and eight-phase recovery assertions' \
  verify_performance_and_recovery
run_gate clean-worktree \
  'unchanged clean HEAD and exact final artifact identities' \
  verify_clean_final

# Retain the structured qualification outcomes, but remove request-bearing
# fields before the evidence leaves staging. Values nested under these keys are
# synthetic in this runner, yet they are still raw prompt material.
while IFS= read -r -d '' qualification_json; do
  normalized="${qualification_json}.normalized"
  jq '
    walk(
      if type == "object" then
        del(.prompt, .messages, .transcript, .situation, .options, .toolArguments)
      else
        .
      end
    )
  ' "${qualification_json}" >"${normalized}"
  mv "${normalized}" "${qualification_json}"
done < <(find "${raw}/qualification" -type f -name '*.json' -print0)

# Sanitize every retained command log and structured artifact before hashing.
while IFS= read -r -d '' retained; do
  sanitize_one_file "${retained}" \
    "${staging}" '<release-staging>' \
    "${out}" '<release-evidence>' \
    "${root}" '<source-root>' \
    "${brainmap}" '<brainmap>' \
    "${brainmapd}" '<brainmapd>' \
    "${out_parent}" '<evidence-parent>' \
    "${HOME:-}" '<home>' \
    "${TMPDIR:-}" '<tmp>'
done < <(
  find "${raw}" -type f \
    ! -path "${raw}/reproducibility-manifest.json" \
    ! -path "${raw}/sbom/brainmap.cdx.json" \
    -print0
)

# Gate result files use the strict verifier shape and bind each stable command
# ID to its sanitized sibling log.
while IFS=$'\t' read -r command_sequence id display expected exit_code passed log_relative; do
  : "${command_sequence}" "${display}" "${expected}"
  log_sha256="$(sha256_file "${raw}/${log_relative}")"
  jq -n \
    --arg schemaVersion brainmap-m8-release-gate-result-v1 \
    --arg gate "${id}" \
    --arg commandId "${id}" \
    --argjson passed "${passed}" \
    --argjson exitCode "${exit_code}" \
    --arg logSha256 "${log_sha256}" '
    {
      schemaVersion: $schemaVersion,
      gate: $gate,
      commandId: $commandId,
      passed: $passed,
      exitCode: $exitCode,
      logSha256: $logSha256
    }
  ' >"${raw}/gates/${id}.json"
done <"${commands_tsv}"

gate_reference() {
  local gate="$1"
  local path="gates/${gate}.json"
  jq -n --arg path "${path}" --arg sha256 "$(sha256_file "${raw}/${path}")" \
    '{path: $path, sha256: $sha256}'
}

format_ref="$(gate_reference format)"
clippy_ref="$(gate_reference clippy)"
workspace_tests_ref="$(gate_reference workspace-tests)"
audit_ref="$(gate_reference audit)"
deny_ref="$(gate_reference deny)"
sbom_ref="$(gate_reference sbom)"
locked_release_build_ref="$(gate_reference locked-release-build)"
package_smoke_ref="$(gate_reference package-smoke)"
scale_1000_ref="$(gate_reference scale-1000)"
scale_5000_ref="$(gate_reference scale-5000)"
performance_ref="$(gate_reference performance)"
clean_worktree_ref="$(gate_reference clean-worktree)"

completed_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
jq -n \
  --arg schemaVersion brainmap-m8-release-v1 \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg startedAt "${started_at}" \
  --arg completedAt "${completed_at}" \
  --arg kernelName "$(uname -s)" \
  --arg kernelRelease "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg rustc "$(rustc --version)" \
  --arg cargo "$(cargo --version)" \
  --arg reproducibilityManifestSha256 "${reproducibility_manifest_sha256}" \
  --argjson format "${format_ref}" \
  --argjson clippy "${clippy_ref}" \
  --argjson workspaceTests "${workspace_tests_ref}" \
  --argjson audit "${audit_ref}" \
  --argjson deny "${deny_ref}" \
  --argjson sbom "${sbom_ref}" \
  --argjson lockedReleaseBuild "${locked_release_build_ref}" \
  --argjson packageSmoke "${package_smoke_ref}" \
  --argjson scale1000 "${scale_1000_ref}" \
  --argjson scale5000 "${scale_5000_ref}" \
  --argjson performance "${performance_ref}" \
  --argjson cleanWorktree "${clean_worktree_ref}" '
  {
    schemaVersion: $schemaVersion,
    qualificationEligible: true,
    candidate: {
      commit: $commit,
      brainmapSha256: $brainmapSha256,
      brainmapdSha256: $brainmapdSha256
    },
    sourceTreeDirtyBefore: false,
    sourceTreeDirtyAfter: false,
    startedAt: $startedAt,
    completedAt: $completedAt,
    host: {
      kernelName: $kernelName,
      kernelRelease: $kernelRelease,
      architecture: $architecture
    },
    toolchain: {rustc: $rustc, cargo: $cargo},
    reproducibilityManifestSha256: $reproducibilityManifestSha256,
    gates: {
      format: $format,
      clippy: $clippy,
      workspaceTests: $workspaceTests,
      audit: $audit,
      deny: $deny,
      sbom: $sbom,
      lockedReleaseBuild: $lockedReleaseBuild,
      packageSmoke: $packageSmoke,
      scale1000: $scale1000,
      scale5000: $scale5000,
      performance: $performance,
      cleanWorktree: $cleanWorktree
    },
    privacy: {
      rawPromptsRetained: false,
      secretsRetained: false,
      privatePathsRetained: false,
      syntheticInputsOnly: true
    }
  }
' >"${raw}/manifest.json"

cat >"${raw}/README.md" <<'EOF'
# Brainmap FIA-8 release evidence

This bundle was emitted atomically by the independent M8 release runner.
`manifest.json` binds one clean candidate commit and two exact reproducible
release binary hashes to twelve strict FIA-8 gate results. Detailed command
identities and sanitized logs are retained under `gates/`.
`SHA256SUMS` recursively covers every retained artifact except itself.
EOF

while IFS= read -r -d '' json_file; do
  jq -e . "${json_file}" >/dev/null || die "retained invalid JSON: ${json_file}"
done < <(find "${raw}" -type f -name '*.json' -print0)

while IFS= read -r -d '' retained; do
  iconv -f UTF-8 -t UTF-8 "${retained}" >/dev/null 2>&1 ||
    die "retained artifact is not valid UTF-8: ${retained}"
done < <(find "${raw}" -type f -print0)

if grep -RIEi \
  '(/home/|/Users/|/tmp/|/opt/|/root/|/var/folders/|[A-Za-z]:\\Users\\)' \
  "${raw}" >/dev/null; then
  die "sanitized evidence retains an absolute private path"
fi
if grep -RIEi \
  '"(prompt|messages|transcript|situation|options|toolarguments)"' \
  "${raw}" >/dev/null; then
  die "sanitized evidence retains a raw prompt or transcript field"
fi
while IFS= read -r -d '' retained; do
  contains_secret_like_material "${retained}" &&
    die "sanitized evidence retains a secret-like value"
done < <(find "${raw}" -type f -print0)

(
  cd "${raw}"
  find . -type f ! -name SHA256SUMS -print0 |
    sort -z |
    while IFS= read -r -d '' retained; do
      sha256sum "${retained#./}"
    done
) >"${staging}/SHA256SUMS"
mv "${staging}/SHA256SUMS" "${raw}/SHA256SUMS"
(cd "${raw}" && sha256sum -c SHA256SUMS >/dev/null) ||
  die "recursive evidence checksums failed verification"

verify_input_hashes
assert_candidate_state
[[ ! -e "${out}" && ! -L "${out}" ]] ||
  die "evidence directory appeared during qualification: ${out}"
rm -rf "${work}"
sync -f "${raw}"
mv -T -n "${raw}" "${out}"
[[ ! -e "${raw}" ]] ||
  die "evidence directory appeared during atomic publication: ${out}"
if ! sync -f "${out}" || ! sync -f "${out_parent}"; then
  rm -rf "${out}"
  sync -f "${out_parent}" || true
  die "failed to sync atomically published release evidence"
fi
rmdir "${staging}" || true
trap - EXIT

printf 'M8 FIA-8 release qualification passed; evidence: %s\n' "${out}"
