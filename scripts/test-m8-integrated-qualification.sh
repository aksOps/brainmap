#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runner="${root}/scripts/m8-integrated-qualification.sh"
repro_runner="${root}/scripts/verify-release-reproducibility.sh"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

fail() {
  echo "m8 runner test failed: $*" >&2
  exit 1
}

expect_failure() {
  local expected="$1"
  shift
  local output
  if output="$(${runner} "$@" 2>&1)"; then
    fail "command unexpectedly succeeded: ${expected}"
  fi
  grep -F -- "${expected}" <<<"${output}" >/dev/null ||
    fail "missing error '${expected}' in: ${output}"
}

expect_command_failure() {
  local expected="$1"
  shift
  local output
  if output="$("$@" 2>&1)"; then
    fail "command unexpectedly succeeded: ${expected}"
  fi
  grep -F -- "${expected}" <<<"${output}" >/dev/null ||
    fail "missing error '${expected}' in: ${output}"
}

[[ -x "${runner}" ]] || fail "runner is not executable"
help="$(${runner} --help)"
for required in \
  '--brainmap PATH' \
  '--brainmap-sha256 SHA256' \
  '--brainmapd PATH' \
  '--brainmapd-sha256 SHA256' \
  '--reproducibility-manifest PATH' \
  '--local' \
  '--include-fia7'; do
  grep -F -- "${required}" <<<"${help}" >/dev/null ||
    fail "help is missing ${required}"
done
grep -F -- 'non-qualifying diagnostic' <<<"${help}" >/dev/null ||
  fail 'help must label local mode non-qualifying'
grep -F -- 'Docker runs always include FIA-7' <<<"${help}" >/dev/null ||
  fail 'help must state that qualifying Docker always includes FIA-7'
grep -F -- 'outside the repository' <<<"${help}" >/dev/null ||
  fail 'help must require external evidence for qualifying Docker runs'
grep -F -- 'brainmap-m8-runner-v2' "${runner}" >/dev/null ||
  fail 'runner must emit the strict v2 evidence manifest'
if grep -F -- 'brainmap-m8-fia-v1' "${runner}" >/dev/null; then
  fail 'runner must not emit the obsolete flat FIA self-attestation'
fi

repro_help="$(${repro_runner} --help)"
grep -F -- '--manifest-out PATH' <<<"${repro_help}" >/dev/null ||
  fail 'release reproducibility help is missing --manifest-out PATH'
expect_command_failure 'manifest output path must be absolute' \
  "${repro_runner}" --manifest-out relative/reproducibility.json

candidate_commit="$(git -C "${root}" rev-parse HEAD)"
producer_digests="$(jq -cn \
  --arg integrated "$(sha256sum "${root}/scripts/m8-integrated-qualification.sh" | cut -d ' ' -f 1)" \
  --arg codex "$(sha256sum "${root}/scripts/m8-codex-fia5.sh" | cut -d ' ' -f 1)" \
  --arg release "$(sha256sum "${root}/scripts/m8-release-qualification.sh" | cut -d ' ' -f 1)" \
  --arg assemble "$(sha256sum "${root}/scripts/m8-assemble-qualification.sh" | cut -d ' ' -f 1)" '
  {
    integratedQualificationSha256: $integrated,
    codexFia5Sha256: $codex,
    releaseQualificationSha256: $release,
    assembleQualificationSha256: $assemble
  }
')"
build_info="$(jq -cn \
  --arg commit "${candidate_commit}" \
  --argjson producers "${producer_digests}" '
  {
    schemaVersion: "brainmap-build-info-v1",
    candidateCommit: $commit,
    cargoProfile: "release",
    qualification: {
      eligible: true,
      marker: "brainmap-clean-locked-two-root-v1",
      release: true,
      locked: true,
      twoRootCandidate: true
    },
    producerDigests: $producers
  }
')"
for binary in brainmap brainmapd; do
  cat >"${temporary}/${binary}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == build-info && "\$#" -eq 1 ]]; then
  printf '%s\n' '${build_info}'
  exit 0
fi
exit 0
EOF
done
chmod 755 "${temporary}/brainmap" "${temporary}/brainmapd"
brainmap_sha="$(sha256sum "${temporary}/brainmap" | cut -d ' ' -f 1)"
brainmapd_sha="$(sha256sum "${temporary}/brainmapd" | cut -d ' ' -f 1)"
build_info_sha="$(printf '%s' "${build_info}" | sha256sum | cut -d ' ' -f 1)"

mkdir "${temporary}/inner-work" "${temporary}/inner-evidence"
expect_failure 'inner brainmap SHA-256 mismatch' \
  __inner \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 0000000000000000000000000000000000000000000000000000000000000000 \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --inner-work "${temporary}/inner-work" \
  --inner-evidence "${temporary}/inner-evidence"

expect_failure 'missing required --reproducibility-manifest PATH for qualifying Docker mode' \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}"

cat >"${temporary}/bad-reproducibility.json" <<EOF
{
  "schemaVersion": "wrong-schema",
  "candidateCommit": "${candidate_commit}",
  "profile": "release",
  "locked": true,
  "twoRootByteIdentical": true,
  "cleanTree": true,
  "buildInfoSha256": "${build_info_sha}",
  "producerDigests": ${producer_digests},
  "brainmapSha256": "${brainmap_sha}",
  "brainmapdSha256": "${brainmapd_sha}"
}
EOF
expect_failure 'invalid strict reproducibility manifest' \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}" \
  --reproducibility-manifest "${temporary}/bad-reproducibility.json" \
  --out "${temporary}/bad-repro-output"

cat >"${temporary}/valid-reproducibility.json" <<EOF
{
  "schemaVersion": "brainmap-release-reproducibility-v2",
  "candidateCommit": "${candidate_commit}",
  "profile": "release",
  "locked": true,
  "twoRootByteIdentical": true,
  "cleanTree": true,
  "buildInfoSha256": "${build_info_sha}",
  "producerDigests": ${producer_digests},
  "brainmapSha256": "${brainmap_sha}",
  "brainmapdSha256": "${brainmapd_sha}"
}
EOF
expect_failure 'qualifying Docker mode requires explicit absolute --out outside the repository' \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}" \
  --reproducibility-manifest "${temporary}/valid-reproducibility.json"
expect_failure 'qualifying Docker mode requires evidence output outside the repository' \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}" \
  --reproducibility-manifest "${temporary}/valid-reproducibility.json" \
  --out "${root}/evidence/m001/forbidden-test-output"

non_head_commit="$(git -C "${root}" rev-parse HEAD^)"
cat >"${temporary}/non-head-reproducibility.json" <<EOF
{
  "schemaVersion": "brainmap-release-reproducibility-v2",
  "candidateCommit": "${non_head_commit}",
  "profile": "release",
  "locked": true,
  "twoRootByteIdentical": true,
  "cleanTree": true,
  "buildInfoSha256": "${build_info_sha}",
  "producerDigests": ${producer_digests},
  "brainmapSha256": "${brainmap_sha}",
  "brainmapdSha256": "${brainmapd_sha}"
}
EOF
expect_failure 'qualifying Docker mode requires candidate commit at clean HEAD' \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${non_head_commit}" \
  --reproducibility-manifest "${temporary}/non-head-reproducibility.json" \
  --out "${temporary}/non-head-output"

# Local mode is deliberately diagnostic and must not require release provenance.
mkdir "${temporary}/existing-output"
expect_failure 'evidence directory already exists' \
  --local \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}" \
  --out "${temporary}/existing-output"

expect_failure 'missing required --brainmap PATH' --local
expect_failure 'must be an absolute path' \
  --local \
  --brainmap relative/brainmap \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}"
expect_failure 'brainmap SHA-256 mismatch' \
  --local \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 0000000000000000000000000000000000000000000000000000000000000000 \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}"
expect_failure 'must be exactly 64 lowercase hexadecimal characters' \
  --local \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 nope \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}"
expect_failure 'missing required --candidate-commit COMMIT' \
  --local \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}"
expect_failure 'candidate commit must be exactly 40 lowercase hexadecimal characters' \
  --local \
  --brainmap "${temporary}/brainmap" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${temporary}/brainmapd" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit short

grep -F -- 'mv -T -n' "${runner}" >/dev/null ||
  fail 'runner publication must use no-replace mv semantics'

echo 'm8 integrated qualification interface tests passed (17 cases)'
