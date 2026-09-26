#!/usr/bin/env bash
# One command for day-to-day work: test DBs up, service in background, UI in front.
# Ctrl+C (or closing the UI) stops the service again.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
"$ROOT/scripts/db.sh" up
cargo build --workspace
info "starting sqail-service"
"$ROOT/target/debug/sqail-service" serve &
SERVICE_PID=$!
trap 'kill $SERVICE_PID 2>/dev/null || true' EXIT
# Wait until it answers, so the UI uses it instead of starting its own.
for _ in $(seq 1 50); do
    curl -skf "$SERVICE_URL/v1/health" >/dev/null 2>&1 && break
    kill -0 $SERVICE_PID 2>/dev/null || die "sqail-service exited (is port 7443 in use?)"
    sleep 0.2
done
info "starting sqail2 UI"
SQAIL2_AUTO_LOCAL=1 cargo run -q -p sqail-ui --bin sqail2
