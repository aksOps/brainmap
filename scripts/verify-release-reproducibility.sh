#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

usage() {
  cat <<'EOF'
Build byte-identical locked release binaries in two isolated source roots.

Usage:
  scripts/verify-release-reproducibility.sh [--manifest-out PATH]

Options:
  --manifest-out PATH  Write a new strict release-provenance manifest. PATH
                       must be absolute. Manifest emission requires clean HEAD.
  --help               Show this help.
EOF
}

manifest_out=
while (($#)); do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --manifest-out)
      [[ -n "${2:-}" ]] || {
        echo "--manifest-out requires a value" >&2
        exit 1
      }
      manifest_out="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -n "${manifest_out}" ]]; then
  if [[ "${manifest_out}" != /* ]]; then
    echo "manifest output path must be absolute" >&2
    exit 1
  fi
  if [[ -e "${manifest_out}" ]]; then
    echo "manifest output already exists: ${manifest_out}" >&2
    exit 1
  fi
fi

for command in cargo cmp cut dirname env git install jq mkdir mktemp mv sha256sum tar tr wc; do
  if ! command -v "${command}" >/dev/null; then
    echo "release reproducibility check requires ${command}" >&2
    exit 1
  fi
done

if ! git -C "${root}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "release reproducibility check requires a Git worktree" >&2
  exit 1
fi

candidate_commit="$(git -C "${root}" rev-parse HEAD)"
[[ "${candidate_commit}" =~ ^[0-9a-f]{40}$ ]] || {
  echo "release reproducibility check requires a full lowercase commit" >&2
  exit 1
}
if [[ -n "${manifest_out}" &&
      -n "$(git -C "${root}" status --porcelain --untracked-files=all)" ]]; then
  echo "release provenance manifest requires clean HEAD" >&2
  exit 1
fi

qualification_marker='brainmap-clean-locked-two-root-v1'
qualifying_build=false
if [[ -n "${manifest_out}" ]]; then
  qualifying_build=true
fi

integrated_qualification_sha256="$(
  sha256sum "${root}/scripts/m8-integrated-qualification.sh" | cut -d ' ' -f 1
)"
codex_fia5_sha256="$(
  sha256sum "${root}/scripts/m8-codex-fia5.sh" | cut -d ' ' -f 1
)"
release_qualification_sha256="$(
  sha256sum "${root}/scripts/m8-release-qualification.sh" | cut -d ' ' -f 1
)"
assemble_qualification_sha256="$(
  sha256sum "${root}/scripts/m8-assemble-qualification.sh" | cut -d ' ' -f 1
)"
producer_digests="$(jq -cn \
  --arg integrated "${integrated_qualification_sha256}" \
  --arg fia5 "${codex_fia5_sha256}" \
  --arg release "${release_qualification_sha256}" \
  --arg assemble "${assemble_qualification_sha256}" '{
    integratedQualificationSha256: $integrated,
    codexFia5Sha256: $fia5,
    releaseQualificationSha256: $release,
    assembleQualificationSha256: $assemble
  }')"

if [[ -n "${BRAINMAP_REPRO_WORKDIR:-}" ]]; then
  work_parent="${BRAINMAP_REPRO_WORKDIR%/}"
  mkdir -p "${work_parent}"
  work_parent="$(cd "${work_parent}" && pwd -P)"
  if [[ "${work_parent}" == "${root}" || "${work_parent}" == "${root}/"* ]]; then
    echo "BRAINMAP_REPRO_WORKDIR must be outside the repository" >&2
    exit 1
  fi
  work="$(mktemp -d "${work_parent}/brainmap-repro.XXXXXX")"
  echo "retaining reproducibility work directory: ${work}"
else
  work="$(mktemp -d)"
  trap 'rm -rf "${work}"' EXIT
fi
work="$(cd "${work}" && pwd -P)"
if [[ "${work}" == "${root}" || "${work}" == "${root}/"* ]]; then
  echo "reproducibility work directory must be outside the repository" >&2
  exit 1
fi

source_archive="${work}/source.tar"
source_a="${work}/source-a"
source_b="${work}/source-b"
target_a="${work}/target-a"
target_b="${work}/target-b"

mkdir -p "${source_a}" "${source_b}"

# Snapshot tracked files plus non-ignored worktree additions. This deliberately
# includes uncommitted changes so the check can run before a release commit.
(
  cd "${root}"
  git ls-files --cached --others --exclude-standard -z |
    while IFS= read -r -d '' path; do
      if [[ -f "${path}" || -L "${path}" ]]; then
        printf '%s\0' "${path}"
      fi
    done |
    tar --create --file "${source_archive}" --null --files-from=-
)

tar --extract --file "${source_archive}" --directory "${source_a}"
tar --extract --file "${source_archive}" --directory "${source_b}"

build_release() {
  local source_root="$1"
  local target_root="$2"
  (
    cd "${source_root}"
    if [[ "${qualifying_build}" == true ]]; then
      env \
        RUSTC_WRAPPER= \
        RUSTC_WORKSPACE_WRAPPER= \
        CARGO_BUILD_RUSTC_WRAPPER= \
        CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER= \
        CARGO_TARGET_DIR="${target_root}" \
        BRAINMAP_INTERNAL_QUALIFICATION_MARKER="${qualification_marker}" \
        BRAINMAP_INTERNAL_CANDIDATE_COMMIT="${candidate_commit}" \
        BRAINMAP_INTERNAL_SOURCE_CLEAN=true \
        BRAINMAP_INTERNAL_LOCKED=true \
        BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE=true \
        cargo build --release --locked -p brainmap-cli --bin brainmap --bin brainmapd
    else
      env \
        RUSTC_WRAPPER= \
        RUSTC_WORKSPACE_WRAPPER= \
        CARGO_BUILD_RUSTC_WRAPPER= \
        CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER= \
        CARGO_TARGET_DIR="${target_root}" \
        BRAINMAP_INTERNAL_QUALIFICATION_MARKER= \
        BRAINMAP_INTERNAL_CANDIDATE_COMMIT= \
        BRAINMAP_INTERNAL_SOURCE_CLEAN= \
        BRAINMAP_INTERNAL_LOCKED= \
        BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE= \
        cargo build --release --locked -p brainmap-cli --bin brainmap --bin brainmapd
    fi
  )
}

build_release "${source_a}" "${target_a}"
build_release "${source_b}" "${target_b}"

for binary in brainmap brainmapd; do
  first="${target_a}/release/${binary}"
  second="${target_b}/release/${binary}"
  if ! cmp --silent "${first}" "${second}"; then
    echo "non-reproducible release binary: ${binary}" >&2
    sha256sum "${first}" "${second}" >&2
    exit 1
  fi
done

build_info_a_brainmap="${work}/build-info-a-brainmap.json"
build_info_a_brainmapd="${work}/build-info-a-brainmapd.json"
build_info_b_brainmap="${work}/build-info-b-brainmap.json"
build_info_b_brainmapd="${work}/build-info-b-brainmapd.json"
"${target_a}/release/brainmap" build-info >"${build_info_a_brainmap}"
"${target_a}/release/brainmapd" build-info >"${build_info_a_brainmapd}"
"${target_b}/release/brainmap" build-info >"${build_info_b_brainmap}"
"${target_b}/release/brainmapd" build-info >"${build_info_b_brainmapd}"

for build_info_file in \
  "${build_info_a_brainmap}" \
  "${build_info_a_brainmapd}" \
  "${build_info_b_brainmap}" \
  "${build_info_b_brainmapd}"; do
  [[ "$(wc -l <"${build_info_file}" | tr -d ' ')" -eq 1 ]] || {
    echo "release binary build-info must be exactly one JSON line" >&2
    exit 1
  }
  jq -e \
    --arg candidateCommit "${candidate_commit}" \
    --arg qualificationMarker "${qualification_marker}" \
    --argjson qualifying "${qualifying_build}" \
    --argjson producerDigests "${producer_digests}" '
      type == "object"
      and (keys == [
        "candidateCommit", "cargoProfile", "producerDigests", "qualification",
        "schemaVersion"
      ])
      and .schemaVersion == "brainmap-build-info-v1"
      and .cargoProfile == "release"
      and (.candidateCommit | test("^[0-9a-f]{40}$"))
      and .producerDigests == $producerDigests
      and (.qualification | keys == [
        "eligible", "locked", "marker", "release", "twoRootCandidate"
      ])
      and if $qualifying then
        .candidateCommit == $candidateCommit
        and .qualification == {
          eligible: true,
          marker: $qualificationMarker,
          release: true,
          locked: true,
          twoRootCandidate: true
        }
      else
        .qualification == {
          eligible: false,
          marker: "nonqualifying",
          release: false,
          locked: false,
          twoRootCandidate: false
        }
      end
    ' "${build_info_file}" >/dev/null || {
      echo "release binary exposed invalid build-info: ${build_info_file}" >&2
      exit 1
    }
done

for reproduced_info in \
  "${build_info_a_brainmapd}" \
  "${build_info_b_brainmap}" \
  "${build_info_b_brainmapd}"; do
  cmp --silent "${build_info_a_brainmap}" "${reproduced_info}" || {
    echo "build-info mismatch across reproduced binaries" >&2
    exit 1
  }
done
build_info_json="$(<"${build_info_a_brainmap}")"
build_info_sha256="$(printf '%s' "${build_info_json}" | sha256sum | cut -d ' ' -f 1)"

published_dir="${root}/target/release"
mkdir -p "${published_dir}"
brainmap_sha256=
brainmapd_sha256=
for binary in brainmap brainmapd; do
  verified="${target_a}/release/${binary}"
  published="${published_dir}/${binary}"
  temporary="$(mktemp "${published_dir}/.${binary}.XXXXXX")"
  install -m 0755 "${verified}" "${temporary}"
  mv -f "${temporary}" "${published}"
  if ! cmp --silent "${verified}" "${published}"; then
    echo "failed to install verified release binary: ${binary}" >&2
    exit 1
  fi
  binary_sha256="$(sha256sum "${published}" | cut -d ' ' -f 1)"
  printf '%s  %s\n' "${binary_sha256}" "${published}"
  if [[ "${binary}" == brainmap ]]; then
    brainmap_sha256="${binary_sha256}"
  else
    brainmapd_sha256="${binary_sha256}"
  fi
done

echo "release binaries are byte-identical across two isolated source roots"
echo "installed verified release binaries in ${published_dir}"

if [[ -n "${manifest_out}" ]]; then
  if [[ "$(git -C "${root}" rev-parse HEAD)" != "${candidate_commit}" ||
        -n "$(git -C "${root}" status --porcelain --untracked-files=all)" ]]; then
    echo "release provenance manifest requires unchanged clean HEAD through build completion" >&2
    exit 1
  fi
  manifest_parent="$(dirname "${manifest_out}")"
  mkdir -p "${manifest_parent}"
  manifest_temporary="$(mktemp "${manifest_parent}/.brainmap-release-reproducibility.XXXXXX")"
  jq -n \
    --arg schemaVersion brainmap-release-reproducibility-v2 \
    --arg candidateCommit "${candidate_commit}" \
    --arg brainmapSha256 "${brainmap_sha256}" \
    --arg brainmapdSha256 "${brainmapd_sha256}" \
    --arg buildInfoSha256 "${build_info_sha256}" \
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
    }' >"${manifest_temporary}"
  chmod 0644 "${manifest_temporary}"
  mv -T -n "${manifest_temporary}" "${manifest_out}"
  [[ ! -e "${manifest_temporary}" ]] || {
    echo "release provenance manifest output appeared during publication" >&2
    exit 1
  }
  printf 'wrote strict release provenance manifest: %s\n' "${manifest_out}"
fi
