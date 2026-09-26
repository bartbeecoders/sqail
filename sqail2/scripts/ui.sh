#!/usr/bin/env bash
# Run the sqail2 desktop editor.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
exec cargo run -p sqail-ui --bin sqail2 -- "$@"
