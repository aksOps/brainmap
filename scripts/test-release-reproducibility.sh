#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
producer="${root}/scripts/verify-release-reproducibility.sh"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

fail() {
  echo "release reproducibility interface test failed: $*" >&2
  exit 1
}

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

fixture="${temporary}/fixture"
fake_bin="${temporary}/bin"
mkdir -p "${fixture}/scripts" "${fake_bin}"
cp "${producer}" "${fixture}/scripts/verify-release-reproducibility.sh"
for script in \
  m8-integrated-qualification.sh \
  m8-codex-fia5.sh \
  m8-release-qualification.sh \
  m8-assemble-qualification.sh; do
  cp "${root}/scripts/${script}" "${fixture}/scripts/${script}"
done
chmod 0755 "${fixture}/scripts/"*.sh
printf '/target/\n' >"${fixture}/.gitignore"

cat >"${fake_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

[[ "$*" == 'build --release --locked -p brainmap-cli --bin brainmap --bin brainmapd' ]] || {
  echo "unexpected cargo invocation: $*" >&2
  exit 91
}
qualifying=false
commit='0000000000000000000000000000000000000000'
marker='nonqualifying'
if [[ "${BRAINMAP_INTERNAL_QUALIFICATION_MARKER:-}" == \
      brainmap-clean-locked-two-root-v1 ]]; then
  [[ "${BRAINMAP_INTERNAL_CANDIDATE_COMMIT:-}" =~ ^[0-9a-f]{40}$ ]]
  [[ "${BRAINMAP_INTERNAL_SOURCE_CLEAN:-}" == true ]]
  [[ "${BRAINMAP_INTERNAL_LOCKED:-}" == true ]]
  [[ "${BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE:-}" == true ]]
  qualifying=true
  commit="${BRAINMAP_INTERNAL_CANDIDATE_COMMIT}"
  marker='brainmap-clean-locked-two-root-v1'
else
  [[ -z "${BRAINMAP_INTERNAL_QUALIFICATION_MARKER:-}" ]]
  [[ -z "${BRAINMAP_INTERNAL_CANDIDATE_COMMIT:-}" ]]
  [[ -z "${BRAINMAP_INTERNAL_SOURCE_CLEAN:-}" ]]
  [[ -z "${BRAINMAP_INTERNAL_LOCKED:-}" ]]
  [[ -z "${BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE:-}" ]]
fi

integrated_sha="$(sha256sum scripts/m8-integrated-qualification.sh | cut -d ' ' -f 1)"
fia5_sha="$(sha256sum scripts/m8-codex-fia5.sh | cut -d ' ' -f 1)"
release_sha="$(sha256sum scripts/m8-release-qualification.sh | cut -d ' ' -f 1)"
assemble_sha="$(sha256sum scripts/m8-assemble-qualification.sh | cut -d ' ' -f 1)"
build_info="$(jq -cn \
  --arg commit "${commit}" \
  --arg marker "${marker}" \
  --argjson qualifying "${qualifying}" \
  --arg integrated "${integrated_sha}" \
  --arg fia5 "${fia5_sha}" \
  --arg release "${release_sha}" \
  --arg assemble "${assemble_sha}" '{
    schemaVersion:"brainmap-build-info-v1",
    candidateCommit:$commit,
    cargoProfile:"release",
    qualification:{
      eligible:$qualifying,
      marker:$marker,
      release:$qualifying,
      locked:$qualifying,
      twoRootCandidate:$qualifying
    },
    producerDigests:{
      integratedQualificationSha256:$integrated,
      codexFia5Sha256:$fia5,
      releaseQualificationSha256:$release,
      assembleQualificationSha256:$assemble
    }
  }')"

mkdir -p "${CARGO_TARGET_DIR:?}/release"
for binary in brainmap brainmapd; do
  cat >"${CARGO_TARGET_DIR}/release/${binary}" <<EOF_BINARY
#!/usr/bin/env bash
set -euo pipefail
[[ "\${1:-}" == build-info && "\$#" -eq 1 ]] || exit 92
build_info='${build_info}'
if [[ "\${FAKE_BUILD_INFO_MISMATCH:-0}" == 1 && "\$0" == *target-b* ]]; then
  build_info="\$(jq -c '{cargoProfile,schemaVersion,candidateCommit,qualification,producerDigests}' \
    <<<"\${build_info}")"
fi
printf '%s\\n' "\${build_info}"
EOF_BINARY
  chmod 0755 "${CARGO_TARGET_DIR}/release/${binary}"
done
EOF
chmod 0755 "${fake_bin}/cargo"

git -C "${fixture}" init -q
git -C "${fixture}" config user.email fixture@example.invalid
git -C "${fixture}" config user.name Fixture
git -C "${fixture}" add .
git -C "${fixture}" commit -qm 'release reproducibility fixture'
candidate_commit="$(git -C "${fixture}" rev-parse HEAD)"

# Even a complete ambient contract cannot mark the no-manifest workflow as
# qualifying: the production script clears every internal variable itself.
PATH="${fake_bin}:${PATH}" \
  BRAINMAP_INTERNAL_QUALIFICATION_MARKER=brainmap-clean-locked-two-root-v1 \
  BRAINMAP_INTERNAL_CANDIDATE_COMMIT="${candidate_commit}" \
  BRAINMAP_INTERNAL_SOURCE_CLEAN=true \
  BRAINMAP_INTERNAL_LOCKED=true \
  BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE=true \
  "${fixture}/scripts/verify-release-reproducibility.sh" >/dev/null
jq -e '.qualification == {
  eligible:false,
  marker:"nonqualifying",
  release:false,
  locked:false,
  twoRootCandidate:false
}' < <("${fixture}/target/release/brainmap" build-info) >/dev/null ||
  fail 'no-manifest build inherited a qualifying marker'

manifest="${temporary}/reproducibility.json"
PATH="${fake_bin}:${PATH}" \
  "${fixture}/scripts/verify-release-reproducibility.sh" \
    --manifest-out "${manifest}" >/dev/null

brainmap_info="$("${fixture}/target/release/brainmap" build-info)"
brainmapd_info="$("${fixture}/target/release/brainmapd" build-info)"
[[ "${brainmap_info}" == "${brainmapd_info}" ]] ||
  fail 'published binaries expose different build info'
build_info_sha="$(printf '%s' "${brainmap_info}" | sha256sum | cut -d ' ' -f 1)"

jq -e \
  --arg commit "${candidate_commit}" \
  --arg buildInfoSha "${build_info_sha}" \
  --argjson producerDigests "$(jq -c '.producerDigests' <<<"${brainmap_info}")" '
  type == "object"
  and (keys == [
    "brainmapSha256", "brainmapdSha256", "buildInfoSha256",
    "candidateCommit", "cleanTree", "locked", "producerDigests", "profile",
    "schemaVersion", "twoRootByteIdentical"
  ])
  and .schemaVersion == "brainmap-release-reproducibility-v2"
  and .candidateCommit == $commit
  and .profile == "release"
  and .locked == true
  and .twoRootByteIdentical == true
  and .cleanTree == true
  and .buildInfoSha256 == $buildInfoSha
  and .producerDigests == $producerDigests
  and (.brainmapSha256 | test("^[0-9a-f]{64}$"))
  and (.brainmapdSha256 | test("^[0-9a-f]{64}$"))
' "${manifest}" >/dev/null || fail 'strict v2 reproducibility manifest is invalid'

[[ -z "$(git -C "${fixture}" status --porcelain --untracked-files=all)" ]] ||
  fail 'reproducibility workflow dirtied the source fixture'

expect_failure 'build-info mismatch across reproduced binaries' \
  env PATH="${fake_bin}:${PATH}" FAKE_BUILD_INFO_MISMATCH=1 \
  "${fixture}/scripts/verify-release-reproducibility.sh" \
  --manifest-out "${temporary}/mismatched.json"

printf 'release reproducibility interface tests passed (v2 provenance + mismatch rejection)\n'
