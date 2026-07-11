#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

usage() {
  cat <<'EOF'
Assemble one strict, verifier-approved Brainmap M8 qualification bundle.

Usage:
  scripts/m8-assemble-qualification.sh \
    --brainmap PATH --brainmap-sha256 SHA256 \
    --brainmapd-sha256 SHA256 --candidate-commit COMMIT \
    --reproducibility-manifest PATH \
    --runner-evidence DIR --host-evidence DIR --release-evidence DIR \
    --out DIR

Required candidate:
  --brainmap PATH                 Canonical absolute exact brainmap executable
  --brainmap-sha256 SHA256        SHA-256 of that exact executable
  --brainmapd-sha256 SHA256       SHA-256 of the exact candidate brainmapd
  --candidate-commit COMMIT       Full 40-character clean HEAD commit

Required evidence:
  --reproducibility-manifest PATH Strict two-root reproducibility manifest
  --runner-evidence DIR           Qualifying FIA-1-4/6-7 runner evidence
  --host-evidence DIR             Qualifying real-host FIA-5 evidence
  --release-evidence DIR          Qualifying FIA-8 release evidence
  --out DIR                       Canonical absolute new output directory

All inputs must be immutable-looking, symlink-free, checksummed evidence for
one exact candidate. This command has no diagnostic, local, or fake mode.
EOF
}

die() {
  echo "m8 qualification assembler: $*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

require_value() {
  [[ -n "${2:-}" ]] || die "$1 requires a value"
}

sha256_file() {
  sha256sum "$1" | cut -d ' ' -f 1
}

canonical_file_path() {
  local path="$1" parent base
  parent="$(dirname "${path}")"
  base="$(basename "${path}")"
  parent="$(cd "${parent}" 2>/dev/null && pwd -P)" || return 1
  printf '%s/%s\n' "${parent}" "${base}"
}

canonical_directory_path() {
  (cd "$1" 2>/dev/null && pwd -P)
}

paths_overlap() {
  local left="$1" right="$2"
  [[ "${left}" == "${right}" || "${left}" == "${right}/"* || "${right}" == "${left}/"* ]]
}

validate_relative_path() {
  local relative="$1" component
  local -a components
  [[ -n "${relative}" && ${#relative} -le 240 ]] || return 1
  [[ "${relative}" != /* && "${relative}" != */ && "${relative}" != *\\* ]] || return 1
  IFS='/' read -r -a components <<<"${relative}"
  for component in "${components[@]}"; do
    [[ -n "${component}" && "${component}" != . && "${component}" != .. ]] || return 1
    [[ "${component}" =~ ^[A-Za-z0-9._-]+$ ]] || return 1
  done
}

validate_evidence_tree() {
  local label="$1" directory="$2" entry relative invalid
  invalid="$(find "${directory}" -mindepth 1 -type l -print -quit)"
  [[ -z "${invalid}" ]] || die "${label} evidence contains a symlink: ${invalid}"
  invalid="$(find "${directory}" -mindepth 1 ! -type f ! -type d -print -quit)"
  [[ -z "${invalid}" ]] || die "${label} evidence contains a non-regular entry: ${invalid}"
  while IFS= read -r -d '' entry; do
    relative="${entry#"${directory}/"}"
    validate_relative_path "${relative}" ||
      die "${label} evidence contains a non-portable path: ${relative}"
  done < <(find "${directory}" -mindepth 1 -print0)
}

write_tree_checksums() {
  local directory="$1" destination="$2" artifact relative
  (
    cd "${directory}"
    find . -type f ! -path './SHA256SUMS' -print0 |
      sort -z |
      while IFS= read -r -d '' artifact; do
        relative="${artifact#./}"
        printf '%s  %s\n' "$(sha256_file "${relative}")" "${relative}"
      done
  ) >"${destination}"
}

validate_checksum_tree() {
  local label="$1" directory="$2" generated
  [[ -f "${directory}/SHA256SUMS" && ! -L "${directory}/SHA256SUMS" ]] ||
    die "${label} evidence is missing regular SHA256SUMS"
  generated="$(mktemp "${scratch}/checksums.XXXXXX")"
  write_tree_checksums "${directory}" "${generated}"
  cmp -s "${generated}" "${directory}/SHA256SUMS" || {
    rm -f "${generated}"
    die "${label} SHA256SUMS is malformed, unsorted, incomplete, or stale"
  }
  rm -f "${generated}"
  (cd "${directory}" && sha256sum -c SHA256SUMS >/dev/null) ||
    die "${label} SHA256SUMS failed verification"
}

validate_candidate_manifest() {
  local label="$1" manifest="$2" additional_filter="$3"
  jq -e \
    --arg commit "${candidate_commit}" \
    --arg brainmapSha256 "${brainmap_sha256}" \
    --arg brainmapdSha256 "${brainmapd_sha256}" \
    --arg reproducibilitySha256 "${reproducibility_sha256}" \
    "
      type == \"object\"
      and .candidate == {
        commit: \$commit,
        brainmapSha256: \$brainmapSha256,
        brainmapdSha256: \$brainmapdSha256
      }
      and (${additional_filter})
    " "${manifest}" >/dev/null ||
    die "${label} manifest does not match the qualifying candidate contract"
}

brainmap=
brainmap_sha256=
brainmapd_sha256=
candidate_commit=
reproducibility_manifest=
runner_evidence=
host_evidence=
release_evidence=
out=

while (($#)); do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --brainmap|--brainmap-sha256|--brainmapd-sha256|--candidate-commit|--reproducibility-manifest|--runner-evidence|--host-evidence|--release-evidence|--out)
      option="$1"
      require_value "${option}" "${2:-}"
      case "${option}" in
        --brainmap) brainmap="$2" ;;
        --brainmap-sha256) brainmap_sha256="$2" ;;
        --brainmapd-sha256) brainmapd_sha256="$2" ;;
        --candidate-commit) candidate_commit="$2" ;;
        --reproducibility-manifest) reproducibility_manifest="$2" ;;
        --runner-evidence) runner_evidence="$2" ;;
        --host-evidence) host_evidence="$2" ;;
        --release-evidence) release_evidence="$2" ;;
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
[[ -n "${brainmapd_sha256}" ]] || die "missing required --brainmapd-sha256 SHA256"
[[ -n "${candidate_commit}" ]] || die "missing required --candidate-commit COMMIT"
[[ -n "${reproducibility_manifest}" ]] ||
  die "missing required --reproducibility-manifest PATH"
[[ -n "${runner_evidence}" ]] || die "missing required --runner-evidence DIR"
[[ -n "${host_evidence}" ]] || die "missing required --host-evidence DIR"
[[ -n "${release_evidence}" ]] || die "missing required --release-evidence DIR"
[[ -n "${out}" ]] || die "missing required --out DIR"

for command in basename cmp cp cut dirname find git jq mktemp mv sha256sum sort sync; do
  require_command "${command}"
done

[[ "${brainmap}" == /* ]] || die "brainmap path must be absolute"
[[ -f "${brainmap}" && -x "${brainmap}" && ! -L "${brainmap}" ]] ||
  die "brainmap is not a symlink-free executable regular file: ${brainmap}"
[[ "${brainmap}" == "$(canonical_file_path "${brainmap}")" ]] ||
  die "brainmap path must be canonical"
[[ "${brainmap_sha256}" =~ ^[0-9a-f]{64}$ ]] ||
  die "brainmap SHA-256 must be exactly 64 lowercase hexadecimal characters"
[[ "${brainmapd_sha256}" =~ ^[0-9a-f]{64}$ ]] ||
  die "brainmapd SHA-256 must be exactly 64 lowercase hexadecimal characters"
actual_brainmap_sha256="$(sha256_file "${brainmap}")"
[[ "${actual_brainmap_sha256}" == "${brainmap_sha256}" ]] ||
  die "brainmap SHA-256 mismatch: expected ${brainmap_sha256}, got ${actual_brainmap_sha256}"

[[ "${candidate_commit}" =~ ^[0-9a-f]{40}$ ]] ||
  die "candidate commit must be exactly 40 lowercase hexadecimal characters"
git -C "${root}" cat-file -e "${candidate_commit}^{commit}" 2>/dev/null ||
  die "candidate commit does not resolve in this repository"
head_commit="$(git -C "${root}" rev-parse HEAD)"
[[ "${candidate_commit}" == "${head_commit}" ]] ||
  die "candidate commit must equal unchanged HEAD"
[[ -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
  die "qualification assembly requires clean HEAD"

[[ "${reproducibility_manifest}" == /* ]] ||
  die "reproducibility manifest path must be absolute"
[[ -f "${reproducibility_manifest}" && ! -L "${reproducibility_manifest}" ]] ||
  die "reproducibility manifest is not a symlink-free regular file: ${reproducibility_manifest}"
[[ "${reproducibility_manifest}" == "$(canonical_file_path "${reproducibility_manifest}")" ]] ||
  die "reproducibility manifest path must be canonical"

for label in runner host release; do
  case "${label}" in
    runner) evidence="${runner_evidence}" ;;
    host) evidence="${host_evidence}" ;;
    release) evidence="${release_evidence}" ;;
  esac
  [[ "${evidence}" == /* ]] || die "${label} evidence path must be absolute"
  [[ -d "${evidence}" && ! -L "${evidence}" ]] ||
    die "${label} evidence is not a symlink-free directory: ${evidence}"
  [[ "${evidence}" == "$(canonical_directory_path "${evidence}")" ]] ||
    die "${label} evidence path must be canonical"
done

[[ ! "${runner_evidence}" -ef "${host_evidence}" &&
   ! "${runner_evidence}" -ef "${release_evidence}" &&
   ! "${host_evidence}" -ef "${release_evidence}" ]] ||
  die "runner, host, and release evidence directories must be distinct"
for pair in \
  "${runner_evidence}|${host_evidence}" \
  "${runner_evidence}|${release_evidence}" \
  "${host_evidence}|${release_evidence}"; do
  left="${pair%%|*}"
  right="${pair#*|}"
  paths_overlap "${left}" "${right}" &&
    die "evidence input directories overlap: ${left} and ${right}"
done

[[ "${out}" == /* ]] || die "qualification output path must be absolute"
[[ ! -e "${out}" && ! -L "${out}" ]] ||
  die "qualification output already exists: ${out}"
out_parent="$(dirname "${out}")"
[[ -d "${out_parent}" && ! -L "${out_parent}" ]] ||
  die "qualification output parent is not a symlink-free directory: ${out_parent}"
out_parent="$(canonical_directory_path "${out_parent}")"
canonical_out="${out_parent}/$(basename "${out}")"
[[ "${out}" == "${canonical_out}" ]] || die "qualification output path must be canonical"
paths_overlap "${out}" "${root}" &&
  die "qualification output must be outside the repository"
for input in \
  "${brainmap}" \
  "${reproducibility_manifest}" \
  "${runner_evidence}" \
  "${host_evidence}" \
  "${release_evidence}"; do
  paths_overlap "${out}" "${input}" &&
    die "qualification output overlaps an input: ${input}"
done

container="$(mktemp -d "${out_parent}/.brainmap-m8-qualification.XXXXXX")"
bundle="${container}/bundle"
scratch="${container}/scratch"
verification="${container}/verification.json"
cleanup() {
  rm -rf "${container:-}"
}
trap cleanup EXIT HUP INT TERM
mkdir -p "${bundle}/reproducibility" "${bundle}/runner" \
  "${bundle}/host" "${bundle}/release" "${scratch}"

validate_evidence_tree runner "${runner_evidence}"
validate_evidence_tree host "${host_evidence}"
validate_evidence_tree release "${release_evidence}"
validate_checksum_tree runner "${runner_evidence}"
validate_checksum_tree host "${host_evidence}"
validate_checksum_tree release "${release_evidence}"

brainmap_build_info="$("${brainmap}" build-info)" ||
  die "brainmap did not expose embedded build provenance"
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
  die "candidate brainmap does not contain qualifying embedded build provenance"
build_info_sha256="$(printf '%s' "${brainmap_build_info}" | sha256sum | cut -d ' ' -f 1)"
producer_digests="$(jq -c '.producerDigests' <<<"${brainmap_build_info}")"
assembler_producer_sha256="$(sha256_file "${root}/scripts/m8-assemble-qualification.sh")"
jq -e \
  --arg candidateCommit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg buildInfoSha256 "${build_info_sha256}" \
  --arg assemblerProducerSha256 "${assembler_producer_sha256}" \
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
  and .producerDigests.assembleQualificationSha256 == $assemblerProducerSha256
' "${reproducibility_manifest}" >/dev/null ||
  die "invalid strict reproducibility manifest"
reproducibility_sha256="$(sha256_file "${reproducibility_manifest}")"

runner_manifest="${runner_evidence}/manifest.json"
if [[ ! -e "${runner_manifest}" ]]; then
  runner_manifest="${runner_evidence}/runner-manifest.json"
fi
[[ -f "${runner_manifest}" && ! -L "${runner_manifest}" ]] ||
  die "runner evidence is missing manifest.json or runner-manifest.json"
if [[ -f "${runner_evidence}/manifest.json" && -f "${runner_evidence}/runner-manifest.json" ]]; then
  cmp -s "${runner_evidence}/manifest.json" "${runner_evidence}/runner-manifest.json" ||
    die "runner evidence contains conflicting manifest aliases"
fi
[[ -f "${host_evidence}/manifest.json" && ! -L "${host_evidence}/manifest.json" ]] ||
  die "host evidence is missing regular manifest.json"
[[ -f "${release_evidence}/manifest.json" && ! -L "${release_evidence}/manifest.json" ]] ||
  die "release evidence is missing regular manifest.json"

# The dollar-prefixed names below are jq variables, not shell variables.
# shellcheck disable=SC2016
validate_candidate_manifest runner "${runner_manifest}" '
  .schemaVersion == "brainmap-m8-runner-v2"
  and .qualificationEligible == true
  and .result == "passed"
  and .executionMode == "docker"
  and .build.profile == "release"
  and .build.locked == true
  and .build.twoRootByteIdentical == true
  and .build.reproducibilityManifestSha256 == $reproducibilitySha256
'
# shellcheck disable=SC2016
validate_candidate_manifest host "${host_evidence}/manifest.json" '
  .schemaVersion == "brainmap-m8-host-v2"
  and .qualificationEligible == true
  and .mode == "qualification"
  and .adapter.target == "codex"
  and .adapter.hostVersion == "codex-cli 0.144.0"
  and .adapter.launchMode == "normal"
  and .adapter.trustBypassUsed == false
  and .adapter.persistedHookAccepted == true
  and .adapter.projectTrusted == true
  and .provenance.configuredBrainmapSha256 == $brainmapSha256
  and .provenance.configuredBrainmapdSha256 == $brainmapdSha256
  and .provenance.codexTarget == "x86_64-unknown-linux-musl"
  and .provenance.officialCodexArchiveSha256 == "6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd"
  and .provenance.officialCodexBinarySha256 == "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429"
  and .provenance.observedCodexBinarySha256 == .provenance.officialCodexBinarySha256
  and .provenance.officialCodexVerified == true
  and .artifacts.hostObservation.path == "host-observation.json"
  and (.artifacts.hostObservation.sha256 | test("^[0-9a-f]{64}$"))
'
host_observation="${host_evidence}/host-observation.json"
[[ -f "${host_observation}" && ! -L "${host_observation}" ]] ||
  die "host evidence is missing regular host-observation.json"
expected_host_observation_sha="$(
  jq -er '.artifacts.hostObservation.sha256' "${host_evidence}/manifest.json"
)"
[[ "$(sha256_file "${host_observation}")" == "${expected_host_observation_sha}" ]] ||
  die "host observation checksum does not match the host manifest"
jq -e \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" '
  type == "object"
  and (keys == [
    "calls", "candidate", "config", "hooks", "launch", "ledger", "mode",
    "officialCodex", "project", "qualificationEligible", "schemaVersion"
  ])
  and .schemaVersion == "brainmap-m8-host-observation-v2"
  and .qualificationEligible == true
  and .mode == "qualification"
  and .candidate == {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  }
  and .officialCodex == {
    version: "codex-cli 0.144.0",
    target: "x86_64-unknown-linux-musl",
    archiveSha256: "6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd",
    binarySha256: "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429",
    observedBinarySha256: "901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429",
    archiveVerified: true,
    binaryVerified: true
  }
  and .config.approvalPolicy == "on-request"
  and .config.approvalsReviewer == "user"
  and .config.sandboxMode == "workspace-write"
  and .config.workspaceWriteNetworkAccess == false
  and .config.bypassHookTrust == false
  and .config.bypassApprovalsAndSandbox == false
  and .config.feedbackApprovalMode == "prompt"
  and .config.applyApprovalMode == "prompt"
  and .config.gateMode == "active"
  and .config.autopilotMode == "conservative"
  and (.config.codexHomeSha256 | test("^[0-9a-f]{64}$"))
  and .launch.codexHomeBound == true
  and .launch.projectInventoryBound == true
  and (.launch.launcherSha256 | test("^[0-9a-f]{64}$"))
  and (.launch.argvSha256 | test("^[0-9a-f]{64}$"))
  and (.launch.argv | type == "array")
  and (.launch.appServerArgvSha256 | test("^[0-9a-f]{64}$"))
  and (.launch.appServerArgv | type == "array")
  and .launch.session.source == "cli"
  and (.launch.session.idSha256 | test("^[0-9a-f]{64}$"))
  and (.launch.session.createdAt | type == "number" and . > 0 and floor == .)
  and .hooks.trustedHookCount == 2
  and (.hooks.entries | length) == 2
  and .hooks.executedHookGateCount >= 1
  and .calls.count == 7
  and .calls.order == [
    "brainmap_decision_gate", "brainmap_record_decision",
    "brainmap_learn_feedback", "brainmap_preview_update",
    "brainmap_apply_update", "brainmap_decision_gate",
    "brainmap_record_decision"
  ]
  and (.calls.first.decisionId | test("^dec_[0-9]{13}_[0-9a-f]{12}$"))
  and .calls.first.outcome == "ask_user"
  and .calls.first.selectedOption == null
  and .calls.first.action == {chosen:"biome",wasAsked:true}
  and (.calls.feedback.packetId | test("^upd_[0-9]{13}_[0-9a-f]{12}$"))
  and .calls.feedback.previewed == true
  and .calls.feedback.approved == true
  and (.calls.second.decisionId | test("^dec_[0-9]{13}_[0-9a-f]{12}$"))
  and .calls.second.decisionId != .calls.first.decisionId
  and .calls.second.outcome == "proceed"
  and .calls.second.selectedOption == "prettier"
  and .calls.second.changed == true
  and .calls.second.action == {chosen:"prettier",wasAsked:false}
  and .ledger.correlation == "complete"
  and .ledger.correlatedEventCount == 5
  and .ledger.postBoundaryEventCount >= 6
  and (.project.inventorySha256 | test("^[0-9a-f]{64}$"))
  and (.project.workflowSha256 | test("^[0-9a-f]{64}$"))
  and .project.unchanged == true
' "${host_observation}" >/dev/null ||
  die "host observation does not match the qualifying contract"
# shellcheck disable=SC2016
validate_candidate_manifest release "${release_evidence}/manifest.json" '
  .schemaVersion == "brainmap-m8-release-v1"
  and .qualificationEligible == true
  and .sourceTreeDirtyBefore == false
  and .sourceTreeDirtyAfter == false
  and .reproducibilityManifestSha256 == $reproducibilitySha256
'

[[ -f "${release_evidence}/reproducibility-manifest.json" &&
   ! -L "${release_evidence}/reproducibility-manifest.json" ]] ||
  die "release evidence is missing regular reproducibility-manifest.json"
release_reproducibility_sha256="$(
  sha256_file "${release_evidence}/reproducibility-manifest.json"
)"
[[ "${release_reproducibility_sha256}" == "${reproducibility_sha256}" ]] ||
  die "release evidence retained a different reproducibility manifest"
[[ -f "${runner_evidence}/release-reproducibility-manifest.json" &&
   ! -L "${runner_evidence}/release-reproducibility-manifest.json" ]] ||
  die "runner evidence is missing regular release-reproducibility-manifest.json"
runner_reproducibility_sha256="$(
  sha256_file "${runner_evidence}/release-reproducibility-manifest.json"
)"
[[ "${runner_reproducibility_sha256}" == "${reproducibility_sha256}" ]] ||
  die "runner evidence retained a different reproducibility manifest"

runner_sums_sha256="$(sha256_file "${runner_evidence}/SHA256SUMS")"
host_sums_sha256="$(sha256_file "${host_evidence}/SHA256SUMS")"
release_sums_sha256="$(sha256_file "${release_evidence}/SHA256SUMS")"

cp "${reproducibility_manifest}" "${bundle}/reproducibility/manifest.json"
cp -R "${runner_evidence}/." "${bundle}/runner/"
cp -R "${host_evidence}/." "${bundle}/host/"
cp -R "${release_evidence}/." "${bundle}/release/"

if [[ ! -f "${bundle}/runner/manifest.json" ]]; then
  [[ ! -e "${bundle}/runner/producer-SHA256SUMS" ]] ||
    die "runner evidence uses reserved producer-SHA256SUMS path"
  mv "${bundle}/runner/SHA256SUMS" "${bundle}/runner/producer-SHA256SUMS"
  cp "${bundle}/runner/runner-manifest.json" "${bundle}/runner/manifest.json"
  write_tree_checksums "${bundle}/runner" "${scratch}/runner-SHA256SUMS"
  mv "${scratch}/runner-SHA256SUMS" "${bundle}/runner/SHA256SUMS"
fi

validate_evidence_tree runner "${bundle}/runner"
validate_evidence_tree host "${bundle}/host"
validate_evidence_tree release "${bundle}/release"
validate_checksum_tree runner "${bundle}/runner"
validate_checksum_tree host "${bundle}/host"
validate_checksum_tree release "${bundle}/release"
cmp -s "${reproducibility_manifest}" "${bundle}/reproducibility/manifest.json" ||
  die "reproducibility manifest changed while copying"

repro_ref_sha="$(sha256_file "${bundle}/reproducibility/manifest.json")"
runner_manifest_sha="$(sha256_file "${bundle}/runner/manifest.json")"
runner_checksums_sha="$(sha256_file "${bundle}/runner/SHA256SUMS")"
host_manifest_sha="$(sha256_file "${bundle}/host/manifest.json")"
host_checksums_sha="$(sha256_file "${bundle}/host/SHA256SUMS")"
release_manifest_sha="$(sha256_file "${bundle}/release/manifest.json")"
release_checksums_sha="$(sha256_file "${bundle}/release/SHA256SUMS")"

jq -n \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg reproSha "${repro_ref_sha}" \
  --arg runnerManifestSha "${runner_manifest_sha}" \
  --arg runnerChecksumsSha "${runner_checksums_sha}" \
  --arg hostManifestSha "${host_manifest_sha}" \
  --arg hostChecksumsSha "${host_checksums_sha}" \
  --arg releaseManifestSha "${release_manifest_sha}" \
  --arg releaseChecksumsSha "${release_checksums_sha}" '
  {
    schemaVersion: "brainmap-m8-qualification-bundle-v1",
    candidate: {
      commit: $commit,
      brainmapSha256: $brainmapSha256,
      brainmapdSha256: $brainmapdSha256
    },
    evidence: {
      reproducibilityManifest: {
        path: "reproducibility/manifest.json", sha256: $reproSha
      },
      runnerManifest: {
        path: "runner/manifest.json", sha256: $runnerManifestSha
      },
      runnerChecksums: {
        path: "runner/SHA256SUMS", sha256: $runnerChecksumsSha
      },
      hostManifest: {
        path: "host/manifest.json", sha256: $hostManifestSha
      },
      hostChecksums: {
        path: "host/SHA256SUMS", sha256: $hostChecksumsSha
      },
      releaseManifest: {
        path: "release/manifest.json", sha256: $releaseManifestSha
      },
      releaseChecksums: {
        path: "release/SHA256SUMS", sha256: $releaseChecksumsSha
      }
    },
    privacy: {
      rawPromptsRetained: false,
      secretsRetained: false,
      privatePathsRetained: false
    }
  }
' >"${bundle}/qualification.json"

write_tree_checksums "${bundle}" "${scratch}/SHA256SUMS"
mv "${scratch}/SHA256SUMS" "${bundle}/SHA256SUMS"
validate_checksum_tree qualification "${bundle}"

[[ "$(sha256_file "${brainmap}")" == "${brainmap_sha256}" ]] ||
  die "brainmap changed during qualification assembly"
[[ "$(sha256_file "${reproducibility_manifest}")" == "${reproducibility_sha256}" ]] ||
  die "reproducibility manifest changed during qualification assembly"
[[ "$(sha256_file "${runner_evidence}/SHA256SUMS")" == "${runner_sums_sha256}" ]] ||
  die "runner evidence changed during qualification assembly"
[[ "$(sha256_file "${host_evidence}/SHA256SUMS")" == "${host_sums_sha256}" ]] ||
  die "host evidence changed during qualification assembly"
[[ "$(sha256_file "${release_evidence}/SHA256SUMS")" == "${release_sums_sha256}" ]] ||
  die "release evidence changed during qualification assembly"
[[ "$(git -C "${root}" rev-parse HEAD)" == "${candidate_commit}" &&
   -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
  die "candidate source changed during qualification assembly"

root_checksum_sha="$(sha256_file "${bundle}/SHA256SUMS")"
"${brainmap}" qualification verify --bundle "${bundle}" >"${verification}" ||
  die "exact candidate rejected the assembled qualification bundle"
jq -e \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg bundleSha256 "${root_checksum_sha}" '
  type == "object"
  and .schemaVersion == "brainmap-m8-qualification-verification-v1"
  and .verified == true
  and .candidate == {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  }
  and .fias == ["FIA-1", "FIA-2", "FIA-3", "FIA-4", "FIA-5", "FIA-6", "FIA-7", "FIA-8"]
  and .bundleSha256 == $bundleSha256
' "${verification}" >/dev/null ||
  die "exact candidate returned invalid qualification verification"
[[ "$(sha256_file "${brainmap}")" == "${brainmap_sha256}" ]] ||
  die "brainmap changed while verifying the qualification bundle"
[[ "$(git -C "${root}" rev-parse HEAD)" == "${candidate_commit}" &&
   -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
  die "candidate source changed while verifying the qualification bundle"
[[ "$(sha256_file "${bundle}/SHA256SUMS")" == "${root_checksum_sha}" ]] ||
  die "assembled qualification bundle changed during verification"
validate_checksum_tree qualification "${bundle}"

rm -rf "${scratch}"
[[ ! -e "${out}" && ! -L "${out}" ]] ||
  die "qualification output appeared during assembly: ${out}"
sync -f "${bundle}"
mv -T -n "${bundle}" "${out}"
[[ ! -e "${bundle}" ]] ||
  die "qualification output appeared during atomic publication: ${out}"
sync -f "${out}"
sync -f "${out_parent}"
rm -f "${verification}"
rmdir "${container}"
container=
trap - EXIT HUP INT TERM

printf 'M8 qualification bundle verified and published: %s\n' "${out}"
