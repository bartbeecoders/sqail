#!/usr/bin/env bash
# demo-mode.sh — swap your real sqail app data for a canonical demo set while
# capturing marketing screenshots, then restore it cleanly.
#
# Usage:
#   ./scripts/demo-mode.sh enter    # back up real data, write demo data
#   ./scripts/demo-mode.sh exit     # restore real data from backup
#   ./scripts/demo-mode.sh status   # report current state
#
# What gets swapped (Linux: ~/.local/share/dev.sqail/):
#   connections.json     → three demo connections (one real SQLite, two fake)
#   query_history.json   → empty array (so the history sidebar is clean)
#   saved_queries.json   → empty array
#   ai_history.json      → empty array (no past prompts leak into screenshots)
#   metadata.json        → empty object (no cached remote schema blobs)
#
# NOT touched:
#   ai_providers.json — your AI key stays so the AI panel works in the shot.
#                       REVIEW the AI settings panel before taking that shot;
#                       redact any visible key.
#
# Safety:
#   - Refuses to enter if sqail is running (would clobber on save)
#   - Refuses to enter if a prior .real-backup already exists (prevents the
#     "entered twice, lost real data" failure mode)
#   - Exit is idempotent — missing backup means nothing to restore
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ─── Platform paths ──────────────────────────────────────────────────
case "$(uname -s)" in
  Linux*)
    APP_DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/dev.sqail"
    ;;
  Darwin*)
    APP_DATA_DIR="$HOME/Library/Application Support/dev.sqail"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    APP_DATA_DIR="${APPDATA:-$HOME/AppData/Roaming}/dev.sqail"
    ;;
  *)
    echo "Unsupported OS: $(uname -s)" >&2
    exit 1
    ;;
esac

# Files we manage. Order matters only for reporting.
MANAGED_FILES=(
  "connections.json"
  "query_history.json"
  "saved_queries.json"
  "ai_history.json"
  "metadata.json"
)

BACKUP_SUFFIX=".real-backup"
DEMO_MARKER="$APP_DATA_DIR/.demo-mode-active"
DEMO_DB="$PROJECT_ROOT/marketing/demo-data/sqail-demo.sqlite"

# ─── Helpers ─────────────────────────────────────────────────────────
log()  { printf '  %s\n' "$*"; }
die()  { printf 'Error: %s\n' "$*" >&2; exit 1; }

check_sqail_not_running() {
  if pgrep -x sqail >/dev/null 2>&1; then
    die "sqail is running. Quit it first — otherwise the app will overwrite whatever this script writes."
  fi
}

check_demo_db_exists() {
  if [[ ! -f "$DEMO_DB" ]]; then
    die "Demo database missing. Run ./scripts/seed-demo-db.sh first."
  fi
}

# ─── enter ───────────────────────────────────────────────────────────
do_enter() {
  check_sqail_not_running
  check_demo_db_exists

  mkdir -p "$APP_DATA_DIR"

  if [[ -f "$DEMO_MARKER" ]]; then
    die "Already in demo mode. Run './scripts/demo-mode.sh exit' first."
  fi

  # Fail if ANY backup file already exists. Belt-and-suspenders for the case
  # where a past demo-mode run crashed halfway through.
  for f in "${MANAGED_FILES[@]}"; do
    if [[ -f "$APP_DATA_DIR/$f$BACKUP_SUFFIX" ]]; then
      die "Stale backup found: $f$BACKUP_SUFFIX. Inspect it manually and remove before entering demo mode."
    fi
  done

  echo "Entering demo mode…"
  echo "App data dir: $APP_DATA_DIR"
  echo

  # Back up real files (cp, not mv — if something crashes mid-way, the real
  # file is still in place).
  for f in "${MANAGED_FILES[@]}"; do
    local src="$APP_DATA_DIR/$f"
    local backup="$src$BACKUP_SUFFIX"
    if [[ -f "$src" ]]; then
      cp -p "$src" "$backup"
      log "backed up $f"
    else
      # Mark "this file didn't exist" with a sentinel so exit knows to delete
      # rather than restore.
      : > "$backup"
      log "no real $f (marked absent)"
    fi
  done

  # Now write demo files.
  write_demo_connections "$APP_DATA_DIR/connections.json"
  printf '[]' > "$APP_DATA_DIR/query_history.json"
  printf '[]' > "$APP_DATA_DIR/saved_queries.json"
  printf '[]' > "$APP_DATA_DIR/ai_history.json"
  printf '{}' > "$APP_DATA_DIR/metadata.json"
  log "wrote 5 demo files"

  # Mark the marker file so status/enter/exit know where we are.
  date -Iseconds > "$DEMO_MARKER"

  echo
  echo "Demo mode active. Launch sqail and start capturing."
  echo "When you're done, run: ./scripts/demo-mode.sh exit"
  echo
  echo "Reminder: ai_providers.json was NOT swapped. Before taking the AI-panel"
  echo "shot, double-check no real API key is visible on screen."
}

# ─── exit ────────────────────────────────────────────────────────────
do_exit() {
  check_sqail_not_running

  if [[ ! -f "$DEMO_MARKER" ]]; then
    echo "Not in demo mode — nothing to restore."
    return 0
  fi

  echo "Exiting demo mode…"
  echo "App data dir: $APP_DATA_DIR"
  echo

  for f in "${MANAGED_FILES[@]}"; do
    local src="$APP_DATA_DIR/$f"
    local backup="$src$BACKUP_SUFFIX"
    if [[ ! -f "$backup" ]]; then
      log "no backup for $f (skipping)"
      continue
    fi

    if [[ ! -s "$backup" ]]; then
      # Zero-byte sentinel = "this file didn't exist before demo mode"
      rm -f "$src"
      rm -f "$backup"
      log "removed $f (was absent)"
    else
      mv "$backup" "$src"
      log "restored $f"
    fi
  done

  rm -f "$DEMO_MARKER"
  echo
  echo "Real data restored. Safe to relaunch sqail."
}

# ─── status ──────────────────────────────────────────────────────────
do_status() {
  echo "App data dir: $APP_DATA_DIR"
  if [[ -f "$DEMO_MARKER" ]]; then
    echo "Mode: DEMO (entered $(cat "$DEMO_MARKER"))"
  else
    echo "Mode: normal"
  fi
  echo
  echo "Managed files:"
  for f in "${MANAGED_FILES[@]}"; do
    local src="$APP_DATA_DIR/$f"
    local backup="$src$BACKUP_SUFFIX"
    local live="absent"
    local back="absent"
    [[ -f "$src" ]] && live="present ($(stat -c%s "$src" 2>/dev/null || stat -f%z "$src") bytes)"
    [[ -f "$backup" ]] && {
      if [[ -s "$backup" ]]; then
        back="present ($(stat -c%s "$backup" 2>/dev/null || stat -f%z "$backup") bytes)"
      else
        back="sentinel (file was absent)"
      fi
    }
    printf '  %-22s live=%s  backup=%s\n' "$f" "$live" "$back"
  done
}

# ─── Demo connections JSON ───────────────────────────────────────────
write_demo_connections() {
  local out="$1"
  # Use printf to substitute the real demo DB path into a heredoc-like payload.
  # The connection schema is `ConnectionConfig` in src-tauri/src/db/connections.rs
  # with camelCase JSON keys.
  cat > "$out" <<JSON
[
  {
    "id": "demo-bookstore",
    "name": "bookstore (sqlite)",
    "driver": "sqlite",
    "filePath": "$DEMO_DB",
    "color": "#38BDF8"
  },
  {
    "id": "demo-analytics-prod",
    "name": "analytics-prod",
    "driver": "postgres",
    "host": "analytics.internal",
    "port": 5432,
    "database": "analytics",
    "user": "readonly",
    "password": "",
    "sslMode": "require",
    "color": "#FBBF24"
  },
  {
    "id": "demo-legacy-cms",
    "name": "legacy-cms",
    "driver": "mysql",
    "host": "db-legacy.internal",
    "port": 3306,
    "database": "cms",
    "user": "app",
    "password": "",
    "color": "#94A3B8"
  }
]
JSON
}

# ─── Dispatch ────────────────────────────────────────────────────────
case "${1:-}" in
  enter)  do_enter ;;
  exit)   do_exit ;;
  status) do_status ;;
  *)
    sed -n '2,12p' "$0"
    exit 2
    ;;
esac
