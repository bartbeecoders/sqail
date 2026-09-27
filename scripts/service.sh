#!/usr/bin/env bash
# Run sqail-service (the HTTPS REST gateway). Extra args go to the service.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
exec cargo run -p sqail-service -- "$@"
