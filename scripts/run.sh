#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# Fix blank screen on Linux with some GPU drivers (WebKitGTK DMA-BUF issue)
export WEBKIT_DISABLE_DMABUF_RENDERER=1

MODE="${1:-dev}"

case "$MODE" in
  dev)
    # Kill anything already listening on the dev ports (DbService 5100, Vite 1420).
    for port in 5100 1420; do
      pids=$(lsof -ti tcp:"$port" 2>/dev/null || true)
      if [ -n "$pids" ]; then
        echo "Killing process(es) on port $port: $pids"
        kill -9 $pids 2>/dev/null || true
      fi
    done

    echo "Starting Sqail.DbService in background..."
    "$PROJECT_ROOT/scripts/start-dbservice.sh" dev &
    DBSERVICE_PID=$!
    trap 'echo "Stopping Sqail.DbService (pid $DBSERVICE_PID)..."; kill $DBSERVICE_PID 2>/dev/null || true' EXIT INT TERM

    echo "Starting sqail in development mode..."
    pnpm tauri dev
    ;;
  build)
    echo "Building sqail for release..."
    pnpm tauri build
    ;;
  check)
    echo "Running all checks..."
    pnpm check
    pnpm lint
    (cd src-tauri && cargo clippy -- -D warnings)
    echo "All checks passed."
    ;;
  *)
    echo "Usage: $0 {dev|build|check}"
    echo "  dev    - Run in development mode with hot reload (default)"
    echo "  build  - Build release binary"
    echo "  check  - Run tsc, eslint, and cargo clippy"
    exit 1
    ;;
esac
