#!/usr/bin/env bash
# Run the sqail desktop editor.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
exec cargo run -p sqail-ui --bin sqail -- "$@"
