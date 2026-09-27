#!/usr/bin/env bash
# Generates/increments build number in yyyymmdd-revision format.
# Stores state in build-number.json at project root.
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_FILE="$PROJECT_ROOT/build-number.json"
TODAY=$(date +%Y%m%d)

if [ -f "$BUILD_FILE" ]; then
  PREV_DATE=$(grep -o '"date": *"[^"]*"' "$BUILD_FILE" | grep -o '[0-9]\{8\}')
  PREV_REV=$(grep -o '"revision": *[0-9]*' "$BUILD_FILE" | grep -o '[0-9]*$')
  if [ "$PREV_DATE" = "$TODAY" ]; then
    REVISION=$((PREV_REV + 1))
  else
    REVISION=1
  fi
else
  REVISION=1
fi

cat > "$BUILD_FILE" <<EOF
{
  "date": "$TODAY",
  "revision": $REVISION,
  "buildNumber": "$TODAY-$REVISION"
}
EOF

echo "$TODAY-$REVISION"
