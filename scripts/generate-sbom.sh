#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="${root}/crates/brainmap-cli/Cargo.toml"
sbom="${root}/crates/brainmap-cli/brainmap.json"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "${manifest}" | head -1)"
stable_ref="pkg:cargo/brainmap-cli@${version}"
vendor_rel="vendor/i18n-embed-fl"
vendor_version="0.9.4"
vendor_fix_commit="https://github.com/kellpossible/cargo-i18n/commit/f02d3ca8acb0c197290f13934aa9541f1e12b097"

unexpected_vendor_files=()
while IFS= read -r -d '' path; do
  unexpected_vendor_files+=("${path}")
done < <(git -C "${root}" ls-files --others --exclude-standard -z -- "${vendor_rel}")
if ((${#unexpected_vendor_files[@]} > 0)); then
  echo "vendored source contains unexpected non-ignored files:" >&2
  printf '  %s\n' "${unexpected_vendor_files[@]}" >&2
  exit 1
fi

vendor_files=()
while IFS= read -r -d '' path; do
  vendor_files+=("${path}")
done < <(git -C "${root}" ls-files --cached -z -- "${vendor_rel}" | sort -z)
if ((${#vendor_files[@]} == 0)); then
  echo "vendored source manifest is empty" >&2
  exit 1
fi

vendor_hash="$({
  for path in "${vendor_files[@]}"; do
    if [[ ! -f "${root}/${path}" ]]; then
      echo "vendored source manifest entry is not a file: ${path}" >&2
      exit 1
    fi
    relative_path="${path#"${vendor_rel}"/}"
    digest="$(sha256sum "${root}/${path}" | cut -d ' ' -f 1)"
    printf '%s\0%s\n' "${relative_path}" "${digest}"
  done
} | sha256sum | cut -d ' ' -f 1)"
vendor_ref="urn:brainmap:vendored:i18n-embed-fl:${vendor_version}:${vendor_hash}"

cd "${root}"
cargo cyclonedx --format json --override-filename brainmap

raw_root_ref="$(jq -er '
  .metadata.component["bom-ref"]
  | select(startswith("path+file://"))
' "${sbom}")"
raw_vendor_ref="$(jq -er --arg vendor_version "${vendor_version}" '
  [.components[]
    | select(.name == "i18n-embed-fl" and .version == $vendor_version)
    | .["bom-ref"]]
  | if length == 1 then .[0] else error("expected one vendored component") end
  | select(startswith("path+file://"))
' "${sbom}")"

if [[ "${raw_root_ref}" == "${raw_vendor_ref}" ]]; then
  echo "generated SBOM aliases root and vendored component references" >&2
  exit 1
fi

jq -e \
  --arg raw_root_ref "${raw_root_ref}" \
  --arg raw_vendor_ref "${raw_vendor_ref}" '
  ([.. | strings | select(startswith("path+file://"))
    | if startswith($raw_root_ref) then
        "root"
      elif startswith($raw_vendor_ref) then
        "vendor"
      else
        "unknown"
      end]
    | unique | sort) == ["root", "vendor"]
' "${sbom}" >/dev/null

temporary="$(mktemp "${sbom}.XXXXXX")"
trap 'rm -f "${temporary}"' EXIT
jq \
  --arg stable_ref "${stable_ref}" \
  --arg vendor_fix_commit "${vendor_fix_commit}" \
  --arg vendor_hash "${vendor_hash}" \
  --arg raw_root_ref "${raw_root_ref}" \
  --arg raw_vendor_ref "${raw_vendor_ref}" \
  --arg vendor_ref "${vendor_ref}" \
  --arg vendor_version "${vendor_version}" '
  def replace_prefix($from; $to):
    if startswith($from) then
      $to + .[($from | length):]
    else
      .
    end;

  del(.serialNumber, .metadata.timestamp)
  | .components |= map(
      if .name == "i18n-embed-fl" and .version == $vendor_version then
        .["bom-ref"] = $vendor_ref
        | .purl = "pkg:cargo/i18n-embed-fl@0.9.4"
        | .hashes = [{"alg": "SHA-256", "content": $vendor_hash}]
        | .properties = [
            {"name": "brainmap:hash-kind", "value": "canonical-source-tree"},
            {"name": "brainmap:upstream-fix", "value": $vendor_fix_commit},
            {"name": "brainmap:vendor-path", "value": "vendor/i18n-embed-fl"}
          ]
        | .externalReferences = (
            (.externalReferences // [])
            + [{"type": "vcs", "url": $vendor_fix_commit}]
            | unique_by([.type, .url])
          )
      else
        .
      end
    )
  | walk(
      if type == "string" then
        replace_prefix($raw_vendor_ref; $vendor_ref)
        | replace_prefix($raw_root_ref; $stable_ref)
      else
        .
      end
    )
' "${sbom}" > "${temporary}"
mv "${temporary}" "${sbom}"
trap - EXIT

jq -e \
  --arg stable_ref "${stable_ref}" \
  --arg vendor_fix_commit "${vendor_fix_commit}" \
  --arg vendor_hash "${vendor_hash}" \
  --arg vendor_ref "${vendor_ref}" '
  ([.components[] | select(.["bom-ref"] == $vendor_ref)] | length) == 1
  and ([.dependencies[] | select(.ref == $stable_ref)] | length) == 1
  and ([.dependencies[] | select(.ref == $vendor_ref)] | length) == 1
  and ([.dependencies[].dependsOn[]? | select(. == $vendor_ref)] | length) == 1
  and ([.components[]
    | select(.["bom-ref"] == $vendor_ref)
    | .hashes[]
    | select(.alg == "SHA-256" and .content == $vendor_hash)] | length) == 1
  and ([.components[]
    | select(.["bom-ref"] == $vendor_ref)
    | .properties[]
    | select(.name == "brainmap:upstream-fix" and .value == $vendor_fix_commit)]
    | length) == 1
' "${sbom}" >/dev/null

if rg -n 'path\+file://|file://[^.#]|/(home|opt)/[^" ]+' "${sbom}"; then
  echo "generated SBOM contains a local filesystem path" >&2
  exit 1
fi
