#!/usr/bin/env bash
# screenshot-capture.sh — focus the sqail window and capture it.
#
# Usage:
#   ./scripts/screenshot-capture.sh <name>
#
# Examples:
#   ./scripts/screenshot-capture.sh editor
#   ./scripts/screenshot-capture.sh connections
#
# The screenshot is saved to marketing/screenshots-raw/<name>.png.
# Requires: xdotool, gnome-screenshot
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RAW_DIR="$(cd "$SCRIPT_DIR/.." && pwd)/marketing/screenshots-raw"

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <name>" >&2
  echo "  e.g. $0 editor" >&2
  exit 1
fi

NAME="$1"
OUTFILE="$RAW_DIR/${NAME}.png"

mkdir -p "$RAW_DIR"

for cmd in xdotool gnome-screenshot xprop; do
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "Error: $cmd not found." >&2
    exit 1
  }
done

# Reuse the same window-finding logic as screenshot-prep.sh
pick_normal_window() {
  local candidates
  candidates=$(xdotool search --class "sqail" 2>/dev/null || true)
  if [[ -z "$candidates" ]]; then
    candidates=$(xdotool search --name "^sqail" 2>/dev/null || true)
  fi
  for wid in $candidates; do
    if xprop -id "$wid" _NET_WM_WINDOW_TYPE 2>/dev/null \
       | grep -q '_NET_WM_WINDOW_TYPE_NORMAL'; then
      echo "$wid"
      return
    fi
  done
  echo "$candidates" | head -1
}

WIN_ID=$(pick_normal_window)

if [[ -z "${WIN_ID:-}" ]]; then
  echo "Error: no sqail window found. Is the app running?" >&2
  exit 1
fi

# Focus the sqail window and let the compositor settle
xdotool windowactivate --sync "$WIN_ID"
sleep 0.5

gnome-screenshot -w -f "$OUTFILE"

echo "Saved: $OUTFILE"
