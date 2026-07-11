#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C
umask 077

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
state_schema='brainmap-m8-codex-fia5-state-v2'
official_codex_version='codex-cli 0.144.0'
official_codex_target='x86_64-unknown-linux-musl'
official_codex_archive_sha256='6b03d2d89910874fa5be27b617621d7638f906e891fd8cb40af3d2876a8a36fd'
official_codex_binary_sha256='901923c1808a151f6926d41d703c17ad48815662cefb1c8d832a052c44271429'
safe_approval_policy='on-request'
safe_approvals_reviewer='user'
safe_sandbox_mode='workspace-write'
safe_workspace_write_network_access='false'
directive='Complete exactly this synthetic workflow using only the brainmap MCP server: call brainmap_decision_gate with intent would-ask-user, situation Choose formatter for synthetic FIA-5 project, options JSON array biome and prettier, risk low, reversible true, decisionType tooling, scope project:fia5, and dryRun false; follow its effective outcome, ask the human to choose biome, then record that first decision with chosen biome and wasAsked true; create feedback for the same first decision with chosen prettier and rejected biome and let the human approve it; preview the returned packet; apply that packet with approved true and let the human approve it; repeat the identical gate call, follow its effective proceed outcome and selected prettier option, then record that distinct second decision with chosen prettier and wasAsked false; report success and make no other tool call.'

usage() {
  cat <<'EOF'
Produce strict real-Codex FIA-5 evidence without retaining prompts or secrets.

Usage:
  scripts/m8-codex-fia5.sh prepare --brainmap PATH --brainmapd PATH --candidate-commit COMMIT --codex-archive PATH --state DIR
  scripts/m8-codex-fia5.sh prepare --fixture --brainmap PATH --brainmapd PATH --candidate-commit COMMIT --state DIR
  scripts/m8-codex-fia5.sh begin --state DIR
  scripts/m8-codex-fia5.sh finalize --state DIR --out DIR

Workflow:
  prepare   Create an isolated synthetic project and Brainmap vault.
  begin     After persisting normal Codex project trust, run installer dry-run,
            install, doctor, and create a one-shot normal Codex launcher.
  finalize  Derive the fixed 12-event lifecycle from Codex app-server thread
            records plus the Brainmap ledger, then atomically publish evidence.

Run the launcher printed by begin in a terminal. Accept the exact project hook
and the prompt-required Brainmap feedback/apply calls in the normal Codex UI.
No caller-supplied FIA result, event assertion, host transcript, or bypass mode
is accepted. `--fixture` exists only for parser tests and always emits
non-assemblable `qualificationEligible:false` evidence. The final tree contains
only allowlisted prompt-free derivatives.
EOF
}

die() {
  echo "m8 Codex FIA-5 producer: $*" >&2
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

sha256_text() {
  printf '%s' "$1" | sha256sum | cut -d ' ' -f 1
}

sha256_argv() {
  printf '%s\0' "$@" | sha256sum | cut -d ' ' -f 1
}

now_iso() {
  date -u +'%Y-%m-%dT%H:%M:%S.%NZ'
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

canonical_new_path() {
  local path="$1" label="$2" parent base canonical_parent
  [[ "${path}" == /* ]] || die "${label} path must be absolute"
  [[ ! -e "${path}" && ! -L "${path}" ]] || die "${label} path already exists: ${path}"
  parent="$(dirname "${path}")"
  base="$(basename "${path}")"
  [[ "${base}" =~ ^[A-Za-z0-9._-]+$ ]] ||
    die "${label} basename must use portable ASCII characters"
  [[ -d "${parent}" && ! -L "${parent}" ]] ||
    die "${label} parent must be an existing non-symlink directory"
  canonical_parent="$(canonical_directory_path "${parent}")" ||
    die "cannot canonicalize ${label} parent"
  [[ "${path}" == "${canonical_parent}/${base}" ]] || die "${label} path must be canonical"
  printf '%s\n' "${path}"
}

paths_overlap() {
  local left="$1" right="$2"
  [[ "${left}" == "${right}" || "${left}" == "${right}/"* || "${right}" == "${left}/"* ]]
}

reject_unsafe_path_text() {
  local label="$1" path="$2"
  [[ "${path}" != *$'\n'* && "${path}" != *$'\r'* && "${path}" != *"'"* ]] ||
    die "${label} contains a character unsupported by the strict evidence workflow"
}

require_regular_file() {
  local label="$1" path="$2"
  [[ -f "${path}" && ! -L "${path}" ]] ||
    die "${label} must be a non-symlink regular file: ${path}"
}

require_executable_file() {
  local label="$1" path="$2"
  require_regular_file "${label}" "${path}"
  [[ -x "${path}" ]] || die "${label} is not executable: ${path}"
}

require_clean_candidate() {
  local commit="$1" head
  [[ "${commit}" =~ ^[0-9a-f]{40}$ ]] ||
    die "candidate commit must be exactly 40 lowercase hexadecimal characters"
  git -C "${root}" cat-file -e "${commit}^{commit}" 2>/dev/null ||
    die "candidate commit does not resolve in this repository"
  head="$(git -C "${root}" rev-parse HEAD)"
  [[ "${commit}" == "${head}" ]] || die "candidate commit must equal unchanged HEAD"
  [[ -z "$(git -C "${root}" status --porcelain --untracked-files=all)" ]] ||
    die "FIA-5 production requires a clean candidate HEAD"
}

validate_official_codex() {
  local codex="$1" archive="$2" package_json extracted_sha
  require_executable_file "Codex executable" "${codex}"
  require_regular_file "official Codex archive" "${archive}"
  [[ "${archive}" == "$(canonical_file_path "${archive}")" ]] ||
    die "official Codex archive path must be canonical"
  [[ "$(sha256_file "${archive}")" == "${official_codex_archive_sha256}" ]] ||
    die "Codex archive is not the pinned official 0.144.0 x86_64-unknown-linux-musl package"
  package_json="$(tar -xOzf "${archive}" codex-package.json)" ||
    die "cannot read codex-package.json from the official archive"
  jq -e \
    --arg version "${official_codex_version#codex-cli }" \
    --arg target "${official_codex_target}" '
      .layoutVersion == 1
      and .version == $version
      and .target == $target
      and .variant == "codex"
      and .entrypoint == "bin/codex"
      and .resourcesDir == "codex-resources"
      and .pathDir == "codex-path"
    ' <<<"${package_json}" >/dev/null || die "official Codex package metadata is invalid"
  extracted_sha="$(tar -xOzf "${archive}" bin/codex | sha256sum | cut -d ' ' -f 1)" ||
    die "cannot hash bin/codex from the official archive"
  [[ "${extracted_sha}" == "${official_codex_binary_sha256}" ]] ||
    die "official Codex archive contains an unexpected bin/codex"
  [[ "$(sha256_file "${codex}")" == "${official_codex_binary_sha256}" ]] ||
    die "installed Codex binary is not byte-identical to the pinned official binary"
  [[ "$("${codex}" --version)" == "${official_codex_version}" ]] ||
    die "installed Codex version is not ${official_codex_version}"
  [[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] ||
    die "official Codex FIA-5 qualification requires Linux x86_64"
}

sync_tree() {
  local directory="$1" artifact
  while IFS= read -r -d '' artifact; do
    sync "${artifact}"
  done < <(find "${directory}" -type f -print0)
  sync "${directory}"
}

write_checksums() {
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
  local directory="$1" generated
  generated="$(mktemp)"
  write_checksums "${directory}" "${generated}"
  cmp -s "${generated}" "${directory}/SHA256SUMS" || {
    rm -f "${generated}"
    die "published SHA256SUMS is stale or incomplete"
  }
  rm -f "${generated}"
  (cd "${directory}" && sha256sum -c SHA256SUMS >/dev/null) ||
    die "published SHA256SUMS failed verification"
}

project_inventory_sha256() {
  local project="$1" invalid artifact relative mode
  invalid="$(find "${project}" -path "${project}/.git" -prune -o -type l -print -quit)"
  [[ -z "${invalid}" ]] || die "synthetic project inventory contains a symlink"
  invalid="$(find "${project}" -path "${project}/.git" -prune -o ! -type f ! -type d -print -quit)"
  [[ -z "${invalid}" ]] || die "synthetic project inventory contains a special entry"
  (
    cd "${project}"
    find . -mindepth 1 -path './.git' -prune -o \( -type f -o -type d \) -print0 |
      sort -z |
      while IFS= read -r -d '' artifact; do
        relative="${artifact#./}"
        mode="$(stat -c '%a' "${artifact}")"
        if [[ -d "${artifact}" ]]; then
          printf 'directory\0%s\0%s\0' "${relative}" "${mode}"
        else
          printf 'file\0%s\0%s\0%s\0' \
            "${relative}" "${mode}" "$(sha256_file "${relative}")"
        fi
      done
  ) | sha256sum | cut -d ' ' -f 1
}

state_value() {
  local state="$1" filter="$2"
  jq -er "${filter}" "${state}/state.json"
}

validate_state_tree() {
  local state="$1" expected_phase="$2"
  [[ "${state}" == /* ]] || die "state path must be absolute"
  [[ -d "${state}" && ! -L "${state}" ]] || die "state is not a non-symlink directory"
  [[ "${state}" == "$(canonical_directory_path "${state}")" ]] ||
    die "state path must be canonical"
  require_regular_file "state manifest" "${state}/state.json"
  jq -e \
    --arg schema "${state_schema}" \
    --arg phase "${expected_phase}" \
    --arg officialVersion "${official_codex_version}" \
    --arg officialTarget "${official_codex_target}" \
    --arg officialArchiveSha "${official_codex_archive_sha256}" \
    --arg officialBinarySha "${official_codex_binary_sha256}" '
      type == "object"
      and .schemaVersion == $schema
      and .phase == $phase
      and (.mode == "qualification" or .mode == "fixture")
      and (.qualificationEligible == (.mode == "qualification"))
      and (.candidate.commit | type == "string" and test("^[0-9a-f]{40}$"))
      and (.candidate.brainmapSha256 | type == "string" and test("^[0-9a-f]{64}$"))
      and (.candidate.brainmapdSha256 | type == "string" and test("^[0-9a-f]{64}$"))
      and .host.version == $officialVersion
      and .host.target == $officialTarget
      and (.host.codexSha256 | type == "string" and test("^[0-9a-f]{64}$"))
      and .host.officialArchiveSha256 == $officialArchiveSha
      and .host.officialBinarySha256 == $officialBinarySha
      and (.host.officialVerified == (.mode == "qualification"))
      and (.paths | keys == ["brainmap", "brainmapd", "codex", "codexArchive", "codexHome", "project", "vault"])
      and ([.paths.brainmap, .paths.brainmapd, .paths.codex, .paths.codexHome, .paths.project, .paths.vault]
        | all(type == "string" and startswith("/")))
      and (
        if .mode == "qualification" then
          (.paths.codexArchive | type == "string" and startswith("/"))
          and .host.codexSha256 == $officialBinarySha
        else
          .paths.codexArchive == null
        end
      )
      and (.preparedAt | type == "string")
      and (.workflowSha256 | type == "string" and test("^[0-9a-f]{64}$"))
      and (.directiveSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    ' "${state}/state.json" >/dev/null || die "invalid ${expected_phase} FIA-5 state"
}

validate_state_candidate() {
  local state="$1" mode brainmap brainmapd codex codex_archive codex_home current_codex_home project vault commit version
  mode="$(state_value "${state}" '.mode')"
  brainmap="$(state_value "${state}" '.paths.brainmap')"
  brainmapd="$(state_value "${state}" '.paths.brainmapd')"
  codex="$(state_value "${state}" '.paths.codex')"
  codex_archive="$(jq -r '.paths.codexArchive // empty' "${state}/state.json")"
  codex_home="$(state_value "${state}" '.paths.codexHome')"
  project="$(state_value "${state}" '.paths.project')"
  vault="$(state_value "${state}" '.paths.vault')"
  commit="$(state_value "${state}" '.candidate.commit')"
  for pair in \
    "brainmap|${brainmap}" \
    "brainmapd|${brainmapd}" \
    "Codex executable|${codex}"; do
    require_executable_file "${pair%%|*}" "${pair#*|}"
  done
  [[ "$(sha256_file "${brainmap}")" == "$(state_value "${state}" '.candidate.brainmapSha256')" ]] ||
    die "brainmap candidate changed after prepare"
  [[ "$(sha256_file "${brainmapd}")" == "$(state_value "${state}" '.candidate.brainmapdSha256')" ]] ||
    die "brainmapd candidate changed after prepare"
  [[ "$(sha256_file "${codex}")" == "$(state_value "${state}" '.host.codexSha256')" ]] ||
    die "Codex executable changed after prepare"
  version="$("${codex}" --version)"
  [[ "${version}" == "${official_codex_version}" &&
     "${version}" == "$(state_value "${state}" '.host.version')" ]] ||
    die "Codex version changed after prepare"
  if [[ "${mode}" == qualification ]]; then
    validate_official_codex "${codex}" "${codex_archive}"
  else
    [[ "$(jq -r '.qualificationEligible' "${state}/state.json")" == false ]] ||
      die "fixture state can never be qualification eligible"
  fi
  [[ -d "${codex_home}" && ! -L "${codex_home}" &&
     "${codex_home}" == "$(canonical_directory_path "${codex_home}")" ]] ||
    die "CODEX_HOME changed or is not canonical"
  current_codex_home="${CODEX_HOME:-${HOME:?HOME is required}/.codex}"
  [[ "${current_codex_home}" == /* && -d "${current_codex_home}" &&
     ! -L "${current_codex_home}" ]] || die "current CODEX_HOME is not canonical"
  current_codex_home="$(canonical_directory_path "${current_codex_home}")"
  [[ "${current_codex_home}" == "${codex_home}" ]] ||
    die "current CODEX_HOME does not match the prepared Codex host"
  [[ -d "${project}" && ! -L "${project}" &&
     "${project}" == "$(canonical_directory_path "${project}")" ]] ||
    die "synthetic project changed or is not canonical"
  [[ -d "${vault}" && ! -L "${vault}" &&
     "${vault}" == "$(canonical_directory_path "${vault}")" ]] ||
    die "synthetic vault changed or is not canonical"
  require_clean_candidate "${commit}"
}

prepare_state() {
  local brainmap='' brainmapd='' candidate_commit='' codex_archive='' state='' fixture=false option
  while (($#)); do
    case "$1" in
      --fixture)
        fixture=true
        shift
        ;;
      --brainmap|--brainmapd|--candidate-commit|--codex-archive|--state)
        option="$1"
        require_value "${option}" "${2:-}"
        case "${option}" in
          --brainmap) brainmap="$2" ;;
          --brainmapd) brainmapd="$2" ;;
          --candidate-commit) candidate_commit="$2" ;;
          --codex-archive) codex_archive="$2" ;;
          --state) state="$2" ;;
        esac
        shift 2
        ;;
      *) die "unknown prepare argument: $1" ;;
    esac
  done
  [[ -n "${brainmap}" ]] || die "prepare requires --brainmap PATH"
  [[ -n "${brainmapd}" ]] || die "prepare requires --brainmapd PATH"
  [[ -n "${candidate_commit}" ]] || die "prepare requires --candidate-commit COMMIT"
  [[ -n "${state}" ]] || die "prepare requires --state DIR"
  if [[ "${fixture}" == true ]]; then
    [[ -z "${codex_archive}" ]] || die "fixture mode does not accept --codex-archive"
  else
    [[ -n "${codex_archive}" ]] || die "qualification prepare requires --codex-archive PATH"
  fi

  for command in basename cut date dirname find git jq mktemp mv pwd readlink sha256sum sync tar uname; do
    require_command "${command}"
  done
  [[ "${brainmap}" == /* && "${brainmapd}" == /* ]] ||
    die "candidate executable paths must be absolute"
  require_executable_file "brainmap" "${brainmap}"
  require_executable_file "brainmapd" "${brainmapd}"
  [[ "${brainmap}" == "$(canonical_file_path "${brainmap}")" ]] ||
    die "brainmap path must be canonical"
  [[ "${brainmapd}" == "$(canonical_file_path "${brainmapd}")" ]] ||
    die "brainmapd path must be canonical"
  reject_unsafe_path_text "brainmap path" "${brainmap}"
  reject_unsafe_path_text "brainmapd path" "${brainmapd}"
  require_clean_candidate "${candidate_commit}"

  local codex_lookup codex codex_home host_version codex_sha
  codex_lookup="$(command -v codex)" || die "Codex CLI is not installed"
  codex="$(readlink -f "${codex_lookup}")"
  require_executable_file "Codex executable" "${codex}"
  [[ "${codex}" == "$(canonical_file_path "${codex}")" ]] ||
    die "Codex executable path must be canonical"
  reject_unsafe_path_text "Codex executable path" "${codex}"
  host_version="$("${codex}" --version)"
  [[ "${host_version}" == "${official_codex_version}" ]] ||
    die "Codex CLI must be the pinned ${official_codex_version}"
  codex_sha="$(sha256_file "${codex}")"
  if [[ "${fixture}" == false ]]; then
    [[ "${codex_archive}" == /* ]] || die "official Codex archive path must be absolute"
    validate_official_codex "${codex}" "${codex_archive}"
    codex_archive="$(canonical_file_path "${codex_archive}")"
  fi

  codex_home="${CODEX_HOME:-${HOME:?HOME is required}/.codex}"
  [[ "${codex_home}" == /* ]] || die "CODEX_HOME must be absolute"
  [[ -d "${codex_home}" && ! -L "${codex_home}" ]] ||
    die "CODEX_HOME must be an existing non-symlink directory"
  codex_home="$(canonical_directory_path "${codex_home}")"
  reject_unsafe_path_text "CODEX_HOME" "${codex_home}"

  state="$(canonical_new_path "${state}" state)"
  reject_unsafe_path_text "state path" "${state}"
  paths_overlap "${state}" "${root}" && die "state must be outside the candidate repository"
  paths_overlap "${state}" "${codex_home}" && die "state must be outside CODEX_HOME"

  local state_parent container staging private project vault final_project final_vault workflow prepared_at
  state_parent="$(dirname "${state}")"
  container="$(mktemp -d "${state_parent}/.brainmap-m8-codex-fia5.XXXXXX")"
  staging="${container}/state"
  private="${staging}/private"
  project="${private}/project"
  vault="${private}/vault"
  final_project="${state}/private/project"
  final_vault="${state}/private/vault"
  cleanup_prepare() {
    rm -rf "${container:-}"
  }
  trap cleanup_prepare EXIT HUP INT TERM
  mkdir -p "${project}" "${private}/normalized"

  workflow=$'# Synthetic Brainmap FIA-5 workflow\n\nThis project exists only for M001 host qualification. Use the `brainmap` MCP server only; do not use shell commands or edit files.\n\n1. Call `brainmap_decision_gate` with exactly: intent `would-ask-user`; situation `Choose formatter for synthetic FIA-5 project`; options JSON array `["biome", "prettier"]`; risk `low`; reversible `true`; decision type `tooling`; scope `project:fia5`; dry run `false`.\n2. Follow the initial effective `ask_user` outcome. Ask the human to choose `biome`, then call `brainmap_record_decision` for that first decision ID with chosen `biome` and `wasAsked: true`.\n3. Call `brainmap_learn_feedback` for the same first decision ID with chosen `prettier` and rejected `biome`. Let the human approve the tool call.\n4. Call `brainmap_preview_update` with the returned packet ID.\n5. Call `brainmap_apply_update` with that packet ID and `approved: true`. Let the human approve the tool call.\n6. Repeat step 1 exactly. Follow the active effective `proceed` outcome and selected `prettier` option, then call `brainmap_record_decision` for the distinct second decision ID with chosen `prettier` and `wasAsked: false`. Do nothing else.\n'
  printf '%s' "${workflow}" >"${project}/FIA5-WORKFLOW.md"
  git -C "${project}" init -q
  git -C "${project}" config user.name 'Brainmap FIA-5'
  git -C "${project}" config user.email 'fia5@brainmap.invalid'
  git -C "${project}" add FIA5-WORKFLOW.md
  git -C "${project}" commit -qm 'add fixed synthetic FIA-5 workflow'

  "${brainmap}" init-vault --vault "${vault}" --yes >/dev/null
  "${brainmap}" index rebuild --vault "${vault}" >/dev/null
  prepared_at="$(now_iso)"
  jq -n \
    --arg schema "${state_schema}" \
    --arg commit "${candidate_commit}" \
    --arg brainmapSha "$(sha256_file "${brainmap}")" \
    --arg brainmapdSha "$(sha256_file "${brainmapd}")" \
    --arg version "${host_version}" \
    --arg target "${official_codex_target}" \
    --arg codexSha "${codex_sha}" \
    --arg officialArchiveSha "${official_codex_archive_sha256}" \
    --arg officialBinarySha "${official_codex_binary_sha256}" \
    --arg mode "$(if [[ "${fixture}" == true ]]; then printf fixture; else printf qualification; fi)" \
    --argjson qualificationEligible "$(if [[ "${fixture}" == true ]]; then printf false; else printf true; fi)" \
    --argjson officialVerified "$(if [[ "${fixture}" == true ]]; then printf false; else printf true; fi)" \
    --arg brainmap "${brainmap}" \
    --arg brainmapd "${brainmapd}" \
    --arg codex "${codex}" \
    --arg codexArchive "${codex_archive}" \
    --arg codexHome "${codex_home}" \
    --arg project "${final_project}" \
    --arg vault "${final_vault}" \
    --arg preparedAt "${prepared_at}" \
    --arg workflowSha "$(sha256_text "${workflow}")" \
    --arg directiveSha "$(sha256_text "${directive}")" '{
      schemaVersion: $schema,
      phase: "prepared",
      mode: $mode,
      qualificationEligible: $qualificationEligible,
      candidate: {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      },
      host: {
        version: $version,
        target: $target,
        codexSha256: $codexSha,
        officialArchiveSha256: $officialArchiveSha,
        officialBinarySha256: $officialBinarySha,
        officialVerified: $officialVerified
      },
      paths: {
        brainmap: $brainmap,
        brainmapd: $brainmapd,
        codex: $codex,
        codexArchive: (if $codexArchive == "" then null else $codexArchive end),
        codexHome: $codexHome,
        project: $project,
        vault: $vault
      },
      preparedAt: $preparedAt,
      workflowSha256: $workflowSha,
      directiveSha256: $directiveSha
    }' >"${staging}/state.json"
  sync_tree "${staging}"
  mv "${staging}" "${state}"
  sync "${state_parent}"
  trap - EXIT HUP INT TERM
  rm -rf "${container}"

  printf 'Prepared synthetic FIA-5 state.\n'
  if [[ "${fixture}" == true ]]; then
    printf 'FIXTURE MODE: all resulting evidence is qualificationEligible=false.\n'
  fi
  printf 'Persist normal Codex project trust for: %s\n' "${state}/private/project"
  printf 'Then run: %s begin --state %s\n' "$0" "${state}"
}

validate_doctor_output() {
  local doctor_output="$1"
  jq -e '
    type == "object"
    and .target == "codex"
    and .supported == true
    and .installed == true
    and .configurationValid == true
    and .executableAvailable == true
    and .vaultExists == true
    and .indexValid == true
    and .gateReachable == true
    and .recordingSupported == true
    and .feedbackSupported == true
    and .activationRequiresApproval == true
    and .mcpVaultConfigured == true
    and .projectTrustRequired == true
    and .projectTrusted == true
    and .projectTrustConfigurationValid == true
    and .projectTrustError == null
    and .healthScope == "local-adapter-files-and-contract"
    and .hostHookTrustVerified == false
    and .hostProbeRequired == true
    and .enforcement == ["instruction-only", "instruction-only", "best-effort", "enforced"]
    and .healthy == true
  ' <<<"${doctor_output}" >/dev/null
}

expected_dry_run_output() {
  local project="$1"
  printf '%s\n' \
    'install harness dry-run target=codex' \
    "would create ${project}/.codex/skills/build-decision-engine/SKILL.md (instruction-only)" \
    "would create ${project}/AGENTS.md (instruction-only)" \
    "would create ${project}/.codex/config.toml (best-effort)" \
    "would create ${project}/.codex/hooks.json (enforced)"
}

expected_install_output() {
  local project="$1"
  printf '%s\n' \
    "wrote ${project}/.codex/skills/build-decision-engine/SKILL.md (instruction-only)" \
    "wrote ${project}/AGENTS.md (instruction-only)" \
    "wrote ${project}/.codex/config.toml (best-effort)" \
    "wrote ${project}/.codex/hooks.json (enforced)"
}

begin_state() {
  local state='' option
  while (($#)); do
    case "$1" in
      --state)
        option="$1"
        require_value "${option}" "${2:-}"
        state="$2"
        shift 2
        ;;
      *) die "unknown begin argument: $1" ;;
    esac
  done
  [[ -n "${state}" ]] || die "begin requires --state DIR"
  for command in cmp cut date find jq sha256sum stat sync; do
    require_command "${command}"
  done
  validate_state_tree "${state}" prepared
  validate_state_candidate "${state}"
  [[ -z "${BRAINMAP_DISABLE_AUTOPILOT:-}" && -z "${BRAINMAP_GATE_MODE:-}" ]] ||
    die "qualification environment cannot override Brainmap gate or autopilot mode"

  local brainmap codex codex_home project vault dry_output install_output doctor_output
  local dry_expected install_expected qualification_started installed_at doctor_at ledger
  brainmap="$(state_value "${state}" '.paths.brainmap')"
  codex="$(state_value "${state}" '.paths.codex')"
  codex_home="$(state_value "${state}" '.paths.codexHome')"
  project="$(state_value "${state}" '.paths.project')"
  vault="$(state_value "${state}" '.paths.vault')"
  [[ "$(sha256_file "${project}/FIA5-WORKFLOW.md")" == \
     "$(state_value "${state}" '.workflowSha256')" ]] ||
    die "fixed synthetic workflow changed after prepare"
  [[ -z "$(git -C "${project}" status --porcelain --untracked-files=all)" ]] ||
    die "synthetic project changed before adapter installation"

  "${brainmap}" gate-mode active --vault "${vault}" >/dev/null
  "${brainmap}" autopilot demote --to conservative --vault "${vault}" >/dev/null
  local engine_status
  engine_status="$("${brainmap}" autopilot status --vault "${vault}")" ||
    die "cannot read active Brainmap qualification mode"
  jq -e '
    .gateMode == "active"
    and .mode == "conservative"
    and .killSwitch == false
  ' <<<"${engine_status}" >/dev/null ||
    die "Brainmap FIA-5 requires active gate mode with conservative autopilot"

  qualification_started="$(now_iso)"
  dry_output="$("${brainmap}" install harness --target codex --project "${project}" \
    --vault "${vault}" --dry-run)" || die "Codex adapter dry-run failed"
  dry_expected="$(expected_dry_run_output "${project}")"
  [[ "${dry_output}" == "${dry_expected}" ]] ||
    die "installer dry-run did not report the exact four fresh project changes"

  install_output="$("${brainmap}" install harness --target codex --project "${project}" \
    --vault "${vault}")" || die "Codex adapter installation failed"
  install_expected="$(expected_install_output "${project}")"
  [[ "${install_output}" == "${install_expected}" ]] ||
    die "installer did not report the exact four fresh project writes"
  installed_at="$(now_iso)"

  doctor_output="$("${brainmap}" integration doctor --target codex --project "${project}" \
    --vault "${vault}")" || die "Brainmap integration doctor is unhealthy"
  validate_doctor_output "${doctor_output}" ||
    die "Brainmap integration doctor did not prove the strict local adapter contract"
  doctor_at="$(now_iso)"

  for artifact in \
    "${project}/.codex/skills/build-decision-engine/SKILL.md" \
    "${project}/AGENTS.md" \
    "${project}/.codex/config.toml" \
    "${project}/.codex/hooks.json"; do
    require_regular_file "installed adapter artifact" "${artifact}"
  done
  jq -e 'type == "object" and (.hooks | type == "object")' \
    "${project}/.codex/hooks.json" >/dev/null || die "installed Codex hooks are invalid"

  query_safe_config "${state}"
  local config_summary="${CONFIG_SUMMARY}"

  local baseline_params baseline_result baseline_thread_ids
  baseline_params="$(jq -nc --arg project "${project}" '{
    cwd:$project,
    sourceKinds:["cli"],
    sortKey:"created_at",
    sortDirection:"asc",
    limit:100,
    useStateDbOnly:false
  }')"
  codex_rpc "${codex}" thread/list "${baseline_params}" "${codex_home}"
  baseline_result="${RPC_RESULT}"
  baseline_thread_ids="$(jq -ce \
    --arg project "${project}" '
      select(.nextCursor == null)
      | select(all(.data[];
          .source == "cli"
          and .cwd == $project
          and (.id | type == "string" and length > 0)
        ))
      | [.data[].id]
      | select(length == (unique | length))
    ' <<<"${baseline_result}")" ||
    die "could not establish a complete pre-launch Codex thread boundary"
  unset RPC_RESULT

  jq -n \
    --arg commit "$(state_value "${state}" '.candidate.commit')" \
    --arg brainmapSha "$(state_value "${state}" '.candidate.brainmapSha256')" \
    --arg brainmapdSha "$(state_value "${state}" '.candidate.brainmapdSha256')" '{
      schemaVersion: "brainmap-m8-host-install-dry-run-v1",
      target: "codex",
      dryRun: true,
      candidate: {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      }
    }' >"${state}/private/normalized/install-dry-run.json"
  jq -n \
    --arg commit "$(state_value "${state}" '.candidate.commit')" \
    --arg brainmapSha "$(state_value "${state}" '.candidate.brainmapSha256')" \
    --arg brainmapdSha "$(state_value "${state}" '.candidate.brainmapdSha256')" '{
      schemaVersion: "brainmap-m8-host-doctor-v1",
      target: "codex",
      healthy: true,
      healthScope: "local-adapter-files-and-contract",
      hostHookTrustVerified: false,
      hostProbeRequired: true,
      candidate: {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      }
    }' >"${state}/private/normalized/doctor.json"

  ledger="${vault}/90-calibration/decision-ledger.jsonl"
  require_regular_file "Brainmap decision ledger" "${ledger}"
  [[ ! -s "${ledger}" || "$(tail -c 1 "${ledger}")" == '' ]] ||
    die "Brainmap decision ledger is not newline-complete"
  local boundary_bytes boundary_lines boundary_sha
  boundary_bytes="$(wc -c <"${ledger}" | tr -d ' ')"
  boundary_lines="$(wc -l <"${ledger}" | tr -d ' ')"
  boundary_sha="$(sha256_file "${ledger}")"

  cat >"${state}/launch-codex.sh" <<'LAUNCHER'
#!/usr/bin/env bash
set -euo pipefail
umask 077
export LC_ALL=C

[[ "$#" -eq 0 ]] || {
  echo 'FIA-5 launcher accepts no arguments' >&2
  exit 64
}
state="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
manifest="${state}/state.json"
[[ "$(jq -r '.phase' "${manifest}")" == ready ]] || {
  echo 'FIA-5 state is not ready' >&2
  exit 65
}
codex="$(jq -er '.paths.codex' "${manifest}")"
project="$(jq -er '.paths.project' "${manifest}")"
codex_home="$(jq -er '.paths.codexHome' "${manifest}")"
expected_codex_sha="$(jq -er '.host.codexSha256' "${manifest}")"
expected_directive_sha="$(jq -er '.directiveSha256' "${manifest}")"
expected_workflow_sha="$(jq -er '.workflowSha256' "${manifest}")"
expected_project_inventory_sha="$(jq -er '.projectInventorySha256' "${manifest}")"
directive='Complete exactly this synthetic workflow using only the brainmap MCP server: call brainmap_decision_gate with intent would-ask-user, situation Choose formatter for synthetic FIA-5 project, options JSON array biome and prettier, risk low, reversible true, decisionType tooling, scope project:fia5, and dryRun false; follow its effective outcome, ask the human to choose biome, then record that first decision with chosen biome and wasAsked true; create feedback for the same first decision with chosen prettier and rejected biome and let the human approve it; preview the returned packet; apply that packet with approved true and let the human approve it; repeat the identical gate call, follow its effective proceed outcome and selected prettier option, then record that distinct second decision with chosen prettier and wasAsked false; report success and make no other tool call.'

project_inventory_sha256() {
  local invalid artifact relative mode
  invalid="$(find "${project}" -path "${project}/.git" -prune -o -type l -print -quit)"
  [[ -z "${invalid}" ]] || return 1
  invalid="$(find "${project}" -path "${project}/.git" -prune -o ! -type f ! -type d -print -quit)"
  [[ -z "${invalid}" ]] || return 1
  (
    cd "${project}"
    find . -mindepth 1 -path './.git' -prune -o \( -type f -o -type d \) -print0 |
      sort -z |
      while IFS= read -r -d '' artifact; do
        relative="${artifact#./}"
        mode="$(stat -c '%a' "${artifact}")"
        if [[ -d "${artifact}" ]]; then
          printf 'directory\0%s\0%s\0' "${relative}" "${mode}"
        else
          printf 'file\0%s\0%s\0%s\0' \
            "${relative}" "${mode}" \
            "$(sha256sum "${relative}" | cut -d ' ' -f 1)"
        fi
      done
  ) | sha256sum | cut -d ' ' -f 1
}

actual_codex_sha="$(sha256sum "${codex}" | cut -d ' ' -f 1)"
actual_directive_sha="$(printf '%s' "${directive}" | sha256sum | cut -d ' ' -f 1)"
actual_workflow_sha="$(sha256sum "${project}/FIA5-WORKFLOW.md" | cut -d ' ' -f 1)"
actual_project_inventory_sha="$(project_inventory_sha256)" || {
  echo 'synthetic project inventory is unsafe before launch' >&2
  exit 66
}
[[ "${actual_codex_sha}" == "${expected_codex_sha}" ]] || {
  echo 'Codex executable changed after begin' >&2
  exit 66
}
[[ "${actual_directive_sha}" == "${expected_directive_sha}" ]] || {
  echo 'fixed workflow directive changed after begin' >&2
  exit 67
}
[[ "${actual_workflow_sha}" == "${expected_workflow_sha}" ]] || {
  echo 'fixed synthetic workflow changed before launch' >&2
  exit 68
}
[[ "${actual_project_inventory_sha}" == "${expected_project_inventory_sha}" ]] || {
  echo 'synthetic project inventory changed before launch' >&2
  exit 69
}
[[ -d "${codex_home}" && ! -L "${codex_home}" ]] || {
  echo 'prepared CODEX_HOME is unavailable or unsafe' >&2
  exit 70
}
[[ -z "${BRAINMAP_DISABLE_AUTOPILOT:-}" && -z "${BRAINMAP_GATE_MODE:-}" ]] || {
  echo 'Brainmap qualification mode cannot be overridden' >&2
  exit 71
}
export CODEX_HOME="${codex_home}"
marker="${state}/private/launch-marker.json"
[[ ! -e "${marker}" && ! -L "${marker}" ]] || {
  echo 'FIA-5 launcher is one-shot and was already used' >&2
  exit 72
}
started_at="$(date -u +'%Y-%m-%dT%H:%M:%S.%NZ')"
started_epoch="$(date -u +'%s')"
set -C
jq -n \
  --arg startedAt "${started_at}" \
  --argjson startedAtEpoch "${started_epoch}" \
  --arg executable "${codex}" \
  --arg codexSha "${actual_codex_sha}" \
  --arg codexHome "${codex_home}" \
  --arg projectInventorySha "${actual_project_inventory_sha}" \
  --arg project "${project}" \
  --arg directive "${directive}" '{
    schemaVersion: "brainmap-m8-codex-fia5-launch-v2",
    startedAt: $startedAt,
    startedAtEpoch: $startedAtEpoch,
    executable: $executable,
    codexSha256: $codexSha,
    codexHome: $codexHome,
    projectInventorySha256: $projectInventorySha,
    argv: [
      "--ask-for-approval", "on-request",
      "--sandbox", "workspace-write",
      "-c", "approvals_reviewer=\"user\"",
      "-c", "sandbox_workspace_write.network_access=false",
      "--cd", $project,
      "--no-alt-screen", $directive
    ]
  }' >"${marker}"
set +C
sync "${marker}"
sync "$(dirname "${marker}")"
exec "${codex}" \
  --ask-for-approval on-request \
  --sandbox workspace-write \
  -c 'approvals_reviewer="user"' \
  -c 'sandbox_workspace_write.network_access=false' \
  --cd "${project}" \
  --no-alt-screen \
  "${directive}"
LAUNCHER
  chmod 0700 "${state}/launch-codex.sh"
  local launcher_sha project_inventory_sha state_tmp
  launcher_sha="$(sha256_file "${state}/launch-codex.sh")"
  project_inventory_sha="$(project_inventory_sha256 "${project}")"
  state_tmp="${state}/.state.json.new"
  jq \
    --arg qualificationStartedAt "${qualification_started}" \
    --arg installedAt "${installed_at}" \
    --arg doctorAt "${doctor_at}" \
    --argjson boundaryBytes "${boundary_bytes}" \
    --argjson boundaryLines "${boundary_lines}" \
    --arg boundarySha "${boundary_sha}" \
    --arg installDryRunSha "$(sha256_file "${state}/private/normalized/install-dry-run.json")" \
    --arg doctorSha "$(sha256_file "${state}/private/normalized/doctor.json")" \
    --arg launcherSha "${launcher_sha}" \
    --arg projectInventorySha "${project_inventory_sha}" \
    --argjson configSummary "${config_summary}" \
    --argjson baselineThreadIds "${baseline_thread_ids}" '
      .phase = "ready"
      | .qualificationStartedAt = $qualificationStartedAt
      | .installedAt = $installedAt
      | .doctorAt = $doctorAt
      | .ledgerBoundary = {
          bytes: $boundaryBytes,
          lines: $boundaryLines,
          sha256: $boundarySha
        }
      | .normalized = {
          installDryRunSha256: $installDryRunSha,
          doctorSha256: $doctorSha
        }
      | .launcherSha256 = $launcherSha
      | .baselineThreadIds = $baselineThreadIds
      | .projectInventorySha256 = $projectInventorySha
      | .engine = {
          gateMode: "active",
          autopilotMode: "conservative"
        }
      | .config = $configSummary
    ' "${state}/state.json" >"${state_tmp}"
  sync "${state_tmp}"
  mv "${state_tmp}" "${state}/state.json"
  sync "${state}/state.json" "${state}"

  printf 'FIA-5 setup is ready. Run this one-shot launcher in a terminal:\n%s\n' \
    "${state}/launch-codex.sh"
  printf 'After the fixed workflow finishes and Codex exits, run:\n%s finalize --state %s --out /absolute/new/evidence-dir\n' \
    "$0" "${state}"
}

rpc_read_id() {
  local fd="$1" expected_id="$2" line
  while IFS= read -r -t 20 -u "${fd}" line; do
    jq -e . >/dev/null 2>&1 <<<"${line}" || die "Codex app-server emitted invalid JSON"
    if jq -e --argjson id "${expected_id}" '.id == $id' >/dev/null <<<"${line}"; then
      jq -e 'has("result") and (has("error") | not)' >/dev/null <<<"${line}" ||
        die "Codex app-server request ${expected_id} failed"
      printf '%s\n' "${line}"
      return 0
    fi
  done
  die "Codex app-server did not answer request ${expected_id}"
}

codex_rpc() {
  local codex="$1" method="$2" params="$3" expected_home="$4" read_fd write_fd pid response
  coproc CODEX_FIA5_RPC {
    CODEX_HOME="${expected_home}" "${codex}" \
      -c "approval_policy=\"${safe_approval_policy}\"" \
      -c "approvals_reviewer=\"${safe_approvals_reviewer}\"" \
      -c "sandbox_mode=\"${safe_sandbox_mode}\"" \
      -c "sandbox_workspace_write.network_access=${safe_workspace_write_network_access}" \
      app-server --stdio 2>/dev/null
  }
  read_fd="${CODEX_FIA5_RPC[0]}"
  write_fd="${CODEX_FIA5_RPC[1]}"
  pid="${CODEX_FIA5_RPC_PID}"
  jq -nc '{id:1,method:"initialize",params:{clientInfo:{name:"brainmap-fia5",version:"1"},capabilities:{experimentalApi:true}}}' \
    >&"${write_fd}"
  response="$(rpc_read_id "${read_fd}" 1)"
  jq -e --arg expectedHome "${expected_home}" \
    '.result.codexHome == $expectedHome' >/dev/null <<<"${response}" ||
    die "Codex app-server initialized with a different CODEX_HOME"
  jq -nc '{method:"initialized"}' >&"${write_fd}"
  jq -nc --arg method "${method}" --argjson params "${params}" \
    '{id:2,method:$method,params:$params}' >&"${write_fd}"
  response="$(rpc_read_id "${read_fd}" 2)"
  exec {write_fd}>&-
  kill "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
  exec {read_fd}>&-
  RPC_RESULT="$(jq -c '.result' <<<"${response}")"
}

query_safe_config() {
  local state="$1" codex codex_home project brainmap vault params config
  codex="$(state_value "${state}" '.paths.codex')"
  codex_home="$(state_value "${state}" '.paths.codexHome')"
  project="$(state_value "${state}" '.paths.project')"
  brainmap="$(state_value "${state}" '.paths.brainmap')"
  vault="$(state_value "${state}" '.paths.vault')"
  params="$(jq -nc --arg project "${project}" '{cwd:$project,includeLayers:true}')"
  codex_rpc "${codex}" config/read "${params}" "${codex_home}"
  config="${RPC_RESULT}"
  CONFIG_SUMMARY="$(jq -ce \
    --arg brainmap "${brainmap}" \
    --arg vault "${vault}" \
    --arg approvalPolicy "${safe_approval_policy}" \
    --arg approvalsReviewer "${safe_approvals_reviewer}" \
    --arg sandboxMode "${safe_sandbox_mode}" '
      .config as $config
      | select($config.approval_policy == $approvalPolicy)
      | select($config.approvals_reviewer == $approvalsReviewer)
      | select($config.sandbox_mode == $sandboxMode)
      | select($config.sandbox_workspace_write.network_access == false)
      | select($config.mcp_servers.brainmap.command == $brainmap)
      | select($config.mcp_servers.brainmap.args == ["mcp", "serve", "--vault", $vault])
      | select($config.mcp_servers.brainmap.required == true)
      | select($config.mcp_servers.brainmap.default_tools_approval_mode == "auto")
      | select($config.mcp_servers.brainmap.tools.brainmap_learn_feedback.approval_mode == "prompt")
      | select($config.mcp_servers.brainmap.tools.brainmap_apply_update.approval_mode == "prompt")
      | [
          $config | .. | objects | to_entries[]
          | select(.key | test("bypass"; "i"))
        ] as $bypassFields
      | select(all($bypassFields[]; .value == false or .value == null))
      | select([
          $config | .. | scalars
          | select(. == "danger-full-access" or . == "fullAccess")
        ] | length == 0)
      | {
          approvalPolicy: $config.approval_policy,
          approvalsReviewer: $config.approvals_reviewer,
          sandboxMode: $config.sandbox_mode,
          workspaceWriteNetworkAccess: $config.sandbox_workspace_write.network_access,
          bypassHookTrust: false,
          bypassApprovalsAndSandbox: false,
          feedbackApprovalMode: $config.mcp_servers.brainmap.tools.brainmap_learn_feedback.approval_mode,
          applyApprovalMode: $config.mcp_servers.brainmap.tools.brainmap_apply_update.approval_mode
        }
    ' <<<"${config}")" ||
    die "Codex effective config is unsafe, bypassed, or not bound to the exact Brainmap adapter"
  unset RPC_RESULT
}

validate_ready_state_shape() {
  local state="$1"
  jq -e '
    (.qualificationStartedAt | type == "string")
    and (.installedAt | type == "string")
    and (.doctorAt | type == "string")
    and (.ledgerBoundary.bytes | type == "number" and . >= 0 and floor == .)
    and (.ledgerBoundary.lines | type == "number" and . >= 0 and floor == .)
    and (.ledgerBoundary.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.normalized.installDryRunSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.normalized.doctorSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.launcherSha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.projectInventorySha256 | type == "string" and test("^[0-9a-f]{64}$"))
    and (.baselineThreadIds | type == "array")
    and (all(.baselineThreadIds[]; type == "string" and length > 0))
    and ((.baselineThreadIds | length) == (.baselineThreadIds | unique | length))
    and .engine == {gateMode:"active",autopilotMode:"conservative"}
    and .config.approvalPolicy == "on-request"
    and .config.approvalsReviewer == "user"
    and .config.sandboxMode == "workspace-write"
    and .config.workspaceWriteNetworkAccess == false
    and .config.bypassHookTrust == false
    and .config.bypassApprovalsAndSandbox == false
    and .config.feedbackApprovalMode == "prompt"
    and .config.applyApprovalMode == "prompt"
  ' "${state}/state.json" >/dev/null || die "ready FIA-5 state is incomplete"
}

validate_normalized_setup() {
  local state="$1" install doctor
  install="${state}/private/normalized/install-dry-run.json"
  doctor="${state}/private/normalized/doctor.json"
  require_regular_file "normalized installer dry-run" "${install}"
  require_regular_file "normalized doctor" "${doctor}"
  [[ "$(sha256_file "${install}")" == \
     "$(state_value "${state}" '.normalized.installDryRunSha256')" ]] ||
    die "normalized installer dry-run changed after begin"
  [[ "$(sha256_file "${doctor}")" == \
     "$(state_value "${state}" '.normalized.doctorSha256')" ]] ||
    die "normalized doctor changed after begin"
  jq -e \
    --arg commit "$(state_value "${state}" '.candidate.commit')" \
    --arg brainmapSha "$(state_value "${state}" '.candidate.brainmapSha256')" \
    --arg brainmapdSha "$(state_value "${state}" '.candidate.brainmapdSha256')" '
      keys == ["candidate", "dryRun", "schemaVersion", "target"]
      and .schemaVersion == "brainmap-m8-host-install-dry-run-v1"
      and .target == "codex"
      and .dryRun == true
      and .candidate == {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      }
    ' "${install}" >/dev/null || die "normalized installer dry-run is invalid"
  jq -e \
    --arg commit "$(state_value "${state}" '.candidate.commit')" \
    --arg brainmapSha "$(state_value "${state}" '.candidate.brainmapSha256')" \
    --arg brainmapdSha "$(state_value "${state}" '.candidate.brainmapdSha256')" '
      keys == [
        "candidate", "healthScope", "healthy", "hostHookTrustVerified",
        "hostProbeRequired", "schemaVersion", "target"
      ]
      and .schemaVersion == "brainmap-m8-host-doctor-v1"
      and .target == "codex"
      and .healthy == true
      and .healthScope == "local-adapter-files-and-contract"
      and .hostHookTrustVerified == false
      and .hostProbeRequired == true
      and .candidate == {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      }
    ' "${doctor}" >/dev/null || die "normalized doctor is invalid"
}

validate_launch_marker() {
  local state="$1" marker="$2" codex codex_home project
  codex="$(state_value "${state}" '.paths.codex')"
  codex_home="$(state_value "${state}" '.paths.codexHome')"
  project="$(state_value "${state}" '.paths.project')"
  require_regular_file "normal Codex launch marker" "${marker}"
  jq -e \
    --arg codex "${codex}" \
    --arg codexSha "$(state_value "${state}" '.host.codexSha256')" \
    --arg codexHome "${codex_home}" \
    --arg project "${project}" \
    --arg projectInventorySha "$(state_value "${state}" '.projectInventorySha256')" \
    --arg directive "${directive}" \
    --arg began "$(state_value "${state}" '.qualificationStartedAt')" '
      keys == [
        "argv", "codexHome", "codexSha256", "executable",
        "projectInventorySha256", "schemaVersion", "startedAt", "startedAtEpoch"
      ]
      and .schemaVersion == "brainmap-m8-codex-fia5-launch-v2"
      and .executable == $codex
      and .codexSha256 == $codexSha
      and .codexHome == $codexHome
      and .projectInventorySha256 == $projectInventorySha
      and .argv == [
        "--ask-for-approval", "on-request",
        "--sandbox", "workspace-write",
        "-c", "approvals_reviewer=\"user\"",
        "-c", "sandbox_workspace_write.network_access=false",
        "--cd", $project,
        "--no-alt-screen", $directive
      ]
      and (.argv | all(
        . != "--dangerously-bypass-hook-trust"
        and . != "--dangerously-bypass-approvals-and-sandbox"
        and . != "danger-full-access"
        and . != "never"
      ))
      and (.startedAt | type == "string" and . >= $began)
      and (.startedAtEpoch | type == "number" and . >= 0 and floor == .)
    ' "${marker}" >/dev/null ||
    die "launch marker does not prove the exact normal no-bypass Codex argv"
}

query_trusted_hooks() {
  local state="$1" codex project brainmap expected_prompt expected_tool params hooks
  codex="$(state_value "${state}" '.paths.codex')"
  project="$(state_value "${state}" '.paths.project')"
  brainmap="$(state_value "${state}" '.paths.brainmap')"
  expected_prompt="'${brainmap}' harness hook --host codex --event UserPromptSubmit"
  expected_tool="'${brainmap}' harness hook --host codex --event PreToolUse"
  params="$(jq -nc --arg project "${project}" '{cwds:[$project]}')"
  codex_rpc "${codex}" hooks/list "${params}" "$(state_value "${state}" '.paths.codexHome')"
  hooks="${RPC_RESULT}"
  HOOK_SUMMARY="$(jq -ce \
    --arg project "${project}" \
    --arg expectedPrompt "${expected_prompt}" \
    --arg expectedTool "${expected_tool}" '
      .data as $data
      | select(($data | length) == 1)
      | $data[0] as $entry
      | select($entry.cwd == $project)
      | select(($entry.errors | length) == 0 and ($entry.warnings | length) == 0)
      | select(($entry.hooks | length) == 2)
      | [$entry.hooks[] | select(
          .command == $expectedPrompt or .command == $expectedTool
        )] as $brainmapHooks
      | select(($brainmapHooks | length) == 2)
      | select(([
          $entry.hooks[]
          | select(((.command // "") | contains(" harness hook --host codex --event ")))
        ] | length) == 2)
      | select(all($brainmapHooks[];
          .enabled == true
          and .handlerType == "command"
          and .source == "project"
          and .isManaged == false
          and .timeoutSec == 10
          and .trustStatus == "trusted"
          and (.currentHash | test("^sha256:[0-9a-f]{64}$"))
        ))
      | select(any($brainmapHooks[];
          .eventName == "userPromptSubmit"
          and .command == $expectedPrompt
          and .matcher == null
        ))
      | select(any($brainmapHooks[];
          .eventName == "preToolUse"
          and .command == $expectedTool
          and .matcher == "Bash|Edit|Write|MultiEdit|NotebookEdit"
        ))
      | {
          trustedHookCount: 2,
          hooks: [
            $brainmapHooks[] | {eventName, currentHash, trustStatus}
          ] | sort_by(.eventName)
        }
    ' <<<"${hooks}")" ||
    die "Codex app-server did not prove both exact project hooks persisted as trusted"
  unset RPC_RESULT
}

query_normal_thread() {
  local state="$1" marker="$2" codex project host_version cli_version launch_epoch params threads
  codex="$(state_value "${state}" '.paths.codex')"
  project="$(state_value "${state}" '.paths.project')"
  host_version="$(state_value "${state}" '.host.version')"
  cli_version="${host_version#codex-cli }"
  launch_epoch="$(jq -er '.startedAtEpoch' "${marker}")"
  params="$(jq -nc --arg project "${project}" '{
    cwd:$project,
    sourceKinds:["cli"],
    sortKey:"created_at",
    sortDirection:"desc",
    limit:20,
    useStateDbOnly:false
  }')"
  codex_rpc "${codex}" thread/list "${params}" "$(state_value "${state}" '.paths.codexHome')"
  threads="${RPC_RESULT}"
  THREAD_SUMMARY="$(jq -ce \
    --arg project "${project}" \
    --arg cliVersion "${cli_version}" \
    --argjson launchEpoch "${launch_epoch}" \
    --argjson baselineThreadIds "$(jq -c '.baselineThreadIds' "${state}/state.json")" '
      select(.nextCursor == null)
      | [.data[] | select(
        .source == "cli"
        and .cwd == $project
        and .cliVersion == $cliVersion
        and .ephemeral == false
        and .parentThreadId == null
        and .createdAt >= $launchEpoch
        and .updatedAt >= .createdAt
        and ((.id as $id | $baselineThreadIds | index($id)) == null)
        and (.id | test("^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"))
      )] as $matches
      | select(($matches | length) == 1)
      | $matches[0]
      | {id, createdAt, updatedAt, source, cliVersion}
    ' <<<"${threads}")" ||
    die "expected exactly one post-launch normal Codex CLI thread for the synthetic project"
  unset RPC_RESULT
}

query_brainmap_calls() {
  local state="$1" thread_id="$2" expected_created_at="$3" codex project cli_version params thread
  codex="$(state_value "${state}" '.paths.codex')"
  project="$(state_value "${state}" '.paths.project')"
  cli_version="$(state_value "${state}" '.host.version')"
  cli_version="${cli_version#codex-cli }"
  params="$(jq -nc --arg threadId "${thread_id}" '{threadId:$threadId,includeTurns:true}')"
  codex_rpc "${codex}" thread/read "${params}" "$(state_value "${state}" '.paths.codexHome')"
  thread="${RPC_RESULT}"
  CALL_SUMMARY="$(jq -ce \
    --arg threadId "${thread_id}" \
    --arg project "${project}" \
    --arg cliVersion "${cli_version}" \
    --argjson expectedCreatedAt "${expected_created_at}" '
      def decoded_result:
        .result.content
        | select(
            type == "array"
            and length == 1
            and .[0].type == "text"
            and (.[0].text | type == "string")
          )
        | (.[0].text | try fromjson catch null)
        | select(. != null);
      select(.thread.id == $threadId)
      | select(
          .thread.source == "cli"
          and .thread.cwd == $project
          and .thread.cliVersion == $cliVersion
          and .thread.ephemeral == false
          and .thread.parentThreadId == null
          and .thread.createdAt == $expectedCreatedAt
          and .thread.updatedAt >= .thread.createdAt
          and all(.thread.turns[]; .status == "completed")
        )
      | [.thread.turns[].items[]?] as $items
      | select(($items | length) > 0)
      | select(all($items[];
          .type as $type
          | [
              "userMessage", "agentMessage", "reasoning", "plan",
              "hookPrompt", "mcpToolCall"
            ]
          | index($type) != null
        ))
      | select(all($items[] | select(.type == "mcpToolCall");
          .server == "brainmap"
        ))
      | [range(0; $items | length) | select(
          $items[.].type == "mcpToolCall" and $items[.].server == "brainmap"
        )] as $callPositions
      | [$callPositions[] | $items[.]] as $calls
      | [range(0; $items | length) | select($items[.].type == "userMessage")] as $userPositions
      | select(($calls | length) == 7)
      | select(($userPositions | length) >= 2)
      | select(any($userPositions[]; . < $callPositions[0]))
      | select(any($userPositions[]; . > $callPositions[0] and . < $callPositions[1]))
      | select(any($userPositions[];
          . > $callPositions[5] and . < $callPositions[6]
        ) | not)
      | select(all($calls[];
          .status == "completed"
          and .error == null
          and (.arguments | type == "object")
          and (.result | type == "object")
        ))
      | select([$calls[].tool] == [
          "brainmap_decision_gate",
          "brainmap_record_decision",
          "brainmap_learn_feedback",
          "brainmap_preview_update",
          "brainmap_apply_update",
          "brainmap_decision_gate",
          "brainmap_record_decision"
        ])
      | ($calls[0].arguments) as $firstArgs
      | ($calls[5].arguments) as $secondArgs
      | select($firstArgs == {
          intent: "would-ask-user",
          situation: "Choose formatter for synthetic FIA-5 project",
          options: ["biome", "prettier"],
          risk: "low",
          reversible: true,
          decisionType: "tooling",
          scope: "project:fia5",
          dryRun: false
        })
      | select($secondArgs == $firstArgs)
      | ($calls[0] | decoded_result) as $first
      | ($calls[1] | decoded_result) as $recorded
      | ($calls[2] | decoded_result) as $feedback
      | ($calls[3] | decoded_result) as $preview
      | ($calls[4] | decoded_result) as $applied
      | ($calls[5] | decoded_result) as $second
      | ($calls[6] | decoded_result) as $secondRecorded
      | select($first.decisionId | test("^dec_[0-9]{13}_[0-9a-f]{12}$"))
      | select($second.decisionId | test("^dec_[0-9]{13}_[0-9a-f]{12}$"))
      | select($second.decisionId != $first.decisionId)
      | select(
          $first.outcome == "ask_user"
          and $first.selectedOption == null
          and $first.predictedOutcome == "ask_user"
          and $first.predictedSelectedOption == null
          and $first.gateMode == "active"
          and $first.autopilotMode == "conservative"
        )
      | select($calls[1].arguments == {
          decisionId: $first.decisionId,
          chosen: "biome",
          wasAsked: true
        })
      | select($recorded.recorded == true)
      | select($calls[2].arguments == {
          decisionId: $first.decisionId,
          chosen: "prettier",
          rejected: "biome"
        })
      | select(
          $feedback.packetCreated == true
          and ($feedback.packetId | test("^upd_[0-9]{13}_[0-9a-f]{12}$"))
        )
      | select($calls[3].arguments == {packetId: $feedback.packetId})
      | select(
          ($preview | type) == "array"
          and ($preview | length) == 1
          and $preview[0].id == $feedback.packetId
        )
      | select($calls[4].arguments == {packetId: $feedback.packetId, approved: true})
      | select($applied == {applied: true, packetId: $feedback.packetId})
      | select(
          $second.outcome == "proceed"
          and $second.selectedOption == "prettier"
          and $second.predictedOutcome == "proceed"
          and $second.predictedSelectedOption == "prettier"
          and $second.gateMode == "active"
          and $second.autopilotMode == "conservative"
        )
      | select($calls[6].arguments == {
          decisionId: $second.decisionId,
          chosen: "prettier",
          wasAsked: false
        })
      | select($secondRecorded.recorded == true)
      | {
          decisionId: $first.decisionId,
          secondDecisionId: $second.decisionId,
          packetId: $feedback.packetId,
          first: {
            outcome: $first.outcome,
            selectedOption: $first.selectedOption,
            predictedOutcome: $first.predictedOutcome,
            predictedSelectedOption: $first.predictedSelectedOption,
            action: {chosen: "biome", wasAsked: true}
          },
          feedback: {previewed: true, approved: true},
          second: {
            outcome: $second.outcome,
            selectedOption: $second.selectedOption,
            predictedOutcome: $second.predictedOutcome,
            predictedSelectedOption: $second.predictedSelectedOption,
            action: {chosen: "prettier", wasAsked: false}
          },
          changed: (
            $first.outcome != $second.outcome
            or $first.selectedOption != $second.selectedOption
          ),
          callCount: 7,
          callOrder: [$calls[].tool],
          firstUserChoiceBoundaryObserved: true,
          secondNoUserChoiceBoundaryObserved: true
        }
      | select(.changed == true)
    ' <<<"${thread}")" ||
    die "Codex thread does not prove the exact completed Brainmap MCP lifecycle"
  unset RPC_RESULT
}

validate_ledger_correlation() {
  local state="$1" decision_id="$2" second_decision_id="$3" packet_id="$4"
  local vault ledger boundary_bytes boundary_sha current_bytes prefix_sha situation new_start
  vault="$(state_value "${state}" '.paths.vault')"
  ledger="${vault}/90-calibration/decision-ledger.jsonl"
  require_regular_file "Brainmap decision ledger" "${ledger}"
  boundary_bytes="$(state_value "${state}" '.ledgerBoundary.bytes')"
  boundary_sha="$(state_value "${state}" '.ledgerBoundary.sha256')"
  current_bytes="$(wc -c <"${ledger}" | tr -d ' ')"
  ((current_bytes >= boundary_bytes)) || die "Brainmap decision ledger was truncated"
  prefix_sha="$(head -c "${boundary_bytes}" "${ledger}" | sha256sum | cut -d ' ' -f 1)"
  [[ "${prefix_sha}" == "${boundary_sha}" ]] ||
    die "Brainmap decision ledger prefix changed after begin"
  [[ "${current_bytes}" -gt "${boundary_bytes}" ]] ||
    die "Brainmap decision ledger has no post-launch evidence"
  [[ "$(tail -c 1 "${ledger}")" == '' ]] || die "Brainmap decision ledger is not newline-complete"
  situation='Choose formatter for synthetic FIA-5 project'
  new_start=$((boundary_bytes + 1))
  LEDGER_SUMMARY="$(tail -c "+${new_start}" "${ledger}" | jq -sce \
    --arg decisionId "${decision_id}" \
    --arg secondDecisionId "${second_decision_id}" \
    --arg packetId "${packet_id}" \
    --arg situation "${situation}" '
      . as $events
      | [range(0; length) | select(
          $events[.].kind == "decision-gate"
          and $events[.].intent == "would-ask-user"
          and $events[.].situation == $situation
          and $events[.].options == ["biome", "prettier"]
          and $events[.].risk == "low"
          and $events[.].reversible == true
          and $events[.].decisionType == "tooling"
          and $events[.].scope == "project:fia5"
        )] as $gates
      | [range(0; length) | select($events[.].kind == "record-decision")] as $actions
      | [range(0; length) | select($events[.].kind == "learn-feedback")] as $feedback
      | [range(0; length) | select(
          $events[.].kind == "decision-gate"
          and $events[.].intent == "agent-hook:UserPromptSubmit"
          and $events[.].decisionType == "agent-harness"
        )] as $hookGates
      | select(($gates | length) == 2)
      | select(($actions | length) == 2)
      | select(($feedback | length) == 1)
      | select(($hookGates | length) >= 1)
      | select(
          (($gates + $actions + $feedback + $hookGates) | unique | length)
          == ($events | length)
        )
      | ($gates[0]) as $g1
      | ($gates[1]) as $g2
      | ($actions[0]) as $firstAction
      | ($actions[1]) as $secondAction
      | ($feedback[0]) as $fb
      | select(
          $g1 < $firstAction
          and $firstAction < $fb
          and $fb < $g2
          and $g2 < $secondAction
        )
      | select(
          $events[$g1].id == $decisionId
          and $events[$g1].outcome == "ask_user"
          and $events[$g1].selectedOption == null
          and $events[$g1].predictedOutcome == "ask_user"
          and $events[$g1].predictedSelectedOption == null
          and $events[$g1].gateMode == "active"
          and $events[$g1].autopilotMode == "conservative"
        )
      | select(
          $events[$firstAction].decisionId == $decisionId
          and $events[$firstAction].chosen == "biome"
          and $events[$firstAction].wasAsked == true
        )
      | select(
          $events[$fb].decisionId == $decisionId
          and $events[$fb].packetId == $packetId
          and $events[$fb].chosen == "prettier"
          and $events[$fb].rejected == ["biome"]
        )
      | select(
          $events[$g2].id == $secondDecisionId
          and $events[$g2].outcome == "proceed"
          and $events[$g2].selectedOption == "prettier"
          and $events[$g2].predictedOutcome == "proceed"
          and $events[$g2].predictedSelectedOption == "prettier"
          and $events[$g2].gateMode == "active"
          and $events[$g2].autopilotMode == "conservative"
        )
      | select(
          $events[$secondAction].decisionId == $secondDecisionId
          and $events[$secondAction].chosen == "prettier"
          and $events[$secondAction].wasAsked == false
        )
      | select(all($events[]; (.createdAt | type == "string" and length > 0)))
      | {
          hookGateCount: ($hookGates | length),
          correlatedEventCount: 5,
          totalPostBoundaryEvents: ($events | length)
        }
    ')" || die "Brainmap ledger does not correlate to the Codex MCP lifecycle"
  LEDGER_SHA256="$(sha256_file "${ledger}")"
}

write_events() {
  local destination="$1" decision_id="$2" second_decision_id="$3" packet_id="$4"
  {
    jq -nc '{sequence:1,kind:"installer-dry-run",success:true}'
    jq -nc '{sequence:2,kind:"installed",success:true}'
    jq -nc '{sequence:3,kind:"doctor-healthy",success:true}'
    jq -nc '{sequence:4,kind:"host-launched",success:true}'
    jq -nc --arg decisionId "${decision_id}" \
      '{sequence:5,kind:"initial-gate",success:true,decisionId:$decisionId}'
    jq -nc --arg decisionId "${decision_id}" \
      '{sequence:6,kind:"initial-outcome-followed",success:true,decisionId:$decisionId}'
    jq -nc --arg decisionId "${decision_id}" \
      '{sequence:7,kind:"initial-action-recorded",success:true,decisionId:$decisionId}'
    jq -nc --arg decisionId "${decision_id}" --arg packetId "${packet_id}" \
      '{sequence:8,kind:"feedback-created",success:true,decisionId:$decisionId,packetId:$packetId}'
    jq -nc --arg decisionId "${decision_id}" --arg packetId "${packet_id}" \
      '{sequence:9,kind:"preview-observed",success:true,decisionId:$decisionId,packetId:$packetId}'
    jq -nc --arg decisionId "${decision_id}" --arg packetId "${packet_id}" \
      '{sequence:10,kind:"update-approved",success:true,decisionId:$decisionId,packetId:$packetId}'
    jq -nc --arg decisionId "${second_decision_id}" --arg packetId "${packet_id}" \
      '{
        sequence:11,kind:"changed-outcome-followed",success:true,
        decisionId:$decisionId,packetId:$packetId,changed:true,
        outcome:"proceed",selectedOption:"prettier"
      }'
    jq -nc --arg decisionId "${second_decision_id}" --arg packetId "${packet_id}" \
      '{
        sequence:12,kind:"changed-action-recorded",success:true,
        decisionId:$decisionId,packetId:$packetId
      }'
  } >"${destination}"
}

validate_public_tree() {
  local directory="$1" invalid sensitive_file
  invalid="$(find "${directory}" -mindepth 1 -type l -print -quit)"
  [[ -z "${invalid}" ]] || die "evidence contains a symlink"
  invalid="$(find "${directory}" -mindepth 1 ! -type f ! -type d -print -quit)"
  [[ -z "${invalid}" ]] || die "evidence contains a non-regular entry"
  sensitive_file="$(grep -ERail \
    '(/home/|/users/|/tmp/|/opt/|/root/|/var/folders/|c:\\users\\)|"(prompt|messages|transcript|situation|options|toolarguments)"[[:space:]]*:' \
    "${directory}" | head -n 1 || true)"
  [[ -z "${sensitive_file}" ]] ||
    die "$(basename "${sensitive_file}") contains a private path, prompt, transcript, or raw decision field"
  sensitive_file="$(grep -ERail \
    '(-----BEGIN [A-Z ]*PRIVATE KEY-----|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,}|authorization:[[:space:]]*(bearer|basic))' \
    "${directory}" | head -n 1 || true)"
  [[ -z "${sensitive_file}" ]] ||
    die "$(basename "${sensitive_file}") contains secret-like material"
}

finalize_state() {
  local state='' out='' option
  while (($#)); do
    case "$1" in
      --state|--out)
        option="$1"
        require_value "${option}" "${2:-}"
        case "${option}" in
          --state) state="$2" ;;
          --out) out="$2" ;;
        esac
        shift 2
        ;;
      *) die "unknown finalize argument: $1" ;;
    esac
  done
  [[ -n "${state}" ]] || die "finalize requires --state DIR"
  [[ -n "${out}" ]] || die "finalize requires --out DIR"
  for command in basename cmp cp cut date find grep head jq mkdir mktemp mv rmdir sha256sum sort stat sync tail tr uname wc; do
    require_command "${command}"
  done
  validate_state_tree "${state}" ready
  validate_ready_state_shape "${state}"
  validate_state_candidate "${state}"
  validate_normalized_setup "${state}"
  [[ -z "${BRAINMAP_DISABLE_AUTOPILOT:-}" && -z "${BRAINMAP_GATE_MODE:-}" ]] ||
    die "qualification environment cannot override Brainmap gate or autopilot mode"

  local brainmap codex codex_home project vault mode qualification_eligible
  local marker launcher_sha marker_sha doctor_output engine_status
  local expected_project_inventory current_project_inventory
  brainmap="$(state_value "${state}" '.paths.brainmap')"
  codex="$(state_value "${state}" '.paths.codex')"
  codex_home="$(state_value "${state}" '.paths.codexHome')"
  project="$(state_value "${state}" '.paths.project')"
  vault="$(state_value "${state}" '.paths.vault')"
  mode="$(state_value "${state}" '.mode')"
  qualification_eligible="$(jq -cr '.qualificationEligible' "${state}/state.json")"
  expected_project_inventory="$(state_value "${state}" '.projectInventorySha256')"
  [[ "$(sha256_file "${project}/FIA5-WORKFLOW.md")" == \
     "$(state_value "${state}" '.workflowSha256')" ]] ||
    die "fixed synthetic workflow changed after the Codex session"
  current_project_inventory="$(project_inventory_sha256 "${project}")"
  [[ "${current_project_inventory}" == "${expected_project_inventory}" ]] ||
    die "synthetic project inventory changed during the Codex session"
  marker="${state}/private/launch-marker.json"
  launcher_sha="$(sha256_file "${state}/launch-codex.sh")"
  [[ "${launcher_sha}" == "$(state_value "${state}" '.launcherSha256')" ]] ||
    die "normal Codex launcher changed after begin"
  validate_launch_marker "${state}" "${marker}"
  marker_sha="$(sha256_file "${marker}")"

  doctor_output="$("${brainmap}" integration doctor --target codex --project "${project}" \
    --vault "${vault}")" || die "Brainmap integration doctor became unhealthy"
  validate_doctor_output "${doctor_output}" ||
    die "Brainmap integration doctor no longer proves the strict local adapter contract"

  engine_status="$("${brainmap}" autopilot status --vault "${vault}")" ||
    die "cannot read active Brainmap qualification mode"
  jq -e '
    .gateMode == "active"
    and .mode == "conservative"
    and .killSwitch == false
  ' <<<"${engine_status}" >/dev/null ||
    die "Brainmap FIA-5 no longer has active gate mode with conservative autopilot"

  query_safe_config "${state}"
  local config_at_start="${CONFIG_SUMMARY}"
  [[ "${config_at_start}" == "$(jq -c '.config' "${state}/state.json")" ]] ||
    die "Codex effective config changed after begin"

  query_trusted_hooks "${state}"
  local hooks_at_start="${HOOK_SUMMARY}"
  query_normal_thread "${state}" "${marker}"
  local thread_at_start="${THREAD_SUMMARY}"
  local thread_id thread_created_at
  thread_id="$(jq -er '.id' <<<"${THREAD_SUMMARY}")"
  thread_created_at="$(jq -er '.createdAt' <<<"${THREAD_SUMMARY}")"
  query_brainmap_calls "${state}" "${thread_id}" "${thread_created_at}"
  local calls_at_start="${CALL_SUMMARY}"
  local decision_id second_decision_id packet_id
  decision_id="$(jq -er '.decisionId' <<<"${CALL_SUMMARY}")"
  second_decision_id="$(jq -er '.secondDecisionId' <<<"${CALL_SUMMARY}")"
  packet_id="$(jq -er '.packetId' <<<"${CALL_SUMMARY}")"
  validate_ledger_correlation "${state}" "${decision_id}" "${second_decision_id}" "${packet_id}"
  local ledger_sha_at_start="${LEDGER_SHA256}"

  out="$(canonical_new_path "${out}" evidence)"
  paths_overlap "${out}" "${root}" && die "evidence output must be outside the repository"
  paths_overlap "${out}" "${state}" && die "evidence output must be outside state"
  paths_overlap "${out}" "${project}" && die "evidence output must be outside the synthetic project"
  paths_overlap "${out}" "${vault}" && die "evidence output must be outside the synthetic vault"
  paths_overlap "${out}" "${codex_home}" && die "evidence output must be outside CODEX_HOME"

  local out_parent publication_lock lock_acquired container bundle cleanup_done completed_at
  out_parent="$(dirname "${out}")"
  publication_lock="${out}.lock"
  lock_acquired=false
  container=''
  bundle=''
  cleanup_done=false
  cleanup_finalize() {
    if [[ "${cleanup_done}" != true && -n "${container}" ]]; then
      rm -rf "${container}"
    fi
    if [[ "${lock_acquired}" == true ]]; then
      rmdir "${publication_lock}" 2>/dev/null || true
      sync "${out_parent}" 2>/dev/null || true
    fi
  }
  trap cleanup_finalize EXIT HUP INT TERM
  mkdir "${publication_lock}" 2>/dev/null ||
    die "evidence publication lock already exists: ${publication_lock}"
  lock_acquired=true
  sync "${publication_lock}" "${out_parent}"
  [[ ! -e "${out}" && ! -L "${out}" ]] ||
    die "evidence path appeared while acquiring the publication lock"
  container="$(mktemp -d "${out_parent}/.brainmap-m8-host.XXXXXX")"
  bundle="${container}/bundle"
  mkdir -p "${bundle}"

  cp "${state}/private/normalized/install-dry-run.json" "${bundle}/install-dry-run.json"
  cp "${state}/private/normalized/doctor.json" "${bundle}/doctor.json"
  write_events \
    "${bundle}/events.jsonl" "${decision_id}" "${second_decision_id}" "${packet_id}"

  local project_path_sha codex_home_sha session_id_sha hook_gate_count codex_sha directive_sha
  local workflow_sha launch_argv_sha app_server_argv_sha official_verified
  local correlated_event_count post_boundary_event_count
  project_path_sha="$(sha256_text "${project}")"
  codex_home_sha="$(sha256_text "${codex_home}")"
  session_id_sha="$(sha256_text "${thread_id}")"
  hook_gate_count="$(jq -er '.hookGateCount' <<<"${LEDGER_SUMMARY}")"
  correlated_event_count="$(jq -er '.correlatedEventCount' <<<"${LEDGER_SUMMARY}")"
  post_boundary_event_count="$(jq -er '.totalPostBoundaryEvents' <<<"${LEDGER_SUMMARY}")"
  codex_sha="$(state_value "${state}" '.host.codexSha256')"
  directive_sha="$(state_value "${state}" '.directiveSha256')"
  workflow_sha="$(state_value "${state}" '.workflowSha256')"
  official_verified="$(jq -cr '.host.officialVerified' "${state}/state.json")"
  launch_argv_sha="$(sha256_argv \
    "${codex}" \
    --ask-for-approval on-request \
    --sandbox workspace-write \
    -c 'approvals_reviewer="user"' \
    -c 'sandbox_workspace_write.network_access=false' \
    --cd "${project}" \
    --no-alt-screen "${directive}")"
  app_server_argv_sha="$(sha256_argv \
    "${codex}" \
    -c 'approval_policy="on-request"' \
    -c 'approvals_reviewer="user"' \
    -c 'sandbox_mode="workspace-write"' \
    -c 'sandbox_workspace_write.network_access=false' \
    app-server --stdio)"
  jq -n \
    --arg mode "${mode}" \
    --argjson qualificationEligible "${qualification_eligible}" \
    --arg commit "$(state_value "${state}" '.candidate.commit')" \
    --arg brainmapSha "$(state_value "${state}" '.candidate.brainmapSha256')" \
    --arg brainmapdSha "$(state_value "${state}" '.candidate.brainmapdSha256')" \
    --arg officialVersion "${official_codex_version}" \
    --arg officialTarget "${official_codex_target}" \
    --arg officialArchiveSha "${official_codex_archive_sha256}" \
    --arg officialBinarySha "${official_codex_binary_sha256}" \
    --argjson officialVerified "${official_verified}" \
    --arg codexSha "${codex_sha}" \
    --arg codexHomeSha "${codex_home_sha}" \
    --arg launcherSha "${launcher_sha}" \
    --arg launchArgvSha "${launch_argv_sha}" \
    --arg appServerArgvSha "${app_server_argv_sha}" \
    --arg projectPathSha "${project_path_sha}" \
    --arg directiveSha "${directive_sha}" \
    --arg workflowSha "${workflow_sha}" \
    --arg projectInventorySha "${expected_project_inventory}" \
    --arg sessionIdSha "${session_id_sha}" \
    --argjson sessionCreatedAt "${thread_created_at}" \
    --argjson hookGateCount "${hook_gate_count}" \
    --argjson correlatedEventCount "${correlated_event_count}" \
    --argjson postBoundaryEventCount "${post_boundary_event_count}" \
    --argjson hooks "$(jq -c '.hooks' <<<"${HOOK_SUMMARY}")" \
    --argjson config "${config_at_start}" \
    --argjson calls "${CALL_SUMMARY}" '{
      schemaVersion: "brainmap-m8-host-observation-v2",
      qualificationEligible: $qualificationEligible,
      mode: $mode,
      candidate: {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      },
      officialCodex: {
        version: $officialVersion,
        target: $officialTarget,
        archiveSha256: $officialArchiveSha,
        binarySha256: $officialBinarySha,
        observedBinarySha256: $codexSha,
        archiveVerified: $officialVerified,
        binaryVerified: $officialVerified
      },
      config: ($config + {
        codexHomeSha256: $codexHomeSha,
        gateMode: "active",
        autopilotMode: "conservative"
      }),
      launch: {
        launcherSha256: $launcherSha,
        argvSha256: $launchArgvSha,
        argv: [
          {position:0,kind:"codex-executable",sha256:$codexSha},
          {position:1,literal:"--ask-for-approval"},
          {position:2,literal:"on-request"},
          {position:3,literal:"--sandbox"},
          {position:4,literal:"workspace-write"},
          {position:5,literal:"-c"},
          {position:6,literal:"approvals_reviewer=\"user\""},
          {position:7,literal:"-c"},
          {position:8,literal:"sandbox_workspace_write.network_access=false"},
          {position:9,literal:"--cd"},
          {position:10,kind:"synthetic-project",sha256:$projectPathSha},
          {position:11,literal:"--no-alt-screen"},
          {position:12,kind:"fixed-workflow-directive",sha256:$directiveSha}
        ],
        appServerArgvSha256: $appServerArgvSha,
        appServerArgv: [
          {position:0,kind:"codex-executable",sha256:$codexSha},
          {position:1,literal:"-c"},
          {position:2,literal:"approval_policy=\"on-request\""},
          {position:3,literal:"-c"},
          {position:4,literal:"approvals_reviewer=\"user\""},
          {position:5,literal:"-c"},
          {position:6,literal:"sandbox_mode=\"workspace-write\""},
          {position:7,literal:"-c"},
          {position:8,literal:"sandbox_workspace_write.network_access=false"},
          {position:9,literal:"app-server"},
          {position:10,literal:"--stdio"}
        ],
        codexHomeBound: true,
        projectInventoryBound: true,
        session: {
          source: "cli",
          idSha256: $sessionIdSha,
          createdAt: $sessionCreatedAt
        }
      },
      hooks: {
        trustedHookCount: 2,
        entries: $hooks,
        executedHookGateCount: $hookGateCount
      },
      calls: {
        count: $calls.callCount,
        order: $calls.callOrder,
        first: {
          decisionId: $calls.decisionId,
          outcome: $calls.first.outcome,
          selectedOption: $calls.first.selectedOption,
          action: $calls.first.action
        },
        feedback: {
          packetId: $calls.packetId,
          previewed: $calls.feedback.previewed,
          approved: $calls.feedback.approved
        },
        second: {
          decisionId: $calls.secondDecisionId,
          outcome: $calls.second.outcome,
          selectedOption: $calls.second.selectedOption,
          changed: $calls.changed,
          action: $calls.second.action
        }
      },
      ledger: {
        correlation: "complete",
        correlatedEventCount: $correlatedEventCount,
        postBoundaryEventCount: $postBoundaryEventCount
      },
      project: {
        inventorySha256: $projectInventorySha,
        workflowSha256: $workflowSha,
        unchanged: true
      }
    }' >"${bundle}/host-observation.json"

  local events_sha install_sha doctor_sha host_observation_sha
  events_sha="$(sha256_file "${bundle}/events.jsonl")"
  install_sha="$(sha256_file "${bundle}/install-dry-run.json")"
  doctor_sha="$(sha256_file "${bundle}/doctor.json")"
  host_observation_sha="$(sha256_file "${bundle}/host-observation.json")"
  completed_at="$(now_iso)"
  jq -n \
    --arg commit "$(state_value "${state}" '.candidate.commit')" \
    --arg brainmapSha "$(state_value "${state}" '.candidate.brainmapSha256')" \
    --arg brainmapdSha "$(state_value "${state}" '.candidate.brainmapdSha256')" \
    --arg startedAt "$(state_value "${state}" '.qualificationStartedAt')" \
    --arg completedAt "${completed_at}" \
    --arg mode "${mode}" \
    --argjson qualificationEligible "${qualification_eligible}" \
    --arg hostVersion "$(state_value "${state}" '.host.version')" \
    --arg hostTarget "$(state_value "${state}" '.host.target')" \
    --arg officialArchiveSha "$(state_value "${state}" '.host.officialArchiveSha256')" \
    --arg officialBinarySha "$(state_value "${state}" '.host.officialBinarySha256')" \
    --arg observedCodexSha "$(state_value "${state}" '.host.codexSha256')" \
    --argjson officialVerified "${official_verified}" \
    --arg kernelName "$(uname -s)" \
    --arg kernelRelease "$(uname -r)" \
    --arg architecture "$(uname -m)" \
    --arg eventsSha "${events_sha}" \
    --arg installSha "${install_sha}" \
    --arg doctorSha "${doctor_sha}" \
    --arg hostObservationSha "${host_observation_sha}" '{
      schemaVersion: "brainmap-m8-host-v2",
      qualificationEligible: $qualificationEligible,
      mode: $mode,
      candidate: {
        commit: $commit,
        brainmapSha256: $brainmapSha,
        brainmapdSha256: $brainmapdSha
      },
      startedAt: $startedAt,
      completedAt: $completedAt,
      adapter: {
        target: "codex",
        hostVersion: $hostVersion,
        launchMode: "normal",
        trustBypassUsed: false,
        persistedHookAccepted: true,
        projectTrusted: true
      },
      provenance: {
        kernelName: $kernelName,
        kernelRelease: $kernelRelease,
        architecture: $architecture,
        configuredBrainmapSha256: $brainmapSha,
        configuredBrainmapdSha256: $brainmapdSha,
        codexTarget: $hostTarget,
        officialCodexArchiveSha256: $officialArchiveSha,
        officialCodexBinarySha256: $officialBinarySha,
        observedCodexBinarySha256: $observedCodexSha,
        officialCodexVerified: $officialVerified
      },
      artifacts: {
        events: {path: "events.jsonl", sha256: $eventsSha},
        installDryRun: {path: "install-dry-run.json", sha256: $installSha},
        doctor: {path: "doctor.json", sha256: $doctorSha},
        hostObservation: {path: "host-observation.json", sha256: $hostObservationSha}
      },
      privacy: {
        rawPromptsRetained: false,
        secretsRetained: false,
        privatePathsRetained: false,
        syntheticInputsOnly: true
      }
    }' >"${bundle}/manifest.json"

  jq -e \
    --arg officialVersion "${official_codex_version}" \
    --arg officialTarget "${official_codex_target}" \
    --arg officialArchiveSha "${official_codex_archive_sha256}" \
    --arg officialBinarySha "${official_codex_binary_sha256}" \
    --arg projectInventorySha "${expected_project_inventory}" \
    --arg workflowSha "${workflow_sha}" '
      keys == [
        "calls", "candidate", "config", "hooks", "launch", "ledger", "mode",
        "officialCodex", "project", "qualificationEligible", "schemaVersion"
      ]
      and .schemaVersion == "brainmap-m8-host-observation-v2"
      and (.mode == "qualification" or .mode == "fixture")
      and (.qualificationEligible == (.mode == "qualification"))
      and .officialCodex.version == $officialVersion
      and .officialCodex.target == $officialTarget
      and .officialCodex.archiveSha256 == $officialArchiveSha
      and .officialCodex.binarySha256 == $officialBinarySha
      and (
        if .qualificationEligible then
          .officialCodex.archiveVerified == true
          and .officialCodex.binaryVerified == true
          and .officialCodex.observedBinarySha256 == $officialBinarySha
        else
          .officialCodex.archiveVerified == false
          and .officialCodex.binaryVerified == false
        end
      )
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
      and (.launch.appServerArgvSha256 | test("^[0-9a-f]{64}$"))
      and [.launch.argv[] | (.literal // .kind)] == [
        "codex-executable", "--ask-for-approval", "on-request", "--sandbox",
        "workspace-write", "-c", "approvals_reviewer=\"user\"", "-c",
        "sandbox_workspace_write.network_access=false", "--cd",
        "synthetic-project", "--no-alt-screen", "fixed-workflow-directive"
      ]
      and [.launch.appServerArgv[] | (.literal // .kind)] == [
        "codex-executable", "-c", "approval_policy=\"on-request\"", "-c",
        "approvals_reviewer=\"user\"", "-c",
        "sandbox_mode=\"workspace-write\"", "-c",
        "sandbox_workspace_write.network_access=false", "app-server", "--stdio"
      ]
      and .launch.session.source == "cli"
      and (.launch.session.idSha256 | test("^[0-9a-f]{64}$"))
      and (.launch.session.createdAt | type == "number")
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
      and .project.inventorySha256 == $projectInventorySha
      and .project.workflowSha256 == $workflowSha
      and .project.unchanged == true
    ' "${bundle}/host-observation.json" >/dev/null ||
    die "generated host observation is invalid"

  write_checksums "${bundle}" "${bundle}/SHA256SUMS"
  validate_checksum_tree "${bundle}"
  validate_public_tree "${bundle}"
  jq -e '
    .schemaVersion == "brainmap-m8-host-v2"
    and (.mode == "qualification" or .mode == "fixture")
    and (.qualificationEligible == (.mode == "qualification"))
    and .adapter == {
      target: "codex",
      hostVersion: .adapter.hostVersion,
      launchMode: "normal",
      trustBypassUsed: false,
      persistedHookAccepted: true,
      projectTrusted: true
    }
    and .privacy == {
      rawPromptsRetained: false,
      secretsRetained: false,
      privatePathsRetained: false,
      syntheticInputsOnly: true
    }
    and .artifacts.hostObservation.path == "host-observation.json"
    and (.artifacts.hostObservation.sha256 | test("^[0-9a-f]{64}$"))
  ' "${bundle}/manifest.json" >/dev/null || die "generated host manifest is invalid"

  validate_state_candidate "${state}"
  [[ "$(sha256_file "${marker}")" == "${marker_sha}" ]] ||
    die "normal launch marker changed during finalization"
  [[ "$(sha256_file "${vault}/90-calibration/decision-ledger.jsonl")" == \
     "${ledger_sha_at_start}" ]] || die "Brainmap ledger changed during finalization"
  query_trusted_hooks "${state}"
  [[ "${HOOK_SUMMARY}" == "${hooks_at_start}" ]] ||
    die "Codex hook trust changed during finalization"

  query_safe_config "${state}"
  [[ "${CONFIG_SUMMARY}" == "${config_at_start}" ]] ||
    die "Codex effective config changed during finalization"
  engine_status="$("${brainmap}" autopilot status --vault "${vault}")" ||
    die "cannot re-read active Brainmap qualification mode"
  jq -e '
    .gateMode == "active"
    and .mode == "conservative"
    and .killSwitch == false
  ' <<<"${engine_status}" >/dev/null ||
    die "Brainmap qualification mode changed during finalization"
  [[ "$(sha256_file "${project}/FIA5-WORKFLOW.md")" == "${workflow_sha}" ]] ||
    die "fixed synthetic workflow changed during finalization"
  current_project_inventory="$(project_inventory_sha256 "${project}")"
  [[ "${current_project_inventory}" == "${expected_project_inventory}" ]] ||
    die "synthetic project inventory changed during finalization"
  query_normal_thread "${state}" "${marker}"
  [[ "${THREAD_SUMMARY}" == "${thread_at_start}" ]] ||
    die "Codex thread set changed during finalization"
  query_brainmap_calls "${state}" "${thread_id}" "${thread_created_at}"
  [[ "${CALL_SUMMARY}" == "${calls_at_start}" ]] ||
    die "Codex thread lifecycle changed during finalization"

  sync_tree "${bundle}"
  sync "${bundle}" "${container}" "${out_parent}"
  mv -n -T "${bundle}" "${out}"
  [[ ! -e "${bundle}" && -d "${out}" && ! -L "${out}" ]] ||
    die "atomic no-replace evidence publication lost a race"
  sync "${out}" "${out_parent}"
  rmdir "${publication_lock}" || die "cannot release evidence publication lock"
  lock_acquired=false
  sync "${out_parent}"
  cleanup_done=true
  trap - EXIT HUP INT TERM
  rm -rf "${container}"
  sync "${out_parent}"
  printf 'Published strict real-Codex FIA-5 evidence: %s\n' "${out}"
}

if (($# == 0)); then
  usage
  exit 1
fi

case "$1" in
  --help|-h)
    usage
    ;;
  prepare)
    shift
    prepare_state "$@"
    ;;
  begin)
    shift
    begin_state "$@"
    ;;
  finalize)
    shift
    finalize_state "$@"
    ;;
  *)
    die "unknown command: $1"
    ;;
esac
