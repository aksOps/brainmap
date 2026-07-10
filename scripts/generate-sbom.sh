#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${root}/crates/brainmap-cli/Cargo.toml"
sbom="${root}/crates/brainmap-cli/brainmap.json"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${manifest}" | head -1)"
stable_ref="pkg:cargo/brainmap-cli@${version}"

cd "${root}"
cargo cyclonedx --format json --override-filename brainmap

temporary="$(mktemp "${sbom}.XXXXXX")"
trap 'rm -f "${temporary}"' EXIT
jq --arg stable_ref "${stable_ref}" '
  del(.serialNumber, .metadata.timestamp)
  | walk(
      if type == "string" and startswith("path+file://") then
        sub("^path\\+file://[^#]+#[^ ]+"; $stable_ref)
      else
        .
      end
    )
' "${sbom}" > "${temporary}"
mv "${temporary}" "${sbom}"
trap - EXIT

if rg -n 'path\+file://|/(home|opt)/[^" ]+' "${sbom}"; then
  echo "generated SBOM contains a local filesystem path" >&2
  exit 1
fi
