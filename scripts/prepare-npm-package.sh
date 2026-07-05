#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pkg="${root}/npm/brainmap"

mkdir -p "${pkg}/bin"
cp "${root}/target/release/brainmap" "${pkg}/bin/brainmap"
cp "${root}/target/release/brainmapd" "${pkg}/bin/brainmapd"
chmod 755 "${pkg}/bin/brainmap" "${pkg}/bin/brainmapd"
