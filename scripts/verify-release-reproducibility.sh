#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

for command in cargo cmp env git install sha256sum tar; do
  if ! command -v "${command}" >/dev/null; then
    echo "release reproducibility check requires ${command}" >&2
    exit 1
  fi
done

if ! git -C "${root}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "release reproducibility check requires a Git worktree" >&2
  exit 1
fi

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
    env \
      RUSTC_WRAPPER= \
      RUSTC_WORKSPACE_WRAPPER= \
      CARGO_BUILD_RUSTC_WRAPPER= \
      CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER= \
      CARGO_TARGET_DIR="${target_root}" \
      cargo build --release --locked -p brainmap-cli --bin brainmap --bin brainmapd
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

published_dir="${root}/target/release"
mkdir -p "${published_dir}"
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
  sha256sum "${published}"
done

echo "release binaries are byte-identical across two isolated source roots"
echo "installed verified release binaries in ${published_dir}"
