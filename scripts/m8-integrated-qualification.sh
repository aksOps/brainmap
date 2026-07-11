#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
Run Brainmap M8 integrated acceptance drills against exact optimized binaries.

Usage:
  scripts/m8-integrated-qualification.sh \
    --brainmap PATH --brainmap-sha256 SHA256 \
    --brainmapd PATH --brainmapd-sha256 SHA256 \
    --candidate-commit COMMIT [OPTIONS]

Required:
  --brainmap PATH                 Absolute optimized brainmap binary path
  --brainmap-sha256 SHA256        Expected SHA-256 for that exact binary
  --brainmapd PATH                Absolute optimized brainmapd binary path
  --brainmapd-sha256 SHA256       Expected SHA-256 for that exact binary
  --candidate-commit COMMIT       Full 40-character source commit
  --reproducibility-manifest PATH Strict two-root release provenance (Docker)

Environment:
  --docker                        Offline ubuntu:24.04 (default)
  --local                         Explicit non-qualifying diagnostic fallback
  --docker-image IMAGE            Override the cached Docker image tag

Evidence:
  --out DIR                       Exact new evidence directory. Qualifying Docker
                                  runs require an absolute path outside the repository;
                                  local diagnostics default to a dated repository path.
  --include-fia7                  Include FIA-7 in local diagnostics; qualifying
                                  Docker runs always include FIA-7

The runner covers FIA-1 through FIA-4, FIA-6, and FIA-7. It does not claim
FIA-5 (real coding-agent host), FIA-8 (final release gate), or dogfood.
EOF
}

die() {
  echo "m8 integrated qualification: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

require_value() {
  [[ -n "${2:-}" ]] || die "$1 requires a value"
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

mode=docker
mode_was_set=false
docker_image=ubuntu:24.04
brainmap=
brainmap_sha256=
brainmapd=
brainmapd_sha256=
candidate_commit=
reproducibility_manifest=
out=
include_fia7=false
inner=false
inner_work=
inner_evidence=

while (($#)); do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --brainmap|--brainmap-sha256|--brainmapd|--brainmapd-sha256|--candidate-commit|--reproducibility-manifest|--out|--docker-image|--inner-work|--inner-evidence)
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
        --docker-image) docker_image="$2" ;;
        --inner-work) inner_work="$2" ;;
        --inner-evidence) inner_evidence="$2" ;;
      esac
      shift 2
      ;;
    --docker|--local)
      requested_mode="${1#--}"
      if [[ "${mode_was_set}" == true && "${mode}" != "${requested_mode}" ]]; then
        die "--docker and --local are mutually exclusive"
      fi
      mode="${requested_mode}"
      mode_was_set=true
      shift
      ;;
    --include-fia7)
      include_fia7=true
      shift
      ;;
    __inner)
      inner=true
      shift
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

validate_outer_inputs() {
  [[ -n "${brainmap}" ]] || die "missing required --brainmap PATH"
  [[ -n "${brainmap_sha256}" ]] || die "missing required --brainmap-sha256 SHA256"
  [[ -n "${brainmapd}" ]] || die "missing required --brainmapd PATH"
  [[ -n "${brainmapd_sha256}" ]] || die "missing required --brainmapd-sha256 SHA256"
  [[ "${brainmap}" == /* && "${brainmapd}" == /* ]] ||
    die "binary paths must be an absolute path"
  [[ -f "${brainmap}" && -x "${brainmap}" ]] ||
    die "brainmap is not an executable regular file: ${brainmap}"
  [[ -f "${brainmapd}" && -x "${brainmapd}" ]] ||
    die "brainmapd is not an executable regular file: ${brainmapd}"
  require_command sha256sum
  local label path expected actual
  for label in brainmap brainmapd; do
    if [[ "${label}" == brainmap ]]; then
      path="${brainmap}"
      expected="${brainmap_sha256}"
    else
      path="${brainmapd}"
      expected="${brainmapd_sha256}"
    fi
    [[ "${expected}" =~ ^[0-9a-f]{64}$ ]] ||
      die "${label} SHA-256 must be exactly 64 lowercase hexadecimal characters"
    actual="$(sha256sum "${path}" | cut -d ' ' -f 1)"
    [[ "${actual}" == "${expected}" ]] ||
      die "${label} SHA-256 mismatch: expected ${expected}, got ${actual}"
  done
  [[ -n "${candidate_commit}" ]] || die "missing required --candidate-commit COMMIT"
  [[ "${candidate_commit}" =~ ^[0-9a-f]{40}$ ]] ||
    die "candidate commit must be exactly 40 lowercase hexadecimal characters"
  require_command git
  git -C "${root}" cat-file -e "${candidate_commit}^{commit}" 2>/dev/null ||
    die "candidate commit does not resolve in this repository"

  if [[ "${mode}" == docker ]]; then
    [[ -n "${reproducibility_manifest}" ]] ||
      die "missing required --reproducibility-manifest PATH for qualifying Docker mode"
    [[ -n "${out}" && "${out}" == /* ]] ||
      die "qualifying Docker mode requires explicit absolute --out outside the repository"
    if [[ "${out}" == "${root}" || "${out}" == "${root}/"* ]]; then
      die "qualifying Docker mode requires evidence output outside the repository"
    fi
    local head
    head="$(git -C "${root}" rev-parse HEAD)"
    [[ "${candidate_commit}" == "${head}" ]] ||
      die "qualifying Docker mode requires candidate commit at clean HEAD"
  fi
  if [[ -n "${reproducibility_manifest}" ]]; then
    [[ "${reproducibility_manifest}" == /* ]] ||
      die "reproducibility manifest path must be absolute"
    [[ -f "${reproducibility_manifest}" ]] ||
      die "reproducibility manifest is not a regular file: ${reproducibility_manifest}"
    require_command jq
    local brainmap_build_info brainmapd_build_info build_info_sha producer_digests
    local integrated_producer_sha
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
    build_info_sha="$(printf '%s' "${brainmap_build_info}" | sha256sum | cut -d ' ' -f 1)"
    producer_digests="$(jq -c '.producerDigests' <<<"${brainmap_build_info}")"
    integrated_producer_sha="$(sha256sum "${root}/scripts/m8-integrated-qualification.sh" | cut -d ' ' -f 1)"
    jq -e \
      --arg candidateCommit "${candidate_commit}" \
      --arg brainmapSha256 "${brainmap_sha256}" \
      --arg brainmapdSha256 "${brainmapd_sha256}" \
      --arg buildInfoSha256 "${build_info_sha}" \
      --arg integratedProducerSha256 "${integrated_producer_sha}" \
      --argjson producerDigests "${producer_digests}" '
      type == "object"
      and (keys == [
        "brainmapSha256", "brainmapdSha256", "buildInfoSha256",
        "candidateCommit", "cleanTree", "locked", "producerDigests",
        "profile", "schemaVersion", "twoRootByteIdentical"
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
      and .producerDigests.integratedQualificationSha256 == $integratedProducerSha256
    ' "${reproducibility_manifest}" >/dev/null ||
      die "invalid strict reproducibility manifest"
  fi
  if [[ "${mode}" == docker ]]; then
    [[ -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
      die "qualifying Docker mode requires candidate commit at clean HEAD"
  fi
}

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

run_fia7() {
  local work="$1" evidence="$2" binary="$3"
  local source="${work}/fia7-source"
  local restored="${work}/fia7-restored"
  local old="${work}/fia7-old"
  local archive="${work}/fia7-new.brainmap.tar.zst"
  local old_archive="${work}/fia7-old.brainmap.tar.zst"

  run_command FIA-7 fia7-source-init '<brainmap> init-vault --vault <source> --yes' \
    "${binary}" init-vault --vault "${source}" --yes
  run_command FIA-7 fia7-source-learn '<brainmap> learn-decision <learned-rule> --vault <source>' \
    "${binary}" learn-decision \
      --situation 'Choose formatter for recovery project' \
      --options 'biome|prettier' --chosen biome --rejected prettier \
      --decision-type tooling --scope project:fia7 --vault "${source}"
  run_command FIA-7 fia7-source-correction-baseline \
    '<brainmap> learn-decision <correction-baseline> --vault <source>' \
    "${binary}" learn-decision \
      --situation 'Choose package manager for recovery project' \
      --options 'npm|pnpm' --chosen npm --rejected pnpm \
      --decision-type tooling --scope project:fia7 --vault "${source}"
  run_command FIA-7 fia7-source-apply-baselines \
    '<brainmap> apply --pending --yes --vault <source>' \
    "${binary}" apply --pending --yes --vault "${source}"
  gate "${source}" fia7-source-correction-before \
    'Choose package manager for recovery project' 'npm|pnpm' tooling project:fia7 false
  local correction_decision_id
  correction_decision_id="$(sed -n \
    's/^[[:space:]]*"decisionId": "\([^"]*\)",*/\1/p' \
    "${evidence}/outputs/fia7-source-correction-before.stdout")"
  [[ "${correction_decision_id}" =~ ^dec_[0-9A-Za-z_]+$ ]] ||
    die "FIA-7 correction source did not return a valid non-dry decision ID"
  run_command FIA-7 fia7-source-record-correction \
    '<brainmap> record-decision --decision-id <decision-id> --chosen pnpm --was-asked true' \
    "${binary}" record-decision --decision-id "${correction_decision_id}" \
      --chosen pnpm --was-asked true --vault "${source}"
  run_command FIA-7 fia7-source-feedback-correction \
    '<brainmap> learn-feedback --decision-id <decision-id> --chosen pnpm --rejected npm' \
    "${binary}" learn-feedback --decision-id "${correction_decision_id}" \
      --chosen pnpm --rejected npm --vault "${source}"
  run_command FIA-7 fia7-source-preview-correction \
    '<brainmap> apply --pending --dry-run --vault <source>' \
    "${binary}" apply --pending --dry-run --vault "${source}"
  run_command FIA-7 fia7-source-apply-correction \
    '<brainmap> apply --pending --yes --vault <source>' \
    "${binary}" apply --pending --yes --vault "${source}"

  local source_policy="${source}/20-decision-frames/fia7-policy.md"
  cat >"${source_policy}" <<'EOF'
---
id: fia7-policy
type: decision-policy
status: tested
confidence: high
risk_tier: reversible-auto
sensitivity: personal
---
# FIA-7 recovery policy

## Deterministic Rule

<!-- brainmap-decision-rule:v1 {"situation":"Choose test runner for recovery project","decision_type":"tooling","scope":"project:fia7","options":["cargo test","cargo nextest"],"chosen":"cargo nextest","rejected":["cargo test"]} -->
EOF
  record_internal FIA-7 fia7-source-write-policy \
    'write canonical executable recovery policy under <source>'
  run_command FIA-7 fia7-source-rebuild-policy \
    '<brainmap> index rebuild --vault <source>' \
    "${binary}" index rebuild --vault "${source}"
  gate "${source}" fia7-source-learned-gate \
    'Choose formatter for recovery project' 'biome|prettier' tooling project:fia7 true
  gate "${source}" fia7-source-corrected-gate \
    'Choose package manager for recovery project' 'npm|pnpm' tooling project:fia7 true
  gate "${source}" fia7-source-policy-gate \
    'Choose test runner for recovery project' \
    'cargo test|cargo nextest' tooling project:fia7 true
  run_command FIA-7 fia7-export '<brainmap> export --mode portable --vault <source> --out <archive>' \
    "${binary}" export --mode portable --vault "${source}" --out "${archive}"
  run_command FIA-7 fia7-verify-export '<brainmap> verify-export <archive>' \
    "${binary}" verify-export "${archive}"
  run_command FIA-7 fia7-restore '<brainmap> restore --file <archive> --to <restored>' \
    "${binary}" restore --file "${archive}" --to "${restored}"
  run_command FIA-7 fia7-restored-index '<brainmap> index verify --vault <restored>' \
    "${binary}" index verify --vault "${restored}"
  run_command FIA-7 fia7-restored-links '<brainmap> link-check --vault <restored>' \
    "${binary}" link-check --vault "${restored}"
  gate "${restored}" fia7-restored-learned-gate \
    'Choose formatter for recovery project' 'biome|prettier' tooling project:fia7 true
  gate "${restored}" fia7-restored-corrected-gate \
    'Choose package manager for recovery project' 'npm|pnpm' tooling project:fia7 true
  gate "${restored}" fia7-restored-policy-gate \
    'Choose test runner for recovery project' \
    'cargo test|cargo nextest' tooling project:fia7 true

  run_command FIA-7 fia7-old-init '<brainmap> init-vault --vault <old> --yes' \
    "${binary}" init-vault --vault "${old}" --yes
  run_command FIA-7 fia7-old-learn '<brainmap> learn-decision <old-rule> --vault <old>' \
    "${binary}" learn-decision \
      --situation 'Choose formatter for recovery project' \
      --options 'biome|prettier' --chosen prettier --rejected biome \
      --decision-type tooling --scope project:fia7 --vault "${old}"
  run_command FIA-7 fia7-old-apply '<brainmap> apply --pending --yes --vault <old>' \
    "${binary}" apply --pending --yes --vault "${old}"
  run_command FIA-7 fia7-old-export '<brainmap> export --mode portable --vault <old> --out <old-archive>' \
    "${binary}" export --mode portable --vault "${old}" --out "${old_archive}"
  run_command FIA-7 fia7-old-verify '<brainmap> verify-export <old-archive>' \
    "${binary}" verify-export "${old_archive}"

  local old_complete="${work}/fia7-old-complete"
  local new_complete="${work}/fia7-new-complete"
  run_command FIA-7 fia7-old-baseline '<brainmap> restore --file <old-archive> --to <old-baseline>' \
    "${binary}" restore --file "${old_archive}" --to "${old_complete}"
  run_command FIA-7 fia7-new-baseline '<brainmap> restore --file <archive> --to <new-baseline>' \
    "${binary}" restore --file "${archive}" --to "${new_complete}"
  local old_hash new_hash
  old_hash="$(canonical_tree_hash "${old_complete}")"
  new_hash="$(canonical_tree_hash "${new_complete}")"
  [[ "${old_hash}" != "${new_hash}" ]] ||
    die "FIA-7 old and new complete vault states are indistinguishable"

  : >"${evidence}/reports/fia7-faults.jsonl"
  local phases=(
    verified
    staging-created
    files-written
    index-rebuilt
    links-checked
    gate-checked
    existing-backed-up
    staging-activated
  )
  local phase target fault_hash complete_state
  for phase in "${phases[@]}"; do
    target="${work}/fia7-fault-${phase}"
    run_command FIA-7 "fia7-${phase}-seed" \
      '<brainmap> restore --file <old-archive> --to <fault-target>' \
      "${binary}" restore --file "${old_archive}" --to "${target}"
    run_expected_failure FIA-7 "fia7-${phase}-fault" \
      '<brainmap> restore --file <archive> --to <fault-target> --fault-phase <phase>' \
      "${binary}" restore --file "${archive}" --to "${target}" --fault-phase "${phase}"
    run_command FIA-7 "fia7-${phase}-index" \
      '<brainmap> index verify --vault <fault-target>' \
      "${binary}" index verify --vault "${target}"
    run_command FIA-7 "fia7-${phase}-links" \
      '<brainmap> link-check --vault <fault-target>' \
      "${binary}" link-check --vault "${target}"
    gate "${target}" "fia7-fault-${phase}-gate" \
      'Choose formatter for recovery project' 'biome|prettier' tooling project:fia7 true
    fault_hash="$(canonical_tree_hash "${target}")"
    if [[ "${fault_hash}" == "${old_hash}" ]]; then
      complete_state=old
    elif [[ "${fault_hash}" == "${new_hash}" ]]; then
      complete_state=new
    else
      die "FIA-7 fault phase ${phase} left a noncanonical vault"
    fi
    printf '{"phase":"%s","completeState":"%s","treeHash":"%s"}\n' \
      "${phase}" "${complete_state}" "${fault_hash}" \
      >>"${evidence}/reports/fia7-faults.jsonl"
  done
  printf '{"exportVerified":true,"archiveSha256":"%s","oldTreeHash":"%s","newTreeHash":"%s","behaviorPairs":3,"faultPhases":8,"canonicalFaultStates":8}\n' \
    "$(sha256sum "${archive}" | cut -d ' ' -f 1)" "${old_hash}" "${new_hash}" \
    >"${evidence}/reports/fia7-inner.json"
}

run_inner() {
  local work="${inner_work}"
  local evidence="${inner_evidence}"
  [[ -x "${brainmap}" && -x "${brainmapd}" ]] ||
    die "inner runner requires both executable binaries"
  [[ -n "${work}" && -n "${evidence}" ]] ||
    die "inner runner requires work and evidence directories"
  [[ "${brainmap_sha256}" =~ ^[0-9a-f]{64}$ &&
     "${brainmapd_sha256}" =~ ^[0-9a-f]{64}$ ]] ||
    die "inner runner requires both expected binary SHA-256 values"
  [[ "$(sha256sum "${brainmap}" | cut -d ' ' -f 1)" == "${brainmap_sha256}" ]] ||
    die "inner brainmap SHA-256 mismatch"
  [[ "$(sha256sum "${brainmapd}" | cut -d ' ' -f 1)" == "${brainmapd_sha256}" ]] ||
    die "inner brainmapd SHA-256 mismatch"

  mkdir -p "${work}/home" "${work}/tmp" "${work}/project" \
    "${evidence}/outputs" "${evidence}/reports"
  export HOME="${work}/home"
  export TMPDIR="${work}/tmp"
  cd "${work}/project"

  local commands_tsv="${evidence}/commands.tsv"
  local sequence=0
  : >"${commands_tsv}"

  append_result() {
    local fia="$1" id="$2" display="$3" expected="$4" exit_code="$5"
    local passed=false
    sequence=$((sequence + 1))
    if [[ "${expected}" == 0 && "${exit_code}" == 0 ]] ||
       [[ "${expected}" == nonzero && "${exit_code}" != 0 ]]; then
      passed=true
    fi
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "${sequence}" "${fia}" "${id}" "${display}" "${expected}" "${exit_code}" "${passed}" \
      >>"${commands_tsv}"
    if [[ "${passed}" != true ]]; then
      echo "${fia} command ${id} exited ${exit_code}; expected ${expected}" >&2
      return 1
    fi
  }

  sanitize_output() {
    sanitize_one_file "$1" \
      "${work}" '<qualification-work>' \
      "${brainmap}" '<brainmap>' \
      "${brainmapd}" '<brainmapd>' \
      "${HOME}" '<qualification-home>' \
      "${TMPDIR}" '<qualification-tmp>'
  }

  run_command() {
    local fia="$1" id="$2" display="$3"
    shift 3
    local stdout="${evidence}/outputs/${id}.stdout"
    local stderr="${evidence}/outputs/${id}.stderr"
    local exit_code
    set +e
    "$@" >"${stdout}" 2>"${stderr}"
    exit_code=$?
    set -e
    sanitize_output "${stdout}"
    sanitize_output "${stderr}"
    append_result "${fia}" "${id}" "${display}" 0 "${exit_code}"
  }

  run_expected_failure() {
    local fia="$1" id="$2" display="$3"
    shift 3
    local stdout="${evidence}/outputs/${id}.stdout"
    local stderr="${evidence}/outputs/${id}.stderr"
    local exit_code
    set +e
    "$@" >"${stdout}" 2>"${stderr}"
    exit_code=$?
    set -e
    sanitize_output "${stdout}"
    sanitize_output "${stderr}"
    append_result "${fia}" "${id}" "${display}" nonzero "${exit_code}"
  }

  run_with_input() {
    local fia="$1" id="$2" display="$3" input="$4"
    shift 4
    local stdout="${evidence}/outputs/${id}.stdout"
    local stderr="${evidence}/outputs/${id}.stderr"
    local exit_code
    set +e
    "$@" <"${input}" >"${stdout}" 2>"${stderr}"
    exit_code=$?
    set -e
    sanitize_output "${stdout}"
    sanitize_output "${stderr}"
    append_result "${fia}" "${id}" "${display}" 0 "${exit_code}"
  }

  record_internal() {
    append_result "$1" "$2" "$3" 0 0
  }

  gate() {
    local vault="$1" output_id="$2" situation="$3" options="$4"
    local decision_type="$5" scope="$6" dry_run="$7"
    local args=(
      gate --json --intent would-ask-user
      --situation "${situation}" --options "${options}"
      --risk low --reversible true --decision-type "${decision_type}"
      --scope "${scope}" --vault "${vault}"
    )
    [[ "${dry_run}" == false ]] || args+=(--dry-run)
    local fia_number="${output_id#fia}"
    fia_number="${fia_number%%-*}"
    run_command "FIA-${fia_number}" "${output_id}" \
      '<brainmap> gate <synthetic-request> --vault <vault>' \
      "${brainmap}" "${args[@]}"
  }

  run_command PRECHECK brainmap-version '<brainmap> --version' \
    "${brainmap}" --version
  run_command PRECHECK brainmapd-help '<brainmapd> --help' \
    "${brainmapd}" --help

  # FIA-1: three interactive answers, exact previews, explicit approval,
  # automatic index rebuild, and a decision prediction derived from an answer.
  local fia1_vault="${work}/fia1-vault"
  run_command FIA-1 fia1-init '<brainmap> init-vault --vault <vault> --yes' \
    "${brainmap}" init-vault --vault "${fia1_vault}" --yes
  cat >"${work}/fia1-answers.txt" <<'EOF'
follow project configuration
ask user
make the smallest reversible change

y
EOF
  run_with_input FIA-1 fia1-onboard \
    '<brainmap> onboard --vault <vault> < three answers and approval' \
    "${work}/fia1-answers.txt" "${brainmap}" onboard --vault "${fia1_vault}"
  sed -n 's/^onboarding exact executable update preview: //p' \
    "${evidence}/outputs/fia1-onboard.stdout" \
    >"${evidence}/outputs/fia1-previews.jsonl"
  gate "${fia1_vault}" fia1-derived-gate \
    'When a project declares a formatter, choose the formatter policy' \
    'ask user|follow project configuration' tooling project:auto true

  # FIA-2: a project formatter preference must generalize to five supported
  # paraphrases and not leak by ecosystem or decision domain.
  local fia2_vault="${work}/fia2-vault"
  run_command FIA-2 fia2-init '<brainmap> init-vault --vault <vault> --yes' \
    "${brainmap}" init-vault --vault "${fia2_vault}" --yes
  run_command FIA-2 fia2-learn '<brainmap> learn-decision <formatter-rule> --vault <vault>' \
    "${brainmap}" learn-decision \
      --situation 'Choose formatter for a Rust repository' \
      --options 'rustfmt|a custom formatter' \
      --chosen 'a custom formatter' --rejected rustfmt \
      --decision-type tooling --scope project:fia2 --vault "${fia2_vault}"
  run_command FIA-2 fia2-preview '<brainmap> apply --pending --dry-run --vault <vault>' \
    "${brainmap}" apply --pending --dry-run --vault "${fia2_vault}"
  run_command FIA-2 fia2-apply '<brainmap> apply --pending --yes --vault <vault>' \
    "${brainmap}" apply --pending --yes --vault "${fia2_vault}"
  gate "${fia2_vault}" fia2-exact \
    'Choose formatter for a Rust repository' \
    'rustfmt|a custom formatter' tooling project:fia2 true
  local paraphrases=(
    'Pick a formatter for this Rust repo'
    'What formatting tool should this Rust codebase use?'
    'Select the formatter for the Rust codebase'
    'Decide on formatting for this Rust repo'
    'Use a formatter in the Rust repository'
  )
  local index=0 situation
  for situation in "${paraphrases[@]}"; do
    index=$((index + 1))
    gate "${fia2_vault}" "$(printf 'fia2-paraphrase-%02d' "${index}")" \
      "${situation}" 'rustfmt|a custom formatter' tooling project:fia2 true
  done
  gate "${fia2_vault}" fia2-negative-ecosystem \
    'Choose formatter for a Python repository' \
    'rustfmt|a custom formatter|ruff format|black' tooling project:fia2 true
  gate "${fia2_vault}" fia2-negative-database \
    'Choose a database for a Rust repository' \
    'rustfmt|a custom formatter|SQLite|PostgreSQL' tooling project:fia2 true
  gate "${fia2_vault}" fia2-negative-logging \
    'Choose a logging library for a Rust repository' \
    'rustfmt|a custom formatter|tracing|log' tooling project:fia2 true
  gate "${fia2_vault}" fia2-negative-package-manager \
    'Choose a package manager for a Rust repository' \
    'rustfmt|a custom formatter|cargo|buck2' tooling project:fia2 true

  # FIA-3: a non-dry decision and independently recorded action feed a
  # structured correction. Dry-run preview cannot activate it; approval must.
  local fia3_vault="${work}/fia3-vault"
  run_command FIA-3 fia3-init '<brainmap> init-vault --vault <vault> --yes' \
    "${brainmap}" init-vault --vault "${fia3_vault}" --yes
  run_command FIA-3 fia3-learn-baseline '<brainmap> learn-decision <baseline-rule> --vault <vault>' \
    "${brainmap}" learn-decision \
      --situation 'Choose package manager for correction project' \
      --options 'npm|pnpm' --chosen npm --rejected pnpm \
      --decision-type tooling --scope project:fia3 --vault "${fia3_vault}"
  run_command FIA-3 fia3-apply-baseline '<brainmap> apply --pending --yes --vault <vault>' \
    "${brainmap}" apply --pending --yes --vault "${fia3_vault}"
  gate "${fia3_vault}" fia3-source-gate \
    'Choose package manager for correction project' 'npm|pnpm' tooling project:fia3 false
  local fia3_decision_id
  fia3_decision_id="$(sed -n 's/^[[:space:]]*"decisionId": "\([^"]*\)",*/\1/p' \
    "${evidence}/outputs/fia3-source-gate.stdout")"
  [[ "${fia3_decision_id}" =~ ^dec_[0-9A-Za-z_]+$ ]] ||
    die "FIA-3 did not return one valid non-dry decision ID"
  run_command FIA-3 fia3-record-actual \
    '<brainmap> record-decision --decision-id <decision-id> --chosen pnpm --was-asked true' \
    "${brainmap}" record-decision --decision-id "${fia3_decision_id}" \
      --chosen pnpm --was-asked true --vault "${fia3_vault}"
  run_command FIA-3 fia3-feedback \
    '<brainmap> learn-feedback --decision-id <decision-id> --chosen pnpm --rejected npm' \
    "${brainmap}" learn-feedback --decision-id "${fia3_decision_id}" \
      --chosen pnpm --rejected npm --vault "${fia3_vault}"
  run_command FIA-3 fia3-preview '<brainmap> apply --pending --dry-run --vault <vault>' \
    "${brainmap}" apply --pending --dry-run --vault "${fia3_vault}"
  gate "${fia3_vault}" fia3-before-approval \
    'Choose package manager for correction project' 'npm|pnpm' tooling project:fia3 true
  run_command FIA-3 fia3-approve '<brainmap> apply --pending --yes --vault <vault>' \
    "${brainmap}" apply --pending --yes --vault "${fia3_vault}"
  gate "${fia3_vault}" fia3-after-approval \
    'Choose package manager for correction project' 'npm|pnpm' tooling project:fia3 true
  run_command FIA-3 fia3-learn-more-relevant \
    '<brainmap> learn-decision <more-relevant-rule> --vault <vault>' \
    "${brainmap}" learn-decision \
      --situation 'Choose package manager for correction project deployment pipeline' \
      --options 'npm|pnpm' --chosen npm --rejected pnpm \
      --decision-type tooling --scope project:fia3 --vault "${fia3_vault}"
  run_command FIA-3 fia3-preview-more-relevant \
    '<brainmap> apply --pending --dry-run --vault <vault>' \
    "${brainmap}" apply --pending --dry-run --vault "${fia3_vault}"
  run_command FIA-3 fia3-apply-more-relevant \
    '<brainmap> apply --pending --yes --vault <vault>' \
    "${brainmap}" apply --pending --yes --vault "${fia3_vault}"
  gate "${fia3_vault}" fia3-more-relevant \
    'Choose package manager for correction project deployment pipeline' \
    'npm|pnpm' tooling project:fia3 true
  gate "${fia3_vault}" fia3-other-scope \
    'Choose package manager for correction project' 'npm|pnpm' tooling project:fia3-other true
  gate "${fia3_vault}" fia3-unrelated-decision \
    'Choose database for correction project' 'SQLite|PostgreSQL' tooling project:fia3 true

  # FIA-4: executable policy metadata must be causal only while active.
  local fia4_vault="${work}/fia4-vault"
  local fia4_policy="${fia4_vault}/20-decision-frames/fia4-policy.md"
  local fia4_decoy_policy="${fia4_vault}/20-decision-frames/fia4-decoy-policy.md"
  run_command FIA-4 fia4-init '<brainmap> init-vault --vault <vault> --yes' \
    "${brainmap}" init-vault --vault "${fia4_vault}" --yes
  gate "${fia4_vault}" fia4-before \
    'Choose test runner for policy project' 'cargo test|cargo nextest' tooling project:fia4 true
  cat >"${fia4_policy}" <<'EOF'
---
id: fia4-policy
type: decision-policy
status: tested
confidence: high
risk_tier: reversible-auto
sensitivity: personal
---
# FIA-4 test runner policy

## Deterministic Rule

<!-- brainmap-decision-rule:v1 {"situation":"Choose test runner for policy project","decision_type":"tooling","scope":"project:fia4","options":["cargo test","cargo nextest"],"chosen":"cargo nextest","rejected":["cargo test"]} -->
EOF
  cat >"${fia4_decoy_policy}" <<'EOF'
---
id: fia4-decoy-policy
type: decision-policy
status: tested
confidence: high
risk_tier: reversible-auto
sensitivity: personal
---
# FIA-4 noncausal decoy policy

## Deterministic Rule

<!-- brainmap-decision-rule:v1 {"situation":"Choose logging library for another policy project","decision_type":"tooling","scope":"project:fia4-other","options":["tracing","log"],"chosen":"tracing","rejected":["log"]} -->
EOF
  record_internal FIA-4 fia4-write-policy \
    'write matching and noncausal active Markdown policies under <vault>'
  run_command FIA-4 fia4-rebuild-active '<brainmap> index rebuild --vault <vault>' \
    "${brainmap}" index rebuild --vault "${fia4_vault}"
  gate "${fia4_vault}" fia4-active \
    'Choose test runner for policy project' 'cargo test|cargo nextest' tooling project:fia4 true
  gate "${fia4_vault}" fia4-unrelated \
    'Choose database for policy project' 'SQLite|PostgreSQL' tooling project:fia4 true
  sed -i 's/^status: tested$/status: retired/' "${fia4_policy}"
  record_internal FIA-4 fia4-retire-policy 'change canonical Markdown policy status to retired'
  run_command FIA-4 fia4-rebuild-retired '<brainmap> index rebuild --vault <vault>' \
    "${brainmap}" index rebuild --vault "${fia4_vault}"
  gate "${fia4_vault}" fia4-retired \
    'Choose test runner for policy project' 'cargo test|cargo nextest' tooling project:fia4 true

  # FIA-6: a barrier-overlapped wave of 16 gate and 16 recording processes,
  # followed by a barrier-overlapped wave of 16 capture and 16 feedback
  # processes (64 Brainmap OS processes total).
  local fia6_vault="${work}/fia6-vault"
  local fia6_dir="${work}/fia6-processes"
  mkdir -p "${fia6_dir}"
  run_command FIA-6 fia6-init '<brainmap> init-vault --vault <vault> --yes' \
    "${brainmap}" init-vault --vault "${fia6_vault}" --yes
  run_command FIA-6 fia6-index '<brainmap> index rebuild --vault <vault>' \
    "${brainmap}" index rebuild --vault "${fia6_vault}"

  local -a pids=()
  local -a process_ids=()
  local -a process_displays=()
  local process_id exit_code wave_ready_count _attempt
  local wave_one_ready="${fia6_dir}/wave-one-ready"
  local wave_one_release="${fia6_dir}/wave-one-release"
  mkdir "${wave_one_ready}"
  for index in $(seq 1 16); do
    process_id="$(printf 'fia6-gate-%02d' "${index}")"
    (
      : >"${wave_one_ready}/${process_id}"
      while [[ ! -e "${wave_one_release}" ]]; do sleep 0.01; done
      exec "${brainmap}" gate --json --intent would-ask-user \
        --situation 'Choose formatter for the concurrent Rust project' \
        --options 'biome|prettier' --risk low --reversible true \
        --decision-type tooling --scope project:fia6 --vault "${fia6_vault}"
    ) \
      >"${fia6_dir}/${process_id}.stdout" 2>"${fia6_dir}/${process_id}.stderr" &
    pids+=("$!")
    process_ids+=("${process_id}")
    process_displays+=('<brainmap> gate <synthetic-request> --vault <vault>')

    process_id="$(printf 'fia6-record-%02d' "${index}")"
    (
      : >"${wave_one_ready}/${process_id}"
      while [[ ! -e "${wave_one_release}" ]]; do sleep 0.01; done
      exec "${brainmap}" record-decision --chosen biome --was-asked true \
        --vault "${fia6_vault}"
    ) \
      >"${fia6_dir}/${process_id}.stdout" 2>"${fia6_dir}/${process_id}.stderr" &
    pids+=("$!")
    process_ids+=("${process_id}")
    process_displays+=('<brainmap> record-decision --chosen biome --was-asked true --vault <vault>')
  done
  wave_ready_count=0
  for _attempt in $(seq 1 1000); do
    wave_ready_count="$(find "${wave_one_ready}" -maxdepth 1 -type f | wc -l)"
    [[ "${wave_ready_count}" -eq 32 ]] && break
    sleep 0.01
  done
  [[ "${wave_ready_count}" -eq 32 ]] ||
    die "FIA-6 gate/record overlap barrier did not collect all 32 workers"
  record_internal FIA-6 fia6-overlap-gate-record \
    'release 16 gate and 16 recording OS processes from one overlap barrier'
  : >"${wave_one_release}"
  for index in "${!pids[@]}"; do
    if wait "${pids[${index}]}"; then exit_code=0; else exit_code=$?; fi
    append_result FIA-6 "${process_ids[${index}]}" \
      "${process_displays[${index}]}" 0 "${exit_code}"
  done

  local -a gate_ids=()
  local gate_id
  for index in $(seq 1 16); do
    process_id="$(printf 'fia6-gate-%02d' "${index}")"
    gate_id="$(sed -n 's/^[[:space:]]*"decisionId": "\([^"]*\)",*/\1/p' \
      "${fia6_dir}/${process_id}.stdout")"
    [[ "${gate_id}" =~ ^dec_[0-9A-Za-z_]+$ ]] ||
      die "invalid FIA-6 gate decision ID"
    gate_ids+=("${gate_id}")
  done

  pids=()
  process_ids=()
  process_displays=()
  local wave_two_ready="${fia6_dir}/wave-two-ready"
  local wave_two_release="${fia6_dir}/wave-two-release"
  mkdir "${wave_two_ready}"
  for index in $(seq 1 16); do
    process_id="$(printf 'fia6-capture-%02d' "${index}")"
    (
      : >"${wave_two_ready}/${process_id}"
      while [[ ! -e "${wave_two_release}" ]]; do sleep 0.01; done
      exec "${brainmap}" capture \
        --text "When formatting concurrent Rust component ${index}, choose biome" \
        --source m8-fia6 --vault "${fia6_vault}"
    ) \
      >"${fia6_dir}/${process_id}.stdout" 2>"${fia6_dir}/${process_id}.stderr" &
    pids+=("$!")
    process_ids+=("${process_id}")
    process_displays+=('<brainmap> capture <synthetic-decision> --vault <vault>')

    process_id="$(printf 'fia6-feedback-%02d' "${index}")"
    (
      : >"${wave_two_ready}/${process_id}"
      while [[ ! -e "${wave_two_release}" ]]; do sleep 0.01; done
      exec "${brainmap}" learn-feedback \
        --decision-id "${gate_ids[$((index - 1))]}" \
        --chosen biome --rejected prettier --vault "${fia6_vault}"
    ) \
      >"${fia6_dir}/${process_id}.stdout" 2>"${fia6_dir}/${process_id}.stderr" &
    pids+=("$!")
    process_ids+=("${process_id}")
    process_displays+=('<brainmap> learn-feedback --decision-id <decision-id> --chosen biome --rejected prettier')
  done
  wave_ready_count=0
  for _attempt in $(seq 1 1000); do
    wave_ready_count="$(find "${wave_two_ready}" -maxdepth 1 -type f | wc -l)"
    [[ "${wave_ready_count}" -eq 32 ]] && break
    sleep 0.01
  done
  [[ "${wave_ready_count}" -eq 32 ]] ||
    die "FIA-6 capture/feedback overlap barrier did not collect all 32 workers"
  record_internal FIA-6 fia6-overlap-capture-feedback \
    'release 16 capture and 16 feedback OS processes from one overlap barrier'
  : >"${wave_two_release}"
  for index in "${!pids[@]}"; do
    if wait "${pids[${index}]}"; then exit_code=0; else exit_code=$?; fi
    append_result FIA-6 "${process_ids[${index}]}" \
      "${process_displays[${index}]}" 0 "${exit_code}"
  done
  run_command FIA-6 fia6-apply '<brainmap> apply --pending --yes --vault <vault>' \
    "${brainmap}" apply --pending --yes --vault "${fia6_vault}"

  if [[ "${include_fia7}" == true ]]; then
    run_fia7 "${work}" "${evidence}" "${brainmap}"
  fi

  printf 'kernelName\t%s\nkernelRelease\t%s\narchitecture\t%s\n' \
    "$(uname -s)" "$(uname -r)" "$(uname -m)" >"${evidence}/environment.tsv"
  echo 'inner operational drills completed'
}

if [[ "${inner}" == true ]]; then
  run_inner
  exit 0
fi

validate_outer_inputs

if [[ "${mode}" == docker ]]; then
  [[ "${docker_image}" == ubuntu:24.04 ]] ||
    die "qualifying Docker mode requires docker image ubuntu:24.04"
  include_fia7=true
fi

for command in jq find sort awk sed grep iconv sha256sum date mktemp cmp cp install \
  seq sleep tail od tr wc sync; do
  require_command "${command}"
done

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
stamp="$(date -u +%Y%m%dT%H%M%SZ)"
if [[ -z "${out}" ]]; then
  out="${root}/evidence/m001/m8-fia-${stamp}"
elif [[ "${out}" != /* ]]; then
  out="${root}/${out}"
fi
[[ ! -e "${out}" ]] || die "evidence directory already exists: ${out}"
out_parent="$(dirname "${out}")"
mkdir -p "${out_parent}"
out_parent="$(cd "${out_parent}" && pwd -P)"
out="${out_parent}/$(basename "${out}")"
if [[ "${mode}" == docker &&
      ( "${out_parent}" == "${root}" || "${out_parent}" == "${root}/"* ) ]]; then
  die "qualifying Docker mode requires evidence output outside the repository"
fi
[[ ! -e "${out}" && ! -L "${out}" ]] ||
  die "evidence directory already exists: ${out}"
staging="$(mktemp -d "${out_parent}/.m8-fia.XXXXXX")"
work="${staging}/work"
raw="${staging}/evidence"
candidate_dir="${staging}/candidate"
mkdir -p "${work}" "${raw}" "${candidate_dir}"

staged_brainmap="${candidate_dir}/brainmap"
staged_brainmapd="${candidate_dir}/brainmapd"
staged_runner="${candidate_dir}/m8-integrated-qualification.sh"
install -m 0555 "${brainmap}" "${staged_brainmap}"
install -m 0555 "${brainmapd}" "${staged_brainmapd}"
install -m 0555 "${root}/scripts/m8-integrated-qualification.sh" "${staged_runner}"
[[ "$(sha256sum "${staged_brainmap}" | cut -d ' ' -f 1)" == "${brainmap_sha256}" ]] ||
  die "brainmap changed while creating its immutable qualification copy"
[[ "$(sha256sum "${staged_brainmapd}" | cut -d ' ' -f 1)" == "${brainmapd_sha256}" ]] ||
  die "brainmapd changed while creating its immutable qualification copy"
staged_runner_sha256="$(sha256sum "${staged_runner}" | cut -d ' ' -f 1)"
sync -f "${candidate_dir}"

cleanup() {
  rm -rf "${staging}"
}
trap cleanup EXIT

qualifying_run=false
release_provenance_verified=false
if [[ "${mode}" == docker ]]; then
  qualifying_run=true
  release_provenance_verified=true
  cp "${reproducibility_manifest}" \
    "${raw}/release-reproducibility-manifest.json"
fi
reproducibility_manifest_sha256=
if [[ -n "${reproducibility_manifest}" ]]; then
  reproducibility_manifest_sha256="$(sha256sum "${reproducibility_manifest}" | cut -d ' ' -f 1)"
fi

assert_outer_identity() {
  [[ "$(sha256sum "${staged_brainmap}" | cut -d ' ' -f 1)" == "${brainmap_sha256}" ]] ||
    die "immutable brainmap qualification copy changed"
  [[ "$(sha256sum "${staged_brainmapd}" | cut -d ' ' -f 1)" == "${brainmapd_sha256}" ]] ||
    die "immutable brainmapd qualification copy changed"
  [[ "$(sha256sum "${brainmap}" | cut -d ' ' -f 1)" == "${brainmap_sha256}" ]] ||
    die "source brainmap changed during qualification"
  [[ "$(sha256sum "${brainmapd}" | cut -d ' ' -f 1)" == "${brainmapd_sha256}" ]] ||
    die "source brainmapd changed during qualification"
  [[ "$(sha256sum "${staged_runner}" | cut -d ' ' -f 1)" == "${staged_runner_sha256}" &&
     "$(sha256sum "${root}/scripts/m8-integrated-qualification.sh" | cut -d ' ' -f 1)" == "${staged_runner_sha256}" ]] ||
    die "integrated qualification producer changed during qualification"
  if [[ "${qualifying_run}" == true ]]; then
    [[ "$(sha256sum "${reproducibility_manifest}" | cut -d ' ' -f 1)" == "${reproducibility_manifest_sha256}" ]] ||
      die "release reproducibility manifest changed during qualification"
    [[ "$(git -C "${root}" rev-parse HEAD)" == "${candidate_commit}" &&
       -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
      die "candidate source changed during qualification"
  fi
}

docker_image_id=
if [[ "${mode}" == docker ]]; then
  require_command docker
  docker image inspect "${docker_image}" >/dev/null 2>&1 ||
    die "Docker image is not cached locally: ${docker_image}"
  docker_image_id="$(docker image inspect --format '{{.Id}}' "${docker_image}")"
  docker_args=()
  [[ "${include_fia7}" == false ]] || docker_args+=(--include-fia7)
  docker run --rm --network none --read-only --cap-drop ALL \
    --security-opt no-new-privileges --pids-limit 512 \
    --user "$(id -u):$(id -g)" \
    --tmpfs /tmp:rw,noexec,nosuid,size=64m \
    --workdir /work/project \
    --mount "type=bind,src=${work},dst=/work" \
    --mount "type=bind,src=${raw},dst=/evidence" \
    --mount "type=bind,src=${staged_brainmap},dst=/candidate/brainmap,readonly" \
    --mount "type=bind,src=${staged_brainmapd},dst=/candidate/brainmapd,readonly" \
    --mount "type=bind,src=${staged_runner},dst=/runner,readonly" \
    "${docker_image}" /runner __inner \
      --brainmap /candidate/brainmap \
      --brainmap-sha256 "${brainmap_sha256}" \
      --brainmapd /candidate/brainmapd \
      --brainmapd-sha256 "${brainmapd_sha256}" \
      --inner-work /work \
      --inner-evidence /evidence \
      "${docker_args[@]}"
else
  local_args=()
  [[ "${include_fia7}" == false ]] || local_args+=(--include-fia7)
  "${staged_runner}" __inner \
    --brainmap "${staged_brainmap}" \
    --brainmap-sha256 "${brainmap_sha256}" \
    --brainmapd "${staged_brainmapd}" \
    --brainmapd-sha256 "${brainmapd_sha256}" \
    --inner-work "${work}" \
    --inner-evidence "${raw}" \
    "${local_args[@]}"
fi

assert_outer_identity

jq -Rn '
  [inputs
   | split("\t")
   | select(length == 7)
   | {
       sequence: (.[0] | tonumber),
       fia: .[1],
       id: .[2],
       command: .[3],
       expectedExit: (if .[4] == "0" then 0 else .[4] end),
       exitCode: (.[5] | tonumber),
       passed: (.[6] == "true")
     }]
' <"${raw}/commands.tsv" >"${raw}/commands.json"
rm "${raw}/commands.tsv"
jq -e 'length > 0 and all(.passed)' "${raw}/commands.json" >/dev/null ||
  die "one or more operational commands failed"

assert_json() {
  local file="$1" expression="$2" description="$3"
  jq -e "${expression}" "${file}" >/dev/null ||
    die "acceptance assertion failed: ${description}"
}

assert_shadow_choice() {
  local file="$1" choice="$2" description="$3"
  jq -e --arg choice "${choice}" '
    .outcome == "ask_user"
    and .selectedOption == null
    and .predictedOutcome == "proceed"
    and .predictedSelectedOption == $choice
    and .gateMode == "shadow"
    and .autopilotMode == "shadow"
  ' "${file}" >/dev/null ||
    die "shadow prediction assertion failed: ${description}"
}

assert_no_prediction() {
  local file="$1" description="$2"
  jq -e '
    .outcome == "ask_user"
    and .selectedOption == null
    and .predictedOutcome == "ask_user"
    and .predictedSelectedOption == null
    and .gateMode == "shadow"
    and .autopilotMode == "shadow"
    and (.ruleId == null)
  ' "${file}" >/dev/null ||
    die "non-leak assertion failed: ${description}"
}

# FIA-1 outer assertions.
grep -F 'Calibration 1/3:' "${raw}/outputs/fia1-onboard.stdout" >/dev/null
grep -F 'Calibration 2/3:' "${raw}/outputs/fia1-onboard.stdout" >/dev/null
grep -F 'Calibration 3/3:' "${raw}/outputs/fia1-onboard.stdout" >/dev/null
grep -F 'Apply these decisions? [y/N]:' "${raw}/outputs/fia1-onboard.stdout" >/dev/null
grep -F 'onboarding applied 3 decision(s)' "${raw}/outputs/fia1-onboard.stdout" >/dev/null
jq -e -s '
  length == 3
  and ([.[].packet.id] | unique | length == 3)
  and all(.[]; .packet.decisionRule != null and .packet.status == "pending")
' "${raw}/outputs/fia1-previews.jsonl" >/dev/null ||
  die "FIA-1 did not emit three unique exact executable previews"
while IFS= read -r packet_id; do
  applied="${work}/fia1-vault/99-meta/pending-update-packets/manual-decision-${packet_id}.applied.json"
  [[ -f "${applied}" ]] || die "FIA-1 preview was not approved: ${packet_id}"
  jq -S --arg id "${packet_id}" 'select(.packet.id == $id) | .packet' \
    "${raw}/outputs/fia1-previews.jsonl" >"${work}/preview.json"
  jq -S . "${applied}" >"${work}/applied.json"
  cmp -s "${work}/preview.json" "${work}/applied.json" ||
    die "FIA-1 applied packet differs from its exact preview"
done < <(jq -r '.packet.id' "${raw}/outputs/fia1-previews.jsonl")
[[ -s "${work}/fia1-vault/.brainmap/brainmap.sqlite" ]] ||
  die "FIA-1 automatic rebuild did not produce SQLite"
jq -e . "${work}/fia1-vault/.brainmap/index-manifest.json" >/dev/null ||
  die "FIA-1 automatic rebuild did not produce a valid index manifest"
assert_shadow_choice "${raw}/outputs/fia1-derived-gate.stdout" \
  'follow project configuration' 'FIA-1 answer-derived behavior'
assert_json "${raw}/outputs/fia1-derived-gate.stdout" \
  '.ruleScope | startswith("project:")' 'FIA-1 learned scope is project-local'
jq -n '{answers: 3, previews: 3, approvedPackets: 3, automaticRebuild: true, behaviorDerived: true}' \
  >"${raw}/reports/fia1.json"

# FIA-2 outer assertions.
assert_shadow_choice "${raw}/outputs/fia2-exact.stdout" \
  'a custom formatter' 'FIA-2 exact learned formatter'
fia2_rule_id="$(jq -r '.ruleId' "${raw}/outputs/fia2-exact.stdout")"
[[ "${fia2_rule_id}" != null && -n "${fia2_rule_id}" ]] ||
  die "FIA-2 exact result did not identify its learned rule"
for index in $(seq 1 5); do
  file="${raw}/outputs/$(printf 'fia2-paraphrase-%02d' "${index}").stdout"
  assert_shadow_choice "${file}" 'a custom formatter' "FIA-2 paraphrase ${index}"
  [[ "$(jq -r '.ruleId' "${file}")" == "${fia2_rule_id}" ]] ||
    die "FIA-2 paraphrase ${index} selected a different rule"
done
for negative in ecosystem database logging package-manager; do
  assert_no_prediction "${raw}/outputs/fia2-negative-${negative}.stdout" \
    "FIA-2 negative ${negative}"
done
jq -n --arg ruleId "${fia2_rule_id}" \
  '{exact: 1, paraphrases: 5, negatives: 4, correctPredictions: 6,
    nonLeaks: 4, negativesRetainedCompatibleLearnedOptions: true,
    ruleId: $ruleId}' \
  >"${raw}/reports/fia2.json"

# FIA-3 outer assertions.
assert_shadow_choice "${raw}/outputs/fia3-source-gate.stdout" npm \
  'FIA-3 original prediction'
assert_json "${raw}/outputs/fia3-source-gate.stdout" \
  '.learningEvent.shouldRecord == true' 'FIA-3 source decision is non-dry'
assert_shadow_choice "${raw}/outputs/fia3-before-approval.stdout" npm \
  'FIA-3 preview did not activate correction'
grep -F 'would apply ' "${raw}/outputs/fia3-preview.stdout" >/dev/null ||
  die "FIA-3 correction preview was not exact and actionable"
assert_shadow_choice "${raw}/outputs/fia3-after-approval.stdout" pnpm \
  'FIA-3 approved correction changed the next prediction'
assert_shadow_choice "${raw}/outputs/fia3-more-relevant.stdout" npm \
  'FIA-3 more-relevant competing rule outranks correction priority'
assert_json "${raw}/outputs/fia3-more-relevant.stdout" \
  '.matchKind == "exact"' 'FIA-3 competing rule is demonstrably more relevant'
assert_no_prediction "${raw}/outputs/fia3-other-scope.stdout" \
  'FIA-3 correction did not cross project scope'
assert_no_prediction "${raw}/outputs/fia3-unrelated-decision.stdout" \
  'FIA-3 correction did not outrank relevance'
fia3_decision_id="$(jq -r '.decisionId' "${raw}/outputs/fia3-source-gate.stdout")"
ledger="${work}/fia3-vault/90-calibration/decision-ledger.jsonl"
jq -e -s --arg id "${fia3_decision_id}" '
  any(.[]; .id == $id and .kind == "decision-gate")
  and any(.[]; .decisionId == $id and .kind == "record-decision"
                  and .chosen == "pnpm" and .wasAsked == true)
  and any(.[]; .decisionId == $id and .kind == "learn-feedback"
                  and .classification == "corrected-decision"
                  and .chosen == "pnpm" and .rejected == ["npm"])
' "${ledger}" >/dev/null ||
  die "FIA-3 ledger does not preserve decision, independent action, and correction"
corrected_packet_count=0
for packet in "${work}"/fia3-vault/99-meta/pending-update-packets/*.applied.json; do
  [[ -e "${packet}" ]] || continue
  if jq -e '
    .classification == "corrected-decision"
    and .decisionRule.chosen == "pnpm"
    and .decisionRule.rejected == ["npm"]
    and .decisionRule.scope == "project:fia3"
  ' "${packet}" >/dev/null; then
    corrected_packet_count=$((corrected_packet_count + 1))
  fi
done
[[ "${corrected_packet_count}" -eq 1 ]] ||
  die "FIA-3 did not persist exactly one scoped corrected packet"
before_rule="$(jq -r '.ruleId' "${raw}/outputs/fia3-before-approval.stdout")"
after_rule="$(jq -r '.ruleId' "${raw}/outputs/fia3-after-approval.stdout")"
more_relevant_rule="$(jq -r '.ruleId' "${raw}/outputs/fia3-more-relevant.stdout")"
[[ -n "${before_rule}" && "${before_rule}" != null &&
   -n "${after_rule}" && "${after_rule}" != null &&
   "${before_rule}" != "${after_rule}" ]] ||
  die "FIA-3 correction did not supersede the baseline rule"
[[ -n "${more_relevant_rule}" && "${more_relevant_rule}" != null &&
   "${more_relevant_rule}" != "${after_rule}" ]] ||
  die "FIA-3 correction improperly outranked the more-relevant competing rule"
jq -n --arg decisionId "${fia3_decision_id}" --arg beforeRule "${before_rule}" \
  --arg afterRule "${after_rule}" --arg moreRelevantRule "${more_relevant_rule}" \
  '{nonDryDecision: true, actionRecorded: true, previewed: true, approved: true,
    beforeChoice: "npm", afterChoice: "pnpm", scopeIsolation: true,
    relevanceIsolation: true, moreRelevantCompetingChoice: "npm",
    moreRelevantCompetingRuleWins: true, decisionId: $decisionId,
    beforeRule: $beforeRule, afterRule: $afterRule,
    moreRelevantRule: $moreRelevantRule}' \
  >"${raw}/reports/fia3.json"

# FIA-4 outer assertions.
assert_no_prediction "${raw}/outputs/fia4-before.stdout" \
  'FIA-4 no behavior before policy rebuild'
assert_json "${raw}/outputs/fia4-before.stdout" \
  '.appliedPolicies == []' 'FIA-4 before state has exactly zero causal policies'
assert_shadow_choice "${raw}/outputs/fia4-active.stdout" 'cargo nextest' \
  'FIA-4 active policy prediction'
assert_json "${raw}/outputs/fia4-active.stdout" \
  '.ruleId == "fia4-policy"
   and .appliedPolicies == ["[[20-decision-frames/fia4-policy.md]]"]' \
  'FIA-4 appliedPolicies contains exactly the one causal policy'
assert_no_prediction "${raw}/outputs/fia4-unrelated.stdout" \
  'FIA-4 unrelated request has no policy behavior'
assert_json "${raw}/outputs/fia4-unrelated.stdout" \
  '.appliedPolicies == []' \
  'FIA-4 unrelated appliedPolicies contains exactly zero policies'
assert_no_prediction "${raw}/outputs/fia4-retired.stdout" \
  'FIA-4 retired policy has no behavior'
assert_json "${raw}/outputs/fia4-retired.stdout" \
  '.appliedPolicies == []' \
  'FIA-4 retired appliedPolicies contains exactly zero policies'
jq -n '{added: true, rebuiltActive: true, activePrediction: "cargo nextest",
        activeDecoyPolicy: true, exactCausalPolicySet: true,
        causallyNamed: true, unrelatedNotNamed: true, retired: true,
        rebuiltRetired: true, retiredNotApplied: true}' \
  >"${raw}/reports/fia4.json"

# FIA-6 outer assertions.
commands="${raw}/commands.json"
jq -e '
  ([.[] | select(.id | startswith("fia6-gate-"))] | length) == 16
  and ([.[] | select(.id | startswith("fia6-record-"))] | length) == 16
  and ([.[] | select(.id | startswith("fia6-capture-"))] | length) == 16
  and ([.[] | select(.id | startswith("fia6-feedback-"))] | length) == 16
  and any(.[]; .id == "fia6-overlap-gate-record" and .passed)
  and any(.[]; .id == "fia6-overlap-capture-feedback" and .passed)
  and all(.[] | select(.id | startswith("fia6-")); .passed)
' "${commands}" >/dev/null || die "FIA-6 did not complete all 64 OS processes"
fia6_ledger="${work}/fia6-vault/90-calibration/decision-ledger.jsonl"
fia6_capture="${work}/fia6-vault/.brainmap/capture-queue.jsonl"
jq -e -s '
  length == 48
  and ([.[].id] | unique | length == 48)
  and ([.[] | select(.kind == "decision-gate")] | length == 16)
  and ([.[] | select(.kind == "record-decision"
                     and .chosen == "biome" and .wasAsked == true)] | length == 16)
  and ([.[] | select(.kind == "learn-feedback"
                     and .classification == "corrected-decision")] | length == 16)
' "${fia6_ledger}" >/dev/null ||
  die "FIA-6 decision ledger is incomplete, invalid, or contains duplicate IDs"
jq -e -s '
  length == 16
  and ([.[].id] | unique | length == 16)
  and all(.[]; .source == "m8-fia6" and .sensitivity != "secret")
' "${fia6_capture}" >/dev/null ||
  die "FIA-6 capture JSONL is incomplete, invalid, or contains duplicate IDs"
[[ "$(tail -c 1 "${fia6_ledger}" | od -An -t x1 | tr -d ' \n')" == 0a ]] ||
  die "FIA-6 decision ledger has a truncated final line"
[[ "$(tail -c 1 "${fia6_capture}" | od -An -t x1 | tr -d ' \n')" == 0a ]] ||
  die "FIA-6 capture queue has a truncated final line"

applied_count=0
note_count=0
packet_ids="${work}/fia6-packet-ids.txt"
: >"${packet_ids}"
for packet in "${work}"/fia6-vault/99-meta/pending-update-packets/*.applied.json; do
  [[ -e "${packet}" ]] || continue
  jq -e '
    .classification == "corrected-decision"
    and .decisionRule.chosen == "biome"
    and .decisionRule.rejected == ["prettier"]
    and .decisionRule.scope == "project:fia6"
  ' "${packet}" >/dev/null || die "FIA-6 contains a malformed applied packet"
  packet_id="$(jq -r '.id' "${packet}")"
  printf '%s\n' "${packet_id}" >>"${packet_ids}"
  note="${work}/fia6-vault/60-decision-examples/${packet_id}.md"
  [[ -s "${note}" ]] || die "FIA-6 lost canonical Markdown note ${packet_id}"
  [[ "$(grep -c 'brainmap-decision-rule:v1' "${note}")" -eq 1 ]] ||
    die "FIA-6 canonical Markdown note is truncated or ambiguous"
  applied_count=$((applied_count + 1))
  note_count=$((note_count + 1))
done
[[ "${applied_count}" -eq 16 && "${note_count}" -eq 16 ]] ||
  die "FIA-6 expected 16 applied packets and 16 canonical notes"
[[ "$(sort -u "${packet_ids}" | wc -l)" -eq 16 ]] ||
  die "FIA-6 applied packet IDs are not unique"
pending_count="$(find "${work}/fia6-vault/99-meta/pending-update-packets" \
  -maxdepth 1 -type f -name '*.json' ! -name '*.applied.json' | wc -l)"
[[ "${pending_count}" -eq 0 ]] || die "FIA-6 lost a pending packet during apply"
ledger_sha="$(sha256sum "${fia6_ledger}" | cut -d ' ' -f 1)"
capture_sha="$(sha256sum "${fia6_capture}" | cut -d ' ' -f 1)"
jq -n --arg ledgerSha256 "${ledger_sha}" --arg captureSha256 "${capture_sha}" \
  '{operatingSystemProcesses: 64, gateProcesses: 16, recordProcesses: 16,
    captureProcesses: 16, feedbackProcesses: 16, ledgerEvents: 48,
    uniqueLedgerIds: 48, captureEvents: 16, uniqueCaptureIds: 16,
    appliedPackets: 16, canonicalNotes: 16, pendingPackets: 0,
    gateRecordOverlapBarrier: true, captureFeedbackOverlapBarrier: true,
    simultaneousGateRecordWorkers: 32, simultaneousCaptureFeedbackWorkers: 32,
    jsonlComplete: true, notesComplete: true,
    ledgerSha256: $ledgerSha256, captureSha256: $captureSha256}' \
  >"${raw}/reports/fia6.json"

fia7_passed=false
if [[ "${include_fia7}" == true ]]; then
  declare -A fia7_choices=(
    [learned]=biome
    [corrected]=pnpm
    [policy]='cargo nextest'
  )
  for behavior in learned corrected policy; do
    assert_shadow_choice "${raw}/outputs/fia7-source-${behavior}-gate.stdout" \
      "${fia7_choices[${behavior}]}" "FIA-7 source ${behavior} behavior"
    assert_shadow_choice "${raw}/outputs/fia7-restored-${behavior}-gate.stdout" \
      "${fia7_choices[${behavior}]}" "FIA-7 restored ${behavior} behavior"
    jq -S '{
      predictedOutcome, predictedSelectedOption, ruleId, ruleScope,
      matchKind, appliedPolicies, restrictionsApplied
    }' "${raw}/outputs/fia7-source-${behavior}-gate.stdout" \
      >"${work}/fia7-source-${behavior}-normalized.json"
    jq -S '{
      predictedOutcome, predictedSelectedOption, ruleId, ruleScope,
      matchKind, appliedPolicies, restrictionsApplied
    }' "${raw}/outputs/fia7-restored-${behavior}-gate.stdout" \
      >"${work}/fia7-restored-${behavior}-normalized.json"
    cmp -s "${work}/fia7-source-${behavior}-normalized.json" \
      "${work}/fia7-restored-${behavior}-normalized.json" ||
      die "FIA-7 restored ${behavior} behavior differs from source behavior"
  done
  assert_json "${raw}/outputs/fia7-source-policy-gate.stdout" \
    '.appliedPolicies == ["[[20-decision-frames/fia7-policy.md]]"]' \
    'FIA-7 source policy behavior is causally attributable'
  assert_json "${raw}/outputs/fia7-restored-policy-gate.stdout" \
    '.appliedPolicies == ["[[20-decision-frames/fia7-policy.md]]"]' \
    'FIA-7 restored policy behavior is causally attributable'
  jq -e -s '
    length == 8
    and ([.[].phase] | unique | length == 8)
    and all(.[]; .completeState == "old" or .completeState == "new")
  ' "${raw}/reports/fia7-faults.jsonl" >/dev/null ||
    die "FIA-7 did not leave eight canonical fault states"
  while IFS=$'\t' read -r phase state; do
    expected=prettier
    [[ "${state}" != new ]] || expected=biome
    assert_shadow_choice "${raw}/outputs/fia7-fault-${phase}-gate.stdout" \
      "${expected}" "FIA-7 usable ${phase} fault state"
  done < <(jq -r '[.phase, .completeState] | @tsv' "${raw}/reports/fia7-faults.jsonl")
  assert_json "${raw}/reports/fia7-inner.json" \
    '.exportVerified == true and .behaviorPairs == 3
     and .faultPhases == 8 and .canonicalFaultStates == 8
     and .oldTreeHash != .newTreeHash' \
    'FIA-7 export and fault baselines'
  jq '. + {
    behaviorEquivalent: true,
    learnedEquivalent: true,
    correctedEquivalent: true,
    policyEquivalent: true
  }' "${raw}/reports/fia7-inner.json" \
    >"${raw}/reports/fia7.json"
  rm "${raw}/reports/fia7-inner.json"
  fia7_passed=true
fi

completed_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
inner_kernel_name="$(awk -F '\t' '$1 == "kernelName" {print $2}' "${raw}/environment.tsv")"
inner_kernel_release="$(awk -F '\t' '$1 == "kernelRelease" {print $2}' "${raw}/environment.tsv")"
inner_architecture="$(awk -F '\t' '$1 == "architecture" {print $2}' "${raw}/environment.tsv")"
if [[ "${fia7_passed}" != true ]]; then
  jq -n '{qualificationEligible: false, notRun: true}' \
    >"${raw}/reports/fia7.json"
fi

fia7_status='not run'
[[ "${fia7_passed}" != true ]] || fia7_status='included and passed'
if [[ "${qualifying_run}" == true ]]; then
  cat >"${raw}/README.md" <<EOF
# Brainmap M8 integrated FIA evidence

Candidate commit: \`${candidate_commit}\`

This dated, synthetic-only bundle proves FIA-1 through FIA-4 and FIA-6 against
the exact optimized binary hashes in \`runner-manifest.json\`. FIA-7 is
${fia7_status}.
FIA-5, FIA-8, and dogfood are outside this runner and are not claimed.

Commands and exit codes are in \`commands.json\`; compact assertion results are
under \`reports/\`; \`SHA256SUMS\` covers every retained artifact except itself.
No vault, private path, secret, or real prompt is retained.
EOF
else
  cat >"${raw}/README.md" <<EOF
# Brainmap M8 local diagnostic evidence

Candidate commit: \`${candidate_commit}\`

This dated, synthetic-only bundle records successful local diagnostic drills
for the FIA-1 through FIA-4 and FIA-6 scenarios. FIA-7 is ${fia7_status}.
It is **non-qualifying** because local mode does not prove the clean-container
or strict release-provenance contract. Its runner manifest is explicitly
non-qualifying and cannot be assembled into a dogfood qualification bundle.

Commands and exit codes are in \`commands.json\`; compact assertion results are
under \`reports/\`; \`SHA256SUMS\` covers every retained artifact except itself.
No vault, private path, secret, or real prompt is retained.
EOF
fi

rm -rf "${raw}/outputs"
rm "${raw}/environment.tsv"
while IFS= read -r -d '' file; do
  sanitize_one_file "${file}" \
    "${staging}" '<qualification-staging>' \
    "${work}" '<qualification-work>' \
    "${root}" '<source-root>' \
    "${brainmap}" '<brainmap>' \
    "${brainmapd}" '<brainmapd>' \
    "${HOME:-}" '<host-home>'
done < <(find "${raw}" -type f -print0)

commands_sha256="$(sha256sum "${raw}/commands.json" | cut -d ' ' -f 1)"
fia1_sha256="$(sha256sum "${raw}/reports/fia1.json" | cut -d ' ' -f 1)"
fia2_sha256="$(sha256sum "${raw}/reports/fia2.json" | cut -d ' ' -f 1)"
fia3_sha256="$(sha256sum "${raw}/reports/fia3.json" | cut -d ' ' -f 1)"
fia4_sha256="$(sha256sum "${raw}/reports/fia4.json" | cut -d ' ' -f 1)"
fia6_sha256="$(sha256sum "${raw}/reports/fia6.json" | cut -d ' ' -f 1)"
fia7_sha256="$(sha256sum "${raw}/reports/fia7.json" | cut -d ' ' -f 1)"
reproducibility_sha256="$(printf '%064d' 0)"
if [[ "${release_provenance_verified}" == true ]]; then
  reproducibility_sha256="$(sha256sum \
    "${raw}/release-reproducibility-manifest.json" | cut -d ' ' -f 1)"
fi

jq -n \
  --arg schemaVersion brainmap-m8-runner-v2 \
  --arg candidateCommit "${candidate_commit}" \
  --arg startedAt "${started_at}" \
  --arg completedAt "${completed_at}" \
  --arg mode "${mode}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg hostKernelName "$(uname -s)" \
  --arg hostKernelRelease "$(uname -r)" \
  --arg hostArchitecture "$(uname -m)" \
  --arg innerKernelName "${inner_kernel_name}" \
  --arg innerKernelRelease "${inner_kernel_release}" \
  --arg innerArchitecture "${inner_architecture}" \
  --arg dockerImage "${docker_image}" \
  --arg dockerImageId "${docker_image_id:-sha256:$(printf '%064d' 0)}" \
  --arg reproducibilitySha256 "${reproducibility_sha256}" \
  --arg commandsSha256 "${commands_sha256}" \
  --arg fia1Sha256 "${fia1_sha256}" \
  --arg fia2Sha256 "${fia2_sha256}" \
  --arg fia3Sha256 "${fia3_sha256}" \
  --arg fia4Sha256 "${fia4_sha256}" \
  --arg fia6Sha256 "${fia6_sha256}" \
  --arg fia7Sha256 "${fia7_sha256}" \
  --argjson qualifying "${qualifying_run}" \
  --argjson releaseProvenanceVerified "${release_provenance_verified}" \
  '{
    schemaVersion: $schemaVersion,
    qualificationEligible: $qualifying,
    result: (if $qualifying then "passed" else "diagnostic-passed" end),
    candidate: {
      commit: $candidateCommit,
      brainmapSha256: $brainmapSha256,
      brainmapdSha256: $brainmapdSha256
    },
    startedAt: $startedAt,
    completedAt: $completedAt,
    executionMode: $mode,
    provenance: {
      host: {
        kernelName: $hostKernelName,
        kernelRelease: $hostKernelRelease,
        architecture: $hostArchitecture
      },
      qualificationEnvironment: {
        kernelName: $innerKernelName,
        kernelRelease: $innerKernelRelease,
        architecture: $innerArchitecture
      },
      container: (if $qualifying then {
        image: $dockerImage,
        imageId: $dockerImageId,
        network: "none",
        rootFilesystem: "read-only",
        capabilities: "dropped",
        noNewPrivileges: true
      } else {
        image: "diagnostic-local",
        imageId: $dockerImageId,
        network: "host",
        rootFilesystem: "host",
        capabilities: "host",
        noNewPrivileges: false
      } end)
    },
    build: {
      profile: (if $releaseProvenanceVerified then "release" else "diagnostic" end),
      locked: $releaseProvenanceVerified,
      twoRootByteIdentical: $releaseProvenanceVerified,
      reproducibilityManifestSha256: $reproducibilitySha256
    },
    commands: {path: "commands.json", sha256: $commandsSha256},
    reports: {
      fia1: {path: "reports/fia1.json", sha256: $fia1Sha256},
      fia2: {path: "reports/fia2.json", sha256: $fia2Sha256},
      fia3: {path: "reports/fia3.json", sha256: $fia3Sha256},
      fia4: {path: "reports/fia4.json", sha256: $fia4Sha256},
      fia6: {path: "reports/fia6.json", sha256: $fia6Sha256},
      fia7: {path: "reports/fia7.json", sha256: $fia7Sha256}
    },
    privacy: {
      rawPromptsRetained: false,
      secretsRetained: false,
      privatePathsRetained: false,
      syntheticInputsOnly: true
    }
  }' >"${raw}/runner-manifest.json"

if grep -RIEi \
  '(/home/|/Users/|/tmp/|/opt/|/root/|/var/folders/|[A-Za-z]:\\Users\\)' \
  "${raw}" >/dev/null; then
  die "retained evidence contains an absolute private path"
fi
if grep -RIEi \
  '(AKIA[0-9A-Z]{16}|api[_-]?key[[:space:]]*[:=]|authorization[[:space:]]*[:=]|cookie[[:space:]]*[:=]|-----BEGIN [A-Z ]*PRIVATE KEY-----|(^|[^[:alnum:]])sk-[A-Za-z0-9_-]{8,}|[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,})' \
  "${raw}" >/dev/null; then
  die "retained evidence contains secret-like material"
fi
if grep -RIEi \
  '"(prompt|messages|transcript|situation|options|toolArguments)"[[:space:]]*:' \
  "${raw}" >/dev/null; then
  die "retained evidence contains raw prompt or transcript fields"
fi
while IFS= read -r -d '' retained_file; do
  iconv -f UTF-8 -t UTF-8 "${retained_file}" >/dev/null ||
    die "retained evidence is not valid UTF-8: $(basename "${retained_file}")"
done < <(find "${raw}" -type f -print0)
while IFS= read -r -d '' json_file; do
  jq -e . "${json_file}" >/dev/null ||
    die "retained JSON is invalid: $(basename "${json_file}")"
done < <(find "${raw}" -type f -name '*.json' -print0)
if [[ "${fia7_passed}" == true ]]; then
  jq -e -s . "${raw}/reports/fia7-faults.jsonl" >/dev/null
fi

checksum_staging="${staging}/SHA256SUMS"
(
  cd "${raw}"
  find . -type f ! -name SHA256SUMS -print0 |
    sort -z |
    while IFS= read -r -d '' artifact; do
      relative="${artifact#./}"
      printf '%s  %s\n' "$(sha256sum "${relative}" | cut -d ' ' -f 1)" "${relative}"
    done
) >"${checksum_staging}"
mv "${checksum_staging}" "${raw}/SHA256SUMS"
(cd "${raw}" && sha256sum -c SHA256SUMS >/dev/null)

assert_outer_identity
sync -f "${raw}"
[[ ! -e "${out}" && ! -L "${out}" ]] ||
  die "evidence output appeared during qualification: ${out}"
mv -T -n "${raw}" "${out}"
[[ ! -e "${raw}" ]] ||
  die "evidence output appeared during atomic publication: ${out}"
sync -f "${out}"
sync -f "${out_parent}"
trap - EXIT
rm -rf "${staging}"
if [[ "${qualifying_run}" == true ]]; then
  printf 'M8 integrated FIA qualification passed; evidence: %s\n' "${out}"
else
  printf 'M8 local diagnostic passed (non-qualifying); evidence: %s\n' "${out}"
fi
