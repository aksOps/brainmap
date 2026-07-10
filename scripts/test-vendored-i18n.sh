#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
vendor_root="${root}/vendor/i18n-embed-fl"
generated_lock="${vendor_root}/Cargo.lock"
target_dir="${root}/target/vendor-i18n-embed-fl"

if [[ -e "${generated_lock}" ]]; then
  echo "unexpected vendored test lockfile already exists: ${generated_lock}" >&2
  exit 1
fi

cleanup() {
  rm -f "${generated_lock}"
}
trap cleanup EXIT

CARGO_TARGET_DIR="${target_dir}" \
  cargo test --manifest-path "${vendor_root}/Cargo.toml" --lib

cleanup
trap - EXIT

if [[ -e "${vendor_root}/target" ]]; then
  echo "vendored tests left a target directory in the source tree" >&2
  exit 1
fi
