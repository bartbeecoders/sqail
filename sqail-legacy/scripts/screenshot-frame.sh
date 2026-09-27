#!/usr/bin/env bash
# screenshot-frame.sh — wrap a raw sqail screenshot in consistent chrome for
# the portal, README, and press kit. Produces a 2x PNG with drop shadow on a
# branded gradient background.
#
# Usage:
#   ./scripts/screenshot-frame.sh input.png [output.png]
#   ./scripts/screenshot-frame.sh --bg dark input.png         # default
#   ./scripts/screenshot-frame.sh --bg light input.png out.png
#
# If output is omitted, writes alongside the input as <name>.framed.png.
# Output dimensions: 2880x1800 (2x the brand sheet). The source screenshot is
# fitted with 120px padding on all sides and a 40px blurred drop shadow.
#
# Dependencies: ImageMagick (magick or convert). Install on:
#   arch:   sudo pacman -S imagemagick
#   debian: sudo apt install imagemagick
#   macos:  brew install imagemagick
set -euo pipefail

# Pick the right ImageMagick binary (v7 uses `magick`, v6 uses `convert`).
if command -v magick >/dev/null 2>&1; then
  IM="magick"
elif command -v convert >/dev/null 2>&1; then
  IM="convert"
else
  echo "Error: ImageMagick not found. Install it and retry." >&2
  exit 1
fi

# Brand colors from marketing/brand-guide.md §3
BG_DARK_TOP="#0F172A"    # bg-primary
BG_DARK_BOT="#1E293B"    # bg-section
BG_LIGHT_TOP="#F8FAFC"   # text-primary (used as light bg)
BG_LIGHT_BOT="#E2E8F0"   # subtle cool gray

# Output sheet dimensions (2x DPI per brand-guide §8)
OUT_W=2880
OUT_H=1800
PADDING=120             # padding around the screenshot
SHADOW_OPACITY=65       # percent
SHADOW_BLUR=40
SHADOW_OFFSET="+0+20"

# Parse args
BG="dark"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bg)
      BG="$2"
      shift 2
      ;;
    --help|-h)
      sed -n '2,20p' "$0"
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 [--bg dark|light] input.png [output.png]" >&2
  exit 2
fi

INPUT="$1"
OUTPUT="${2:-${INPUT%.*}.framed.png}"

if [[ ! -f "$INPUT" ]]; then
  echo "Error: input file not found: $INPUT" >&2
  exit 1
fi

if [[ "$BG" == "light" ]]; then
  GRAD_TOP="$BG_LIGHT_TOP"
  GRAD_BOT="$BG_LIGHT_BOT"
else
  GRAD_TOP="$BG_DARK_TOP"
  GRAD_BOT="$BG_DARK_BOT"
fi

# Max content area for the screenshot (inside padding)
MAX_W=$((OUT_W - 2 * PADDING))
MAX_H=$((OUT_H - 2 * PADDING))

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# Step 1: build the gradient background sheet
$IM -size "${OUT_W}x${OUT_H}" \
  gradient:"${GRAD_TOP}-${GRAD_BOT}" \
  "$TMPDIR/bg.png"

# Step 2: scale the screenshot to fit inside the content area while preserving aspect
$IM "$INPUT" \
  -resize "${MAX_W}x${MAX_H}>" \
  -bordercolor none -border 0 \
  "$TMPDIR/shot.png"

# Step 3: build a drop shadow from the screenshot's silhouette
$IM "$TMPDIR/shot.png" \
  \( +clone -background black -shadow "${SHADOW_OPACITY}x${SHADOW_BLUR}${SHADOW_OFFSET}" \) \
  +swap -background none -layers merge +repage \
  "$TMPDIR/shot-shadow.png"

# Step 4: composite onto the background, centered
$IM "$TMPDIR/bg.png" \
  "$TMPDIR/shot-shadow.png" -gravity center -composite \
  -strip -quality 95 \
  "$OUTPUT"

echo "Wrote $OUTPUT"
