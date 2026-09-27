#!/usr/bin/env bash
# screenshot-prep.sh — resize and position the sqail window to a fixed size so
# every marketing screenshot has identical framing.
#
# Target: 1600×1000, positioned at (120, 120) so window shadows don't clip the
# edge of most screens. Adjust WIDTH/HEIGHT/X/Y below if you prefer a different
# canonical size — just make sure it fits inside 2880×1800 when padded.
#
# Usage:
#   ./scripts/screenshot-prep.sh
#
# Requires: xdotool (X11). For Wayland sessions you'll have to resize manually —
# there's no portable Wayland equivalent.
set -euo pipefail

WIDTH=1600
HEIGHT=1000
X=120
Y=120

if [[ "${XDG_SESSION_TYPE:-}" == "wayland" ]]; then
  echo "Warning: Wayland session detected. xdotool can't resize Wayland windows." >&2
  echo "Resize the sqail window to ${WIDTH}x${HEIGHT} manually, then capture." >&2
  exit 1
fi

command -v xdotool >/dev/null 2>&1 || {
  echo "Error: xdotool not found. Install with: sudo pacman -S xdotool" >&2
  exit 1
}

# Find the sqail window. Tauri apps set WM_CLASS to the productName.
# Multiple windows may match (Tauri creates helper windows), so we pick the
# one with _NET_WM_WINDOW_TYPE_NORMAL — the actual visible app window.
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
  # Fallback: return the first candidate if none had the property
  echo "$candidates" | head -1
}
WIN_ID=$(pick_normal_window)

if [[ -z "${WIN_ID:-}" ]]; then
  echo "Error: no sqail window found. Is the app running?" >&2
  echo "Hint: launch sqail first, then re-run this script." >&2
  exit 1
fi

echo "Found sqail window: $WIN_ID"
xdotool windowactivate "$WIN_ID"
xdotool windowsize "$WIN_ID" "$WIDTH" "$HEIGHT"
xdotool windowmove "$WIN_ID" "$X" "$Y"

# Give the compositor a moment to redraw before you screenshot.
sleep 0.3

# Report final geometry for sanity
GEOM=$(xdotool getwindowgeometry --shell "$WIN_ID" | grep -E '^(WIDTH|HEIGHT|X|Y)=' | tr '\n' ' ')
echo "Window repositioned. Geometry: $GEOM"
echo "Ready to capture. Suggested: gnome-screenshot -w -f marketing/screenshots-raw/<name>.png"
