#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
assembler="${root}/scripts/m8-assemble-qualification.sh"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

fail() {
  echo "m8 qualification assembler test failed: $*" >&2
  exit 1
}

[[ -x "${assembler}" ]] || fail "assembler is not executable"

expect_failure() {
  local expected="$1"
  shift
  local output
  if output="$("$@" 2>&1)"; then
    fail "command unexpectedly succeeded; expected: ${expected}"
  fi
  grep -F -- "${expected}" <<<"${output}" >/dev/null ||
    fail "missing error '${expected}' in: ${output}"
}

write_checksums() {
  local directory="$1" artifact relative temporary_sums
  temporary_sums="${directory}/.SHA256SUMS.new"
  (
    cd "${directory}"
    find . -type f ! -path './SHA256SUMS' ! -path './.SHA256SUMS.new' -print0 |
      sort -z |
      while IFS= read -r -d '' artifact; do
        relative="${artifact#./}"
        printf '%s  %s\n' "$(sha256sum "${relative}" | cut -d ' ' -f 1)" "${relative}"
      done
  ) >"${temporary_sums}"
  mv "${temporary_sums}" "${directory}/SHA256SUMS"
}

help="$(${assembler} --help)"
for required in \
  '--brainmap PATH' \
  '--brainmap-sha256 SHA256' \
  '--brainmapd-sha256 SHA256' \
  '--candidate-commit COMMIT' \
  '--reproducibility-manifest PATH' \
  '--runner-evidence DIR' \
  '--host-evidence DIR' \
  '--release-evidence DIR' \
  '--out DIR'; do
  grep -F -- "${required}" <<<"${help}" >/dev/null ||
    fail "help is missing ${required}"
done

grep -E -- '--(local|diagnostic|fake|test)([[:space:]]|$)' <<<"${help}" >/dev/null &&
  fail 'help exposes a non-qualifying public mode'

# The fixture is a clean Git repository. The exact candidate executable is a
# test-only verifier at the same public CLI seam as the Rust binary.
fixture="${temporary}/fixture"
mkdir -p "${fixture}/scripts" "${fixture}/bin"
cp "${assembler}" "${fixture}/scripts/m8-assemble-qualification.sh"
chmod 0755 "${fixture}/scripts/m8-assemble-qualification.sh"

cat >"${fixture}/bin/brainmap" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -eq 1 && "$1" == build-info ]]; then
  printf '%s\n' "${EXPECTED_BUILD_INFO:?}"
  exit 0
fi

[[ "$#" -eq 4 && "$1" == qualification && "$2" == verify && "$3" == --bundle ]] || {
  echo 'fixture verifier received the wrong CLI invocation' >&2
  exit 71
}
[[ "${BRAINMAP_STUB_FAIL:-0}" != 1 ]] || {
  echo 'fixture verifier rejected bundle' >&2
  exit 72
}

bundle="$4"
[[ -d "${bundle}" && ! -L "${bundle}" ]] || exit 73
(cd "${bundle}" && sha256sum -c SHA256SUMS >/dev/null)
for subtree in runner host release; do
  (cd "${bundle}/${subtree}" && sha256sum -c SHA256SUMS >/dev/null)
done

self_sha="$(sha256sum "$0" | cut -d ' ' -f 1)"
[[ "${self_sha}" == "${EXPECTED_BRAINMAP_SHA256:?}" ]] || exit 74
jq -e \
  --arg commit "${EXPECTED_CANDIDATE_COMMIT:?}" \
  --arg brainmapSha256 "${EXPECTED_BRAINMAP_SHA256}" \
  --arg brainmapdSha256 "${EXPECTED_BRAINMAPD_SHA256:?}" '
  type == "object"
  and (keys == ["candidate", "evidence", "privacy", "schemaVersion"])
  and .schemaVersion == "brainmap-m8-qualification-bundle-v1"
  and .candidate == {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  }
  and (.evidence | keys == [
    "hostChecksums", "hostManifest", "releaseChecksums", "releaseManifest",
    "reproducibilityManifest", "runnerChecksums", "runnerManifest"
  ])
  and .privacy == {
    rawPromptsRetained: false,
    secretsRetained: false,
    privatePathsRetained: false
  }
' "${bundle}/qualification.json" >/dev/null

declare -A expected_paths=(
  [reproducibilityManifest]='reproducibility/manifest.json'
  [runnerManifest]='runner/manifest.json'
  [runnerChecksums]='runner/SHA256SUMS'
  [hostManifest]='host/manifest.json'
  [hostChecksums]='host/SHA256SUMS'
  [releaseManifest]='release/manifest.json'
  [releaseChecksums]='release/SHA256SUMS'
)
for key in "${!expected_paths[@]}"; do
  expected_path="${expected_paths[${key}]}"
  referenced_path="$(
    jq -r --arg key "${key}" '.evidence[$key].path' "${bundle}/qualification.json"
  )"
  [[ "${referenced_path}" == "${expected_path}" ]] || exit 75
  actual_sha="$(sha256sum "${bundle}/${expected_path}" | cut -d ' ' -f 1)"
  referenced_sha="$(
    jq -r --arg key "${key}" '.evidence[$key].sha256' "${bundle}/qualification.json"
  )"
  [[ "${referenced_sha}" == "${actual_sha}" ]] || exit 76
done

printf '%s\n' "${bundle}" >"${BRAINMAP_STUB_MARKER:?}"
bundle_sha="$(sha256sum "${bundle}/SHA256SUMS" | cut -d ' ' -f 1)"
jq -n \
  --arg commit "${EXPECTED_CANDIDATE_COMMIT}" \
  --arg brainmapSha256 "${EXPECTED_BRAINMAP_SHA256}" \
  --arg brainmapdSha256 "${EXPECTED_BRAINMAPD_SHA256}" \
  --arg bundleSha256 "${bundle_sha}" '{
  schemaVersion: "brainmap-m8-qualification-verification-v1",
  verified: true,
  candidate: {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  },
  fias: ["FIA-1", "FIA-2", "FIA-3", "FIA-4", "FIA-5", "FIA-6", "FIA-7", "FIA-8"],
  bundleSha256: $bundleSha256
}'
EOF
chmod 0755 "${fixture}/bin/brainmap"

git -C "${fixture}" init -q
git -C "${fixture}" config user.email fixture@example.invalid
git -C "${fixture}" config user.name Fixture
git -C "${fixture}" add .
git -C "${fixture}" commit -qm 'qualification assembler fixture'

candidate_commit="$(git -C "${fixture}" rev-parse HEAD)"
brainmap="${fixture}/bin/brainmap"
brainmap_sha256="$(sha256sum "${brainmap}" | cut -d ' ' -f 1)"
brainmapd_sha256="$(printf 'b%.0s' {1..64})"
producer_digests="$(jq -cn \
  --arg integrated "$(sha256sum "${root}/scripts/m8-integrated-qualification.sh" | cut -d ' ' -f 1)" \
  --arg codex "$(sha256sum "${root}/scripts/m8-codex-fia5.sh" | cut -d ' ' -f 1)" \
  --arg release "$(sha256sum "${root}/scripts/m8-release-qualification.sh" | cut -d ' ' -f 1)" \
  --arg assemble "$(sha256sum "${fixture}/scripts/m8-assemble-qualification.sh" | cut -d ' ' -f 1)" '
  {
    integratedQualificationSha256: $integrated,
    codexFia5Sha256: $codex,
    releaseQualificationSha256: $release,
    assembleQualificationSha256: $assemble
  }
')"
build_info="$(jq -cn \
  --arg candidateCommit "${candidate_commit}" \
  --argjson producerDigests "${producer_digests}" '
  {
    schemaVersion: "brainmap-build-info-v1",
    candidateCommit: $candidateCommit,
    cargoProfile: "release",
    qualification: {
      eligible: true,
      marker: "brainmap-clean-locked-two-root-v1",
      release: true,
      locked: true,
      twoRootCandidate: true
    },
    producerDigests: $producerDigests
  }
')"
build_info_sha256="$(printf '%s' "${build_info}" | sha256sum | cut -d ' ' -f 1)"
reproducibility_manifest="${temporary}/reproducibility.json"
jq -n \
  --arg candidateCommit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg buildInfoSha256 "${build_info_sha256}" \
  --argjson producerDigests "${producer_digests}" '{
  schemaVersion: "brainmap-release-reproducibility-v2",
  candidateCommit: $candidateCommit,
  profile: "release",
  locked: true,
  twoRootByteIdentical: true,
  cleanTree: true,
  brainmapSha256: $brainmapSha256,
  brainmapdSha256: $brainmapdSha256,
  buildInfoSha256: $buildInfoSha256,
  producerDigests: $producerDigests
}' >"${reproducibility_manifest}"
reproducibility_sha256="$(sha256sum "${reproducibility_manifest}" | cut -d ' ' -f 1)"

runner_evidence="${temporary}/runner-evidence"
host_evidence="${temporary}/host-evidence"
release_evidence="${temporary}/release-evidence"
mkdir -p \
  "${runner_evidence}/reports" \
  "${host_evidence}" \
  "${release_evidence}/gates"

jq -n \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg reproducibilitySha256 "${reproducibility_sha256}" '{
  schemaVersion: "brainmap-m8-runner-v2",
  qualificationEligible: true,
  result: "passed",
  candidate: {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  },
  executionMode: "docker",
  build: {
    profile: "release",
    locked: true,
    twoRootByteIdentical: true,
    reproducibilityManifestSha256: $reproducibilitySha256
  }
}' >"${runner_evidence}/runner-manifest.json"
printf '{"fixture":"runner payload"}\n' >"${runner_evidence}/reports/fia1.json"
cp "${reproducibility_manifest}" \
  "${runner_evidence}/release-reproducibility-manifest.json"
write_checksums "${runner_evidence}"

codex_archive_sha='6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd'
codex_binary_sha='901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429'
first_decision_id='dec_1720000000000_aaaaaaaaaaaa'
second_decision_id='dec_1720000000002_cccccccccccc'
packet_id='upd_1720000000001_bbbbbbbbbbbb'
jq -n \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg codexArchiveSha "${codex_archive_sha}" \
  --arg codexBinarySha "${codex_binary_sha}" \
  --arg firstDecisionId "${first_decision_id}" \
  --arg secondDecisionId "${second_decision_id}" \
  --arg packetId "${packet_id}" '{
  schemaVersion: "brainmap-m8-host-observation-v2",
  qualificationEligible: true,
  mode: "qualification",
  candidate: {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  },
  officialCodex: {
    version: "codex-cli 0.144.0",
    target: "x86_64-unknown-linux-musl",
    archiveSha256: $codexArchiveSha,
    binarySha256: $codexBinarySha,
    observedBinarySha256: $codexBinarySha,
    archiveVerified: true,
    binaryVerified: true
  },
  config: {
    approvalPolicy: "on-request",
    approvalsReviewer: "user",
    sandboxMode: "workspace-write",
    workspaceWriteNetworkAccess: false,
    bypassHookTrust: false,
    bypassApprovalsAndSandbox: false,
    feedbackApprovalMode: "prompt",
    applyApprovalMode: "prompt",
    codexHomeSha256: ("8" * 64),
    gateMode: "active",
    autopilotMode: "conservative"
  },
  launch: {
    launcherSha256: ("3" * 64),
    argvSha256: ("4" * 64),
    argv: [],
    appServerArgvSha256: ("5" * 64),
    appServerArgv: [],
    codexHomeBound: true,
    projectInventoryBound: true,
    session: {source:"cli",idSha256:("9" * 64),createdAt:1720000000}
  },
  hooks: {
    trustedHookCount: 2,
    entries: [
      {eventName:"preToolUse",currentHash:("sha256:" + ("6" * 64)),trustStatus:"trusted"},
      {eventName:"userPromptSubmit",currentHash:("sha256:" + ("7" * 64)),trustStatus:"trusted"}
    ],
    executedHookGateCount: 1
  },
  calls: {
    count: 7,
    order: [
      "brainmap_decision_gate", "brainmap_record_decision",
      "brainmap_learn_feedback", "brainmap_preview_update",
      "brainmap_apply_update", "brainmap_decision_gate",
      "brainmap_record_decision"
    ],
    first: {
      decisionId:$firstDecisionId,outcome:"ask_user",selectedOption:null,
      action:{chosen:"biome",wasAsked:true}
    },
    feedback:{packetId:$packetId,previewed:true,approved:true},
    second: {
      decisionId:$secondDecisionId,outcome:"proceed",selectedOption:"prettier",
      changed:true,action:{chosen:"prettier",wasAsked:false}
    }
  },
  ledger:{correlation:"complete",correlatedEventCount:5,postBoundaryEventCount:6},
  project:{inventorySha256:("a" * 64),workflowSha256:("b" * 64),unchanged:true}
}' >"${host_evidence}/host-observation.json"
host_observation_sha="$(sha256sum "${host_evidence}/host-observation.json" | cut -d ' ' -f 1)"

jq -n \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg codexArchiveSha "${codex_archive_sha}" \
  --arg codexBinarySha "${codex_binary_sha}" \
  --arg hostObservationSha "${host_observation_sha}" '{
  schemaVersion: "brainmap-m8-host-v2",
  qualificationEligible: true,
  mode: "qualification",
  candidate: {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  },
  adapter: {
    target: "codex",
    hostVersion: "codex-cli 0.144.0",
    launchMode: "normal",
    trustBypassUsed: false,
    persistedHookAccepted: true,
    projectTrusted: true
  },
  provenance: {
    configuredBrainmapSha256: $brainmapSha256,
    configuredBrainmapdSha256: $brainmapdSha256,
    codexTarget: "x86_64-unknown-linux-musl",
    officialCodexArchiveSha256: $codexArchiveSha,
    officialCodexBinarySha256: $codexBinarySha,
    observedCodexBinarySha256: $codexBinarySha,
    officialCodexVerified: true
  },
  artifacts: {
    hostObservation: {path:"host-observation.json",sha256:$hostObservationSha}
  }
}' >"${host_evidence}/manifest.json"
printf '{"sequence":1,"success":true}\n' >"${host_evidence}/events.jsonl"
write_checksums "${host_evidence}"

jq -n \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" \
  --arg reproducibilitySha256 "${reproducibility_sha256}" '{
  schemaVersion: "brainmap-m8-release-v1",
  qualificationEligible: true,
  candidate: {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  },
  sourceTreeDirtyBefore: false,
  sourceTreeDirtyAfter: false,
  reproducibilityManifestSha256: $reproducibilitySha256
}' >"${release_evidence}/manifest.json"
cp "${reproducibility_manifest}" \
  "${release_evidence}/reproducibility-manifest.json"
printf 'release gate passed\n' >"${release_evidence}/gates/format.log"
write_checksums "${release_evidence}"

export EXPECTED_CANDIDATE_COMMIT="${candidate_commit}"
export EXPECTED_BRAINMAP_SHA256="${brainmap_sha256}"
export EXPECTED_BRAINMAPD_SHA256="${brainmapd_sha256}"
export EXPECTED_BUILD_INFO="${build_info}"
export BRAINMAP_STUB_MARKER="${temporary}/verifier-invocation.txt"

common=(
  --brainmap "${brainmap}"
  --brainmap-sha256 "${brainmap_sha256}"
  --brainmapd-sha256 "${brainmapd_sha256}"
  --candidate-commit "${candidate_commit}"
  --reproducibility-manifest "${reproducibility_manifest}"
  --runner-evidence "${runner_evidence}"
  --host-evidence "${host_evidence}"
  --release-evidence "${release_evidence}"
)

fixture_assembler="${fixture}/scripts/m8-assemble-qualification.sh"
expect_failure 'unknown argument: --local' "${fixture_assembler}" --local
expect_failure 'missing required --brainmap PATH' "${fixture_assembler}"
expect_failure 'brainmap SHA-256 mismatch' \
  "${fixture_assembler}" \
  --brainmap "${brainmap}" \
  --brainmap-sha256 0000000000000000000000000000000000000000000000000000000000000000 \
  "${common[@]:4}" \
  --out "${temporary}/bad-binary-hash"

brainmap_link="${temporary}/brainmap-link"
ln -s "${brainmap}" "${brainmap_link}"
expect_failure 'brainmap is not a symlink-free executable regular file' \
  "${fixture_assembler}" \
  --brainmap "${brainmap_link}" \
  "${common[@]:2}" \
  --out "${temporary}/symlinked-binary"

touch "${fixture}/dirty"
expect_failure 'qualification assembly requires clean HEAD' \
  "${fixture_assembler}" "${common[@]}" --out "${temporary}/dirty-output"
rm "${fixture}/dirty"

bad_host="${temporary}/bad-host-checksums"
cp -R "${host_evidence}" "${bad_host}"
printf 'changed\n' >>"${bad_host}/events.jsonl"
expect_failure 'host SHA256SUMS is malformed, unsorted, incomplete, or stale' \
  "${fixture_assembler}" \
  "${common[@]:0:12}" \
  --host-evidence "${bad_host}" \
  "${common[@]:14}" \
  --out "${temporary}/bad-checksums-output"

linked_host="${temporary}/linked-host"
cp -R "${host_evidence}" "${linked_host}"
ln -s manifest.json "${linked_host}/linked.json"
expect_failure 'host evidence contains a symlink' \
  "${fixture_assembler}" \
  "${common[@]:0:12}" \
  --host-evidence "${linked_host}" \
  "${common[@]:14}" \
  --out "${temporary}/linked-output"

mixed_runner="${temporary}/mixed-runner"
cp -R "${runner_evidence}" "${mixed_runner}"
jq '.candidate.commit = "0000000000000000000000000000000000000000"' \
  "${mixed_runner}/runner-manifest.json" >"${mixed_runner}/changed.json"
mv "${mixed_runner}/changed.json" "${mixed_runner}/runner-manifest.json"
write_checksums "${mixed_runner}"
expect_failure 'runner manifest does not match the qualifying candidate contract' \
  "${fixture_assembler}" \
  "${common[@]:0:10}" \
  --runner-evidence "${mixed_runner}" \
  "${common[@]:12}" \
  --out "${temporary}/mixed-output"

legacy_host="${temporary}/legacy-host"
cp -R "${host_evidence}" "${legacy_host}"
jq '.schemaVersion = "brainmap-m8-host-v1"' \
  "${legacy_host}/manifest.json" >"${legacy_host}/changed.json"
mv "${legacy_host}/changed.json" "${legacy_host}/manifest.json"
write_checksums "${legacy_host}"
expect_failure 'host manifest does not match the qualifying candidate contract' \
  "${fixture_assembler}" \
  "${common[@]:0:12}" \
  --host-evidence "${legacy_host}" \
  "${common[@]:14}" \
  --out "${temporary}/legacy-host-output"

unsafe_host="${temporary}/unsafe-host"
cp -R "${host_evidence}" "${unsafe_host}"
jq '.config.workspaceWriteNetworkAccess = true' \
  "${unsafe_host}/host-observation.json" >"${unsafe_host}/changed.json"
mv "${unsafe_host}/changed.json" "${unsafe_host}/host-observation.json"
unsafe_observation_sha="$(sha256sum "${unsafe_host}/host-observation.json" | cut -d ' ' -f 1)"
jq --arg sha "${unsafe_observation_sha}" '.artifacts.hostObservation.sha256 = $sha' \
  "${unsafe_host}/manifest.json" >"${unsafe_host}/changed.json"
mv "${unsafe_host}/changed.json" "${unsafe_host}/manifest.json"
write_checksums "${unsafe_host}"
expect_failure 'host observation does not match the qualifying contract' \
  "${fixture_assembler}" \
  "${common[@]:0:12}" \
  --host-evidence "${unsafe_host}" \
  "${common[@]:14}" \
  --out "${temporary}/unsafe-host-output"

expect_failure 'qualification output path must be absolute' \
  "${fixture_assembler}" "${common[@]}" --out relative-output
expect_failure 'qualification output overlaps an input' \
  "${fixture_assembler}" "${common[@]}" --out "${runner_evidence}/nested-output"
mkdir "${temporary}/existing-output"
expect_failure 'qualification output already exists' \
  "${fixture_assembler}" "${common[@]}" --out "${temporary}/existing-output"

rejected_output="${temporary}/verifier-rejected-output"
expect_failure 'exact candidate rejected the assembled qualification bundle' \
  env BRAINMAP_STUB_FAIL=1 \
  "${fixture_assembler}" "${common[@]}" --out "${rejected_output}"
[[ ! -e "${rejected_output}" ]] || fail 'verifier failure published partial evidence'

out="${temporary}/qualification"
"${fixture_assembler}" "${common[@]}" --out "${out}" >/dev/null
[[ -d "${out}" && ! -L "${out}" ]] || fail 'valid bundle was not published'
[[ -f "${BRAINMAP_STUB_MARKER}" ]] || fail 'exact candidate verifier was not invoked'
verified_staging="$(<"${BRAINMAP_STUB_MARKER}")"
[[ "${verified_staging}" == "${temporary}/.brainmap-m8-qualification."*'/bundle' ]] ||
  fail "verifier did not receive same-filesystem staging: ${verified_staging}"

jq -e \
  --arg commit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha256}" \
  --arg brainmapdSha256 "${brainmapd_sha256}" '
  type == "object"
  and (keys == ["candidate", "evidence", "privacy", "schemaVersion"])
  and .schemaVersion == "brainmap-m8-qualification-bundle-v1"
  and .candidate == {
    commit: $commit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  }
  and (.evidence | keys == [
    "hostChecksums", "hostManifest", "releaseChecksums", "releaseManifest",
    "reproducibilityManifest", "runnerChecksums", "runnerManifest"
  ])
  and .privacy == {
    rawPromptsRetained: false,
    secretsRetained: false,
    privatePathsRetained: false
  }
' "${out}/qualification.json" >/dev/null || fail 'strict root manifest is invalid'

cmp "${runner_evidence}/runner-manifest.json" "${out}/runner/manifest.json" >/dev/null ||
  fail 'runner manifest alias changed producer evidence'
cmp "${runner_evidence}/SHA256SUMS" "${out}/runner/producer-SHA256SUMS" >/dev/null ||
  fail 'runner producer checksum ledger was not retained'
for source in runner host release; do
  case "${source}" in
    runner) source_root="${runner_evidence}" ;;
    host) source_root="${host_evidence}" ;;
    release) source_root="${release_evidence}" ;;
  esac
  while IFS= read -r -d '' source_file; do
    relative="${source_file#"${source_root}/"}"
    if [[ "${source}" == runner && "${relative}" == SHA256SUMS ]]; then
      continue
    fi
    cmp "${source_file}" "${out}/${source}/${relative}" >/dev/null ||
      fail "copied evidence changed: ${source}/${relative}"
  done < <(find "${source_root}" -type f -print0)
done

(cd "${out}" && sha256sum -c SHA256SUMS >/dev/null) ||
  fail 'root checksum verification failed'
for subtree in runner host release; do
  (cd "${out}/${subtree}" && sha256sum -c SHA256SUMS >/dev/null) ||
    fail "${subtree} checksum verification failed"
done
generated_root_sums="${temporary}/generated-root-sums"
(
  cd "${out}"
  find . -type f ! -path './SHA256SUMS' -print0 |
    sort -z |
    while IFS= read -r -d '' artifact; do
      relative="${artifact#./}"
      printf '%s  %s\n' "$(sha256sum "${relative}" | cut -d ' ' -f 1)" "${relative}"
    done
) >"${generated_root_sums}"
cmp "${generated_root_sums}" "${out}/SHA256SUMS" >/dev/null ||
  fail 'root SHA256SUMS is not sorted, recursive, and exact'
[[ -z "$(find "${out}" -type l -print -quit)" ]] || fail 'published bundle contains a symlink'
[[ -z "$(find "${temporary}" -maxdepth 1 -name '.brainmap-m8-qualification.*' -print -quit)" ]] ||
  fail 'assembler left a staging directory after publication'

printf 'm8 qualification assembler interface tests passed (16 cases)\n'
