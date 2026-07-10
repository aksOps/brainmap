#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
vendor_root="${root}/vendor/i18n-embed-fl"
vendor_lock="${vendor_root}/Cargo.lock"
target_dir="${root}/target/vendor-i18n-embed-fl"

if [[ ! -f "${vendor_lock}" ]]; then
  echo "committed vendored lockfile is missing: ${vendor_lock}" >&2
  exit 1
fi

lock_hash_before="$(sha256sum "${vendor_lock}" | cut -d ' ' -f 1)"

CARGO_TARGET_DIR="${target_dir}" \
  cargo test --locked --manifest-path "${vendor_root}/Cargo.toml" --lib

lock_hash_after="$(sha256sum "${vendor_lock}" | cut -d ' ' -f 1)"
if [[ "${lock_hash_before}" != "${lock_hash_after}" ]]; then
  echo "vendored tests modified the committed lockfile" >&2
  exit 1
fi

if [[ -e "${vendor_root}/target" ]]; then
  echo "vendored tests left a target directory in the source tree" >&2
  exit 1
fi
