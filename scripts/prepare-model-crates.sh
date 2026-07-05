#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
asset="${1:-${root}/assets/models/default.brainmap-model.tar.zst}"
model_rs="${root}/crates/brainmap-cli/src/model.rs"
parts=4
max_chunk_size=$((9 * 1024 * 1024))

expected_sha="$(sed -n 's/^const PACK_SHA256: &str = "\(.*\)";$/\1/p' "${model_rs}")"
expected_len="$(sed -n 's/^const PACK_LEN: usize = \([0-9_]*\);$/\1/p' "${model_rs}" | tr -d '_')"

actual_sha="$(sha256sum "${asset}" | awk '{print $1}')"
actual_len="$(wc -c < "${asset}" | tr -d ' ')"
if [ "${actual_sha}" != "${expected_sha}" ]; then
  echo "model pack sha mismatch: got ${actual_sha}, expected ${expected_sha}" >&2
  exit 1
fi
if [ "${actual_len}" != "${expected_len}" ]; then
  echo "model pack length mismatch: got ${actual_len}, expected ${expected_len}" >&2
  exit 1
fi

chunk_size=$(((actual_len + parts - 1) / parts))
if [ "${chunk_size}" -gt "${max_chunk_size}" ]; then
  echo "model chunks would exceed ${max_chunk_size} bytes; add more chunk crates" >&2
  exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
split -b "${chunk_size}" -d -a 1 --additional-suffix=.bin "${asset}" "${tmp}/part-"

actual_parts="$(find "${tmp}" -name 'part-*.bin' | wc -l | tr -d ' ')"
if [ "${actual_parts}" != "${parts}" ]; then
  echo "expected ${parts} chunks, got ${actual_parts}" >&2
  exit 1
fi

for idx in 0 1 2 3; do
  part=$((idx + 1))
  cp "${tmp}/part-${idx}.bin" "${root}/crates/brainmap-model-potion-base-8m-part-${part}/data/part.bin"
done
