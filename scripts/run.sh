#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

# Fix blank screen on Linux with some GPU drivers (WebKitGTK DMA-BUF issue)
export WEBKIT_DISABLE_DMABUF_RENDERER=1

# Current global pnpm builds need Node ≥ 22.13 (built-in node:sqlite). Prefer an
# nvm-managed toolchain so we don't pick up a newer ~/.npm-global pnpm that can
# hang on its SQLite store while `pnpm tauri` silently waits on `pnpm install`.
MIN_NODE_MAJOR=22
MIN_NODE_MINOR=13

node_meets_min() {
  local ver="$1" major minor
  major="${ver%%.*}"
  minor="${ver#*.}"
  minor="${minor%%.*}"
  [[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ ]] || return 1
  [ "$major" -gt "$MIN_NODE_MAJOR" ] && return 0
  [ "$major" -eq "$MIN_NODE_MAJOR" ] && [ "$minor" -ge "$MIN_NODE_MINOR" ]
}

# Put the newest suitable nvm Node bin dir first on PATH (node + that install's pnpm).
activate_nvm_node() {
  local nvm_dir="${NVM_DIR:-$HOME/.nvm}"
  local versions_dir="$nvm_dir/versions/node"
  [ -d "$versions_dir" ] || return 1

  local best="" best_major=0 best_minor=0 best_patch=0
  local name ver major minor patch
  for dir in "$versions_dir"/v*; do
    [ -x "$dir/bin/node" ] || continue
    name="$(basename "$dir")"
    ver="${name#v}"
    major="${ver%%.*}"
    minor="${ver#*.}"
    minor="${minor%%.*}"
    patch="${ver##*.}"
    [[ "$major" =~ ^[0-9]+$ && "$minor" =~ ^[0-9]+$ && "$patch" =~ ^[0-9]+$ ]] || continue
    node_meets_min "$ver" || continue
    if [ "$major" -gt "$best_major" ] \
      || { [ "$major" -eq "$best_major" ] && [ "$minor" -gt "$best_minor" ]; } \
      || { [ "$major" -eq "$best_major" ] && [ "$minor" -eq "$best_minor" ] && [ "$patch" -gt "$best_patch" ]; }; then
      best="$dir"
      best_major="$major"
      best_minor="$minor"
      best_patch="$patch"
    fi
  done

  [ -n "$best" ] || return 1
  export PATH="$best/bin:$PATH"
  hash -r 2>/dev/null || true
  echo "Using Node $(node -v) / pnpm $(pnpm -v) from $best"
}

ensure_toolchain() {
  # Always prefer nvm when available so ~/.npm-global/bin does not win.
  if activate_nvm_node; then
    :
  elif command -v node >/dev/null 2>&1 && node_meets_min "$(node -p "process.versions.node")"; then
    echo "Using Node $(node -v) / pnpm $(command -v pnpm >/dev/null && pnpm -v || echo 'missing') from PATH"
  else
    local active="missing"
    if command -v node >/dev/null 2>&1; then
      active="$(node -v 2>/dev/null || echo unknown)"
    fi
    echo "Error: Node.js >= ${MIN_NODE_MAJOR}.${MIN_NODE_MINOR} is required (found ${active})." >&2
    echo "Install it (e.g. \`nvm install ${MIN_NODE_MAJOR}\`) and re-run." >&2
    exit 1
  fi

  if ! command -v pnpm >/dev/null 2>&1; then
    echo "Error: pnpm not found on PATH." >&2
    exit 1
  fi

  # Smoke-check: new pnpm + old Node fails here; hung sqlite store also fails fast on -v sometimes.
  if ! pnpm -v >/dev/null 2>&1; then
    echo "Error: pnpm is not usable with the active Node $(node -v)." >&2
    exit 1
  fi
}

ensure_toolchain

if [ ! -d "$PROJECT_ROOT/node_modules" ] || [ ! -x "$PROJECT_ROOT/node_modules/.bin/tauri" ]; then
  echo "Dependencies missing — installing..."
  pnpm install
fi

MODE="${1:-dev}"

case "$MODE" in
  dev)
    # Kill anything already listening on the dev ports (DbService 5100, Vite 1420).
    for port in 5100 1420; do
      pids=$(lsof -ti tcp:"$port" 2>/dev/null || true)
      if [ -n "$pids" ]; then
        echo "Killing process(es) on port $port: $pids"
        # shellcheck disable=SC2086
        kill -9 $pids 2>/dev/null || true
      fi
    done

    echo "Starting Sqail.DbService in background..."
    "$PROJECT_ROOT/scripts/start-dbservice.sh" dev &
    DBSERVICE_PID=$!
    trap 'echo "Stopping Sqail.DbService (pid $DBSERVICE_PID)..."; kill $DBSERVICE_PID 2>/dev/null || true' EXIT INT TERM

    echo "Starting sqail in development mode..."
    # Use the local CLI directly — avoids a global pnpm re-resolving/installing first.
    pnpm exec tauri dev
    ;;
  build)
    echo "Building sqail for release..."
    pnpm exec tauri build
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
