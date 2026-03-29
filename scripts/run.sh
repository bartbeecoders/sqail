#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# Fix blank screen on Linux with some GPU drivers (WebKitGTK DMA-BUF issue)
export WEBKIT_DISABLE_DMABUF_RENDERER=1

MODE="${1:-dev}"

case "$MODE" in
  dev)
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
