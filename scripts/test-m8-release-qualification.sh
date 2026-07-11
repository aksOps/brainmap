#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
runner="${root}/scripts/m8-release-qualification.sh"
temporary="$(mktemp -d)"
trap 'rm -rf "${temporary}"' EXIT

fail() {
  echo "m8 release qualification test failed: $*" >&2
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

[[ -x "${runner}" ]] || fail "runner is not executable"
help="$(${runner} --help)"
for required in \
  '--brainmap PATH' \
  '--brainmap-sha256 SHA256' \
  '--brainmapd PATH' \
  '--brainmapd-sha256 SHA256' \
  '--candidate-commit COMMIT' \
  '--reproducibility-manifest PATH' \
  '--out DIR'; do
  grep -F -- "${required}" <<<"${help}" >/dev/null ||
    fail "help is missing ${required}"
done

# The fixture is a real clean Git repository, but every expensive gate is a
# fixture-local executable. The production runner has no fake/test mode.
fixture="${temporary}/fixture"
fake_bin="${temporary}/fake-bin"
failing_bin="${temporary}/failing-bin"
secret_bin="${temporary}/secret-bin"
email_bin="${temporary}/email-bin"
non_utf8_bin="${temporary}/non-utf8-bin"
mkdir -p \
  "${fixture}/scripts" \
  "${fixture}/crates/brainmap-cli" \
  "${fixture}/vendor/i18n-embed-fl" \
  "${fixture}/npm/brainmap" \
  "${fixture}/.github/workflows" \
  "${fixture}/target/release" \
  "${fake_bin}" \
  "${failing_bin}" \
  "${secret_bin}" \
  "${email_bin}" \
  "${non_utf8_bin}"
cp "${runner}" "${fixture}/scripts/m8-release-qualification.sh"
cp "${root}/scripts/m8-integrated-qualification.sh" \
  "${fixture}/scripts/m8-integrated-qualification.sh"
cp "${root}/scripts/m8-codex-fia5.sh" \
  "${fixture}/scripts/m8-codex-fia5.sh"
cp "${root}/scripts/m8-assemble-qualification.sh" \
  "${fixture}/scripts/m8-assemble-qualification.sh"
chmod 0755 "${fixture}/scripts/m8-release-qualification.sh"

cat >"${fixture}/.gitignore" <<'EOF'
/target/
/npm/brainmap/bin/
EOF
cat >"${fixture}/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/brainmap-cli"]
resolver = "3"
EOF
cat >"${fixture}/Cargo.lock" <<'EOF'
version = 4
EOF
cat >"${fixture}/crates/brainmap-cli/Cargo.toml" <<'EOF'
[package]
name = "brainmap-cli"
version = "0.1.0"
edition = "2024"
EOF
cat >"${fixture}/vendor/i18n-embed-fl/Cargo.lock" <<'EOF'
version = 4
EOF
cat >"${fixture}/npm/brainmap/package.json" <<'EOF'
{"name":"@fixture/brainmap","version":"0.1.0","scripts":{"test":"true"}}
EOF
cat >"${fixture}/crates/brainmap-cli/brainmap.json" <<'EOF'
{"bomFormat":"CycloneDX","specVersion":"1.5","metadata":{},"components":[]}
EOF
cat >"${fixture}/.github/workflows/ci.yml" <<'EOF'
name: fixture
on: [push]
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: true
EOF

cat >"${fixture}/scripts/generate-sbom.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
jq -e '.bomFormat == "CycloneDX"' crates/brainmap-cli/brainmap.json >/dev/null
EOF
cat >"${fixture}/scripts/test-vendored-i18n.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'vendored tests passed'
EOF
cat >"${fixture}/scripts/prepare-npm-package.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
mkdir -p npm/brainmap/bin
cp target/release/brainmap npm/brainmap/bin/brainmap
cp target/release/brainmapd npm/brainmap/bin/brainmapd
chmod 0755 npm/brainmap/bin/brainmap npm/brainmap/bin/brainmapd
EOF
cat >"${fixture}/scripts/test-m8-integrated-qualification.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'integrated runner interface passed'
EOF
cat >"${fixture}/scripts/test-m8-release-qualification.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'release runner interface passed'
EOF
cat >"${fixture}/scripts/test-m8-codex-fia5.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'Codex FIA-5 producer interface passed'
EOF
cat >"${fixture}/scripts/test-m8-assemble-qualification.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'qualification assembler interface passed'
EOF
cat >"${fixture}/scripts/test-release-reproducibility.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'release reproducibility interface passed'
EOF
cat >"${fixture}/scripts/release-qualification.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
out="${BRAINMAP_QUALIFICATION_OUT:?}"
mkdir -p "${out}"
jq -n \
  --arg sourceCommit "${BRAINMAP_CANDIDATE_COMMIT:?}" \
  --arg brainmapSha256 "${BRAINMAP_EXPECTED_SHA256:?}" \
  --arg brainmapdSha256 "${BRAINMAPD_EXPECTED_SHA256:?}" \
  --arg archiveSha256 "$(printf fixture-archive | sha256sum | cut -d ' ' -f 1)" \
  '{
    schemaVersion: "brainmap-release-qualification-v1",
    sourceCommit: $sourceCommit,
    sourceTreeDirty: false,
    startedAt: "2026-07-10T00:00:00Z",
    completedAt: "2026-07-10T00:30:00Z",
    host: "Linux fixture x86_64",
    toolchain: {rustc: "rustc fixture", cargo: "cargo fixture"},
    binaries: {
      brainmapSha256: $brainmapSha256,
      brainmapdSha256: $brainmapdSha256
    },
    portableArchiveSha256: $archiveSha256
  }' >"${out}/qualification-manifest.json"
jq -n '{
  cases: 238,
  falseProceed: 0,
  falseAsk: 0,
  falseBlock: 0,
  wrongChoice: 0,
  wrongRule: 0,
  wrongMetadata: 0,
  learnedRuleRecall: {
    exact: 1,
    supportedParaphrase: 1,
    negativeExpected: 207,
    negativeSpecificity: 1
  }
}' >"${out}/eval.json"
jq -n '{
  scaleRequested: 1000,
  executableRules: 1000,
  gateP95Ms: 2.5,
  indexRebuildMs: 150,
  candidateBounds: {retrieval: "actual-rule-term-postings"}
}' >"${out}/bench-1000.json"
jq -n '{
  scaleRequested: 5000,
  executableRules: 5000,
  gateP95Ms: 11.0,
  indexRebuildMs: 500,
  candidateBounds: {retrieval: "actual-rule-term-postings"}
}' >"${out}/bench-5000.json"
jq -n '{
  outcome: "proceed",
  learningEvent: {
    situation: "synthetic interface prompt",
    options: ["one", "two"]
  }
}' >"${out}/source-learned-gate.json"
for phase in verified staging-created files-written index-rebuilt links-checked \
  gate-checked existing-backed-up staging-activated; do
  jq -n --arg phase "${phase}" \
    --arg oldTreeHash "$(printf fixture-old | sha256sum | cut -d ' ' -f 1)" \
    --arg newTreeHash "$(printf fixture-new | sha256sum | cut -d ' ' -f 1)" \
    '{phase: $phase, completeState: "old", treeHash: $oldTreeHash, oldTreeHash: $oldTreeHash, newTreeHash: $newTreeHash}' \
    >"${out}/restore-fault-${phase}-state.json"
done
printf 'fixture evidence: %s\n' "${out}"
EOF
chmod 0755 "${fixture}"/scripts/*.sh

cat >"${fake_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo 'cargo 1.95.0 (fixture)'
fi
if [[ "${1:-}" == "build" ]]; then
  [[ "${BRAINMAP_INTERNAL_QUALIFICATION_MARKER:-}" == \
    brainmap-clean-locked-two-root-v1 ]]
  [[ "${BRAINMAP_INTERNAL_CANDIDATE_COMMIT:-}" == \
    "$(git rev-parse HEAD)" ]]
  [[ "${BRAINMAP_INTERNAL_SOURCE_CLEAN:-}" == true ]]
  [[ "${BRAINMAP_INTERNAL_LOCKED:-}" == true ]]
  [[ "${BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE:-}" == true ]]
fi
exit 0
EOF
cat >"${fake_bin}/rustc" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'rustc 1.95.0 (fixture)'
EOF
cat >"${fake_bin}/node" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'v22.0.0'
EOF
cat >"${fake_bin}/npm" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo '10.0.0'
else
  echo 'npm fixture gate passed'
fi
EOF
cat >"${fake_bin}/shellcheck" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'shellcheck fixture gate passed'
EOF
cat >"${fake_bin}/actionlint" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'actionlint fixture gate passed'
EOF
chmod 0755 "${fake_bin}"/*

cat >"${failing_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'fixture cargo failure' >&2
exit 7
EOF
cat >"${secret_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo 'api_key=12345678901234567890'
exit 0
EOF
cat >"${email_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo 'cargo 1.95.0 (fixture)'
else
  echo 'developer@example.invalid'
fi
exit 0
EOF
cat >"${non_utf8_bin}/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo 'cargo 1.95.0 (fixture)'
else
  printf '\377\n'
fi
exit 0
EOF
chmod 0755 \
  "${failing_bin}/cargo" \
  "${secret_bin}/cargo" \
  "${email_bin}/cargo" \
  "${non_utf8_bin}/cargo"

cp /bin/true "${fixture}/target/release/brainmap"
cp /bin/true "${fixture}/target/release/brainmapd"
chmod 0755 "${fixture}/target/release/brainmap" "${fixture}/target/release/brainmapd"
brainmap="${fixture}/target/release/brainmap"
brainmapd="${fixture}/target/release/brainmapd"
brainmap_sha="$(sha256sum "${brainmap}" | cut -d ' ' -f 1)"
brainmapd_sha="$(sha256sum "${brainmapd}" | cut -d ' ' -f 1)"

git -C "${fixture}" init -q
git -C "${fixture}" config user.name fixture
git -C "${fixture}" config user.email fixture@example.invalid
git -C "${fixture}" add .
git -C "${fixture}" commit -qm 'fixture base'
echo 'fixture head' >"${fixture}/HEAD-MARKER"
git -C "${fixture}" add HEAD-MARKER
git -C "${fixture}" commit -qm 'fixture head'
candidate_commit="$(git -C "${fixture}" rev-parse HEAD)"
non_head_commit="$(git -C "${fixture}" rev-parse HEAD^)"

producer_digests="$(jq -cn \
  --arg integrated "$(sha256sum "${fixture}/scripts/m8-integrated-qualification.sh" | cut -d ' ' -f 1)" \
  --arg codex "$(sha256sum "${fixture}/scripts/m8-codex-fia5.sh" | cut -d ' ' -f 1)" \
  --arg release "$(sha256sum "${fixture}/scripts/m8-release-qualification.sh" | cut -d ' ' -f 1)" \
  --arg assemble "$(sha256sum "${fixture}/scripts/m8-assemble-qualification.sh" | cut -d ' ' -f 1)" '
  {
    integratedQualificationSha256: $integrated,
    codexFia5Sha256: $codex,
    releaseQualificationSha256: $release,
    assembleQualificationSha256: $assemble
  }
')"

write_candidate_binaries() {
  local commit="$1" binary
  build_info="$(jq -cn \
    --arg candidateCommit "${commit}" \
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
  for binary in "${brainmap}" "${brainmapd}"; do
    cat >"${binary}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == build-info && "\$#" -eq 1 ]]; then
  printf '%s\n' '${build_info}'
  exit 0
fi
exit 0
EOF
    chmod 0755 "${binary}"
  done
  brainmap_sha="$(sha256sum "${brainmap}" | cut -d ' ' -f 1)"
  brainmapd_sha="$(sha256sum "${brainmapd}" | cut -d ' ' -f 1)"
  build_info_sha="$(printf '%s' "${build_info}" | sha256sum | cut -d ' ' -f 1)"
}

write_candidate_binaries "${candidate_commit}"

write_reproducibility_manifest() {
  local path="$1" commit="$2" schema="${3:-brainmap-release-reproducibility-v2}"
  jq -n \
    --arg schemaVersion "${schema}" \
    --arg candidateCommit "${commit}" \
    --arg brainmapSha256 "${brainmap_sha}" \
    --arg brainmapdSha256 "${brainmapd_sha}" \
    --arg buildInfoSha256 "${build_info_sha}" \
    --argjson producerDigests "${producer_digests}" \
    '{
      schemaVersion: $schemaVersion,
      candidateCommit: $candidateCommit,
      profile: "release",
      locked: true,
      twoRootByteIdentical: true,
      cleanTree: true,
      brainmapSha256: $brainmapSha256,
      brainmapdSha256: $brainmapdSha256,
      buildInfoSha256: $buildInfoSha256,
      producerDigests: $producerDigests
    }' >"${path}"
}

repro="${temporary}/reproducibility.json"
write_reproducibility_manifest "${repro}" "${candidate_commit}"

common=(
  --brainmap "${brainmap}"
  --brainmap-sha256 "${brainmap_sha}"
  --brainmapd "${brainmapd}"
  --brainmapd-sha256 "${brainmapd_sha}"
  --candidate-commit "${candidate_commit}"
  --reproducibility-manifest "${repro}"
)

expect_failure 'missing required --brainmap PATH' "${fixture}/scripts/m8-release-qualification.sh"
expect_failure 'binary paths must be absolute' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  --brainmap relative/brainmap \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${brainmapd}" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}" \
  --reproducibility-manifest "${repro}" \
  --out "${temporary}/relative-path-rejection"
expect_failure 'brainmap SHA-256 mismatch' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  --brainmap "${brainmap}" \
  --brainmap-sha256 0000000000000000000000000000000000000000000000000000000000000000 \
  --brainmapd "${brainmapd}" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${candidate_commit}" \
  --reproducibility-manifest "${repro}" \
  --out "${temporary}/hash-rejection"

bad_repro="${temporary}/bad-reproducibility.json"
write_reproducibility_manifest "${bad_repro}" "${candidate_commit}" wrong-schema
expect_failure 'invalid strict reproducibility manifest' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]:0:10}" \
  --reproducibility-manifest "${bad_repro}" \
  --out "${temporary}/repro-rejection"

non_head_repro="${temporary}/non-head-reproducibility.json"
write_reproducibility_manifest "${non_head_repro}" "${non_head_commit}"
expect_failure 'candidate commit must equal unchanged HEAD' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  --brainmap "${brainmap}" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${brainmapd}" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${non_head_commit}" \
  --reproducibility-manifest "${non_head_repro}" \
  --out "${temporary}/ref-rejection"

touch "${fixture}/untracked-dirty-file"
expect_failure 'qualification requires clean HEAD' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${temporary}/dirty-rejection"
rm "${fixture}/untracked-dirty-file"

expect_failure 'evidence output path must be absolute' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out relative/evidence
expect_failure 'evidence output path must be outside the repository' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${fixture}/release-evidence"
mkdir "${temporary}/existing-output"
expect_failure 'evidence directory already exists' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${temporary}/existing-output"
ln -s "${temporary}/missing-output-target" "${temporary}/dangling-output"
expect_failure 'evidence directory already exists' \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${temporary}/dangling-output"

failed_gate_out="${temporary}/failed-gate-evidence"
expect_failure 'gate format failed with exit 7' \
  env PATH="${failing_bin}:${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${failed_gate_out}"
[[ ! -e "${failed_gate_out}" ]] || fail 'failed gate published partial evidence'

secret_out="${temporary}/secret-evidence"
expect_failure 'sanitized evidence retains a secret-like value' \
  env PATH="${secret_bin}:${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${secret_out}"
[[ ! -e "${secret_out}" ]] || fail 'privacy failure published partial evidence'

email_out="${temporary}/email-evidence"
expect_failure 'sanitized evidence retains a secret-like value' \
  env PATH="${email_bin}:${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${email_out}"
[[ ! -e "${email_out}" ]] || fail 'email privacy failure published partial evidence'

non_utf8_out="${temporary}/non-utf8-evidence"
expect_failure 'retained artifact is not valid UTF-8' \
  env PATH="${non_utf8_bin}:${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
  "${common[@]}" \
  --out "${non_utf8_out}"
[[ ! -e "${non_utf8_out}" ]] || fail 'UTF-8 failure published partial evidence'

out="${temporary}/evidence"
PATH="${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
    "${common[@]}" \
    --out "${out}" >/dev/null

manifest="${out}/manifest.json"
[[ -f "${manifest}" ]] || fail 'release manifest is missing'
jq -e \
  --arg candidateCommit "${candidate_commit}" \
  --arg brainmapSha256 "${brainmap_sha}" \
  --arg brainmapdSha256 "${brainmapd_sha}" '
  type == "object"
  and (keys == [
    "candidate", "completedAt", "gates", "host", "privacy",
    "qualificationEligible", "reproducibilityManifestSha256",
    "schemaVersion", "sourceTreeDirtyAfter", "sourceTreeDirtyBefore",
    "startedAt", "toolchain"
  ])
  and .schemaVersion == "brainmap-m8-release-v1"
  and .qualificationEligible == true
  and .candidate == {
    commit: $candidateCommit,
    brainmapSha256: $brainmapSha256,
    brainmapdSha256: $brainmapdSha256
  }
  and .sourceTreeDirtyBefore == false
  and .sourceTreeDirtyAfter == false
  and (.startedAt | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T"))
  and (.completedAt | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T"))
  and ((.startedAt | fromdateiso8601) <= (.completedAt | fromdateiso8601))
  and (.host | keys == ["architecture", "kernelName", "kernelRelease"])
  and all(.host[]; type == "string" and length > 0)
  and (.toolchain | keys == ["cargo", "rustc"])
  and all(.toolchain[]; type == "string" and length > 0)
  and (.reproducibilityManifestSha256 | test("^[0-9a-f]{64}$"))
  and (.gates | keys == [
    "audit", "cleanWorktree", "clippy", "deny", "format",
    "lockedReleaseBuild", "packageSmoke", "performance", "sbom",
    "scale1000", "scale5000", "workspaceTests"
  ])
  and all(.gates[];
    type == "object"
    and (keys == ["path", "sha256"])
    and (.path | test("^gates/[a-z0-9-]+\\.json$"))
    and (.sha256 | test("^[0-9a-f]{64}$")))
  and .privacy == {
    rawPromptsRetained: false,
    secretsRetained: false,
    privatePathsRetained: false,
    syntheticInputsOnly: true
  }
' "${manifest}" >/dev/null || fail 'strict manifest shape is invalid'

declare -A expected_gate_paths=(
  [format]='gates/format.json'
  [clippy]='gates/clippy.json'
  [workspaceTests]='gates/workspace-tests.json'
  [audit]='gates/audit.json'
  [deny]='gates/deny.json'
  [sbom]='gates/sbom.json'
  [lockedReleaseBuild]='gates/locked-release-build.json'
  [packageSmoke]='gates/package-smoke.json'
  [scale1000]='gates/scale-1000.json'
  [scale5000]='gates/scale-5000.json'
  [performance]='gates/performance.json'
  [cleanWorktree]='gates/clean-worktree.json'
)
for gate_key in "${!expected_gate_paths[@]}"; do
  gate_path="${expected_gate_paths[${gate_key}]}"
  actual_path="$(jq -r --arg key "${gate_key}" '.gates[$key].path' "${manifest}")"
  [[ "${actual_path}" == "${gate_path}" ]] || fail "wrong gate path for ${gate_key}"
  gate_sha="$(sha256sum "${out}/${gate_path}" | cut -d ' ' -f 1)"
  manifest_gate_sha="$(jq -r --arg key "${gate_key}" '.gates[$key].sha256' "${manifest}")"
  [[ "${gate_sha}" == "${manifest_gate_sha}" ]] || fail "gate hash mismatch: ${gate_path}"

  gate_id="$(basename "${gate_path}" .json)"
  gate_log="${out}/gates/${gate_id}.log"
  [[ -f "${gate_log}" ]] || fail "gate log is missing: ${gate_id}"
  jq -e --arg gate "${gate_id}" '
    type == "object"
    and (keys == [
      "commandId", "exitCode", "gate", "logSha256", "passed", "schemaVersion"
    ])
    and .schemaVersion == "brainmap-m8-release-gate-result-v1"
    and .gate == $gate
    and .commandId == $gate
    and .passed == true
    and .exitCode == 0
    and (.logSha256 | test("^[0-9a-f]{64}$"))
  ' "${out}/${gate_path}" >/dev/null || fail "invalid gate result: ${gate_id}"
  expected_log_sha="$(jq -r '.logSha256' "${out}/${gate_path}")"
  actual_log_sha="$(sha256sum "${gate_log}" | cut -d ' ' -f 1)"
  [[ "${actual_log_sha}" == "${expected_log_sha}" ]] ||
    fail "log hash mismatch: ${gate_id}"
done

workspace_log="${out}/gates/workspace-tests.log"
for marker in \
  'integrated runner interface passed' \
  'release runner interface passed' \
  'Codex FIA-5 producer interface passed' \
  'qualification assembler interface passed' \
  'release reproducibility interface passed'; do
  grep -F -- "${marker}" "${workspace_log}" >/dev/null ||
    fail "workspace gate omitted interface suite: ${marker}"
done
grep -F -- 'mv -T -n' "${fixture}/scripts/m8-release-qualification.sh" >/dev/null ||
  fail 'release evidence publication must use no-replace mv semantics'

repro_sha="$(sha256sum "${out}/reproducibility-manifest.json" | cut -d ' ' -f 1)"
[[ "$(jq -r '.reproducibilityManifestSha256' "${manifest}")" == "${repro_sha}" ]] ||
  fail 'reproducibility manifest hash is not bound'
cmp "${fixture}/crates/brainmap-cli/brainmap.json" "${out}/sbom/brainmap.cdx.json" >/dev/null ||
  fail 'retained SBOM is not byte-identical to the tracked SBOM'
jq -e '
  .bomFormat == "CycloneDX"
  and (has("authors") or has("metadata"))
' "${out}/sbom/brainmap.cdx.json" >/dev/null ||
  fail 'retained SBOM does not satisfy the strict verifier shape'

jq -e '
  .cases >= 100
  and .falseProceed == 0
  and .falseAsk == 0
  and .falseBlock == 0
  and .wrongChoice == 0
  and .wrongRule == 0
  and .wrongMetadata == 0
' "${out}/qualification/eval.json" >/dev/null || fail 'eval evidence is invalid'
jq -e '.scaleRequested == 1000 and .gateP95Ms < 10' \
  "${out}/qualification/bench-1000.json" >/dev/null || fail '1k budget evidence is invalid'
jq -e '.scaleRequested == 5000 and .gateP95Ms < 25 and .indexRebuildMs < 1000' \
  "${out}/qualification/bench-5000.json" >/dev/null || fail '5k budget evidence is invalid'
[[ "$(find "${out}/qualification" -maxdepth 1 -type f \
  -name 'restore-fault-*-state.json' | wc -l)" -eq 8 ]] ||
  fail 'recovery evidence does not contain eight state files'
if grep -RIE '"(prompt|messages|transcript|situation|options|toolarguments)"' \
  "${out}" >/dev/null; then
  fail 'evidence retained a raw prompt or transcript field'
fi

(cd "${out}" && sha256sum -c SHA256SUMS >/dev/null) ||
  fail 'recursive checksums do not verify'
expected_paths="$(
  cd "${out}"
  find . -type f ! -name SHA256SUMS -print | sed 's#^\./##' | sort
)"
checksummed_paths="$(awk '{print $2}' "${out}/SHA256SUMS" | sort)"
[[ "${expected_paths}" == "${checksummed_paths}" ]] ||
  fail 'SHA256SUMS does not cover every retained artifact exactly once'

if grep -R -F -- "${temporary}" "${out}" >/dev/null; then
  fail 'evidence retained its temporary private path'
fi
grep -R -F -- '<release-staging>' "${out}/gates" >/dev/null ||
  fail 'interface fixture did not exercise evidence-path sanitization'
if grep -RIE '(/home/|/Users/|/tmp/|/opt/|/root/|[A-Za-z]:\\Users\\)' "${out}" >/dev/null; then
  fail 'evidence retained an absolute private path'
fi

[[ -z "$(git -C "${fixture}" status --porcelain --untracked-files=all)" ]] ||
  fail 'successful runner left the fixture worktree dirty'
[[ "$(git -C "${fixture}" rev-parse HEAD)" == "${candidate_commit}" ]] ||
  fail 'successful runner changed fixture HEAD'

# Author emails are not portable dogfood evidence, even inside the tracked
# CycloneDX document.
cat >"${fixture}/crates/brainmap-cli/brainmap.json" <<'EOF'
{"bomFormat":"CycloneDX","specVersion":"1.5","metadata":{},"components":[{"author":"Dependency Author <author@example.com>"}]}
EOF
git -C "${fixture}" add crates/brainmap-cli/brainmap.json
git -C "${fixture}" commit -qm 'fixture email-bearing SBOM candidate'
email_sbom_commit="$(git -C "${fixture}" rev-parse HEAD)"
write_candidate_binaries "${email_sbom_commit}"
email_sbom_repro="${temporary}/email-sbom-reproducibility.json"
write_reproducibility_manifest "${email_sbom_repro}" "${email_sbom_commit}"
email_sbom_out="${temporary}/email-sbom-evidence"
expect_failure 'sanitized evidence retains a secret-like value' \
  env PATH="${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
  --brainmap "${brainmap}" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${brainmapd}" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${email_sbom_commit}" \
  --reproducibility-manifest "${email_sbom_repro}" \
  --out "${email_sbom_out}"
[[ ! -e "${email_sbom_out}" ]] || fail 'email-bearing SBOM published partial evidence'

# Prove the producer rejects a document that calls itself CycloneDX but lacks
# the authors/metadata shape required by the strict Rust bundle verifier.
cat >"${fixture}/crates/brainmap-cli/brainmap.json" <<'EOF'
{"bomFormat":"CycloneDX","specVersion":"1.5","components":[]}
EOF
git -C "${fixture}" add crates/brainmap-cli/brainmap.json
git -C "${fixture}" commit -qm 'fixture malformed SBOM candidate'
bad_sbom_commit="$(git -C "${fixture}" rev-parse HEAD)"
write_candidate_binaries "${bad_sbom_commit}"
bad_sbom_repro="${temporary}/bad-sbom-reproducibility.json"
write_reproducibility_manifest "${bad_sbom_repro}" "${bad_sbom_commit}"
bad_sbom_out="${temporary}/bad-sbom-evidence"
expect_failure 'gate sbom failed with exit 1' \
  env PATH="${fake_bin}:${PATH}" \
  "${fixture}/scripts/m8-release-qualification.sh" \
  --brainmap "${brainmap}" \
  --brainmap-sha256 "${brainmap_sha}" \
  --brainmapd "${brainmapd}" \
  --brainmapd-sha256 "${brainmapd_sha}" \
  --candidate-commit "${bad_sbom_commit}" \
  --reproducibility-manifest "${bad_sbom_repro}" \
  --out "${bad_sbom_out}"
[[ ! -e "${bad_sbom_out}" ]] || fail 'invalid SBOM published partial evidence'

echo 'm8 release qualification interface tests passed (15 rejection classes + strict evidence)'
