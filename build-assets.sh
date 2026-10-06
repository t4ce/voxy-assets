#!/usr/bin/env bash
set -euo pipefail

asset_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd -- "$asset_root"
exec cargo +"${VOXY_ASSET_TOOLCHAIN:-nightly-2026-07-10}" run \
    --locked --release --manifest-path "$asset_root/asset-builder/Cargo.toml" \
    --bin voxy-assets -- "$@"
