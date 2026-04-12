#!/usr/bin/env bash
# frame-all.sh — run screenshot-frame.sh on every raw capture in
# marketing/screenshots-raw/ and drop the framed output in
# sqail.portal/public/screenshots/.
#
# Skips files that don't exist so you can capture them in any order and run
# this repeatedly as you go.
#
# Usage:
#   ./scripts/frame-all.sh           # frame every raw shot that exists
#   ./scripts/frame-all.sh editor    # frame only the named shot
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RAW_DIR="$PROJECT_ROOT/marketing/screenshots-raw"
OUT_DIR="$PROJECT_ROOT/sqail.portal/public/screenshots"
FRAMER="$PROJECT_ROOT/scripts/screenshot-frame.sh"

# Shot name → background mode (dark/light)
# light.png uses the light background; everything else is dark.
declare -A SHOTS=(
  [editor]=dark
  [connections]=dark
  [ai]=dark
  [split]=dark
  [light]=light
)

mkdir -p "$RAW_DIR" "$OUT_DIR"

if [[ ! -x "$FRAMER" ]]; then
  echo "Error: $FRAMER not found or not executable." >&2
  exit 1
fi

TARGETS=()
if [[ $# -gt 0 ]]; then
  TARGETS=("$@")
else
  TARGETS=("${!SHOTS[@]}")
fi

framed=0
skipped=0
for name in "${TARGETS[@]}"; do
  bg="${SHOTS[$name]:-dark}"
  raw="$RAW_DIR/$name.png"
  out="$OUT_DIR/$name.png"

  if [[ ! -f "$raw" ]]; then
    printf '  %-14s skipped (no raw at %s)\n' "$name" "$raw"
    skipped=$((skipped + 1))
    continue
  fi

  "$FRAMER" --bg "$bg" "$raw" "$out" >/dev/null
  printf '  %-14s framed → %s (%s bg)\n' "$name" "$out" "$bg"
  framed=$((framed + 1))
done

echo
echo "Done: $framed framed, $skipped skipped."
