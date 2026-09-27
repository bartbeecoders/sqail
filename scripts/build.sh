#!/usr/bin/env bash
# Build everything. Pass --release for an optimised build.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
cargo build --workspace "$@"
ok "build finished"
