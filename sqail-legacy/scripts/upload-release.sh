#!/usr/bin/env bash
#===============================================================================
# SQaiL — Upload built binaries to VPS for portal download
#===============================================================================
# Uploads all binaries from dist/releases/ to the VPS at /opt/sqail-releases/
# so they're served by the portal via nginx.
#
# Usage:
#   ./scripts/upload-release.sh                    # upload all from dist/releases/
#   ./scripts/upload-release.sh path/to/file.exe   # upload specific file(s)
#===============================================================================
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

VPS_IP="${VPS_IP:-212.47.77.32}"
VPS_USER="${VPS_USER:-bart}"
REMOTE_DIR="/opt/sqail-releases"

# Ensure remote directory exists
ssh -o StrictHostKeyChecking=accept-new "$VPS_USER@$VPS_IP" \
  "sudo mkdir -p $REMOTE_DIR && sudo chown $VPS_USER:$VPS_USER $REMOTE_DIR"

if [ $# -gt 0 ]; then
  # Upload specific files
  FILES=("$@")
else
  # Upload everything from dist/releases/
  RELEASE_DIR="$PROJECT_ROOT/dist/releases"
  if [ ! -d "$RELEASE_DIR" ] || [ -z "$(ls -A "$RELEASE_DIR" 2>/dev/null)" ]; then
    echo "No files in $RELEASE_DIR — run ./scripts/build-release.sh first."
    exit 1
  fi
  FILES=("$RELEASE_DIR"/*)
fi

echo "Uploading ${#FILES[@]} file(s) to $VPS_USER@$VPS_IP:$REMOTE_DIR ..."

for f in "${FILES[@]}"; do
  if [ -f "$f" ]; then
    FNAME=$(basename "$f")
    echo "  $FNAME ($(du -h "$f" | cut -f1))"
    scp -o StrictHostKeyChecking=accept-new "$f" "$VPS_USER@$VPS_IP:$REMOTE_DIR/$FNAME"
  fi
done

echo ""
echo "Upload complete. Files on VPS:"
ssh "$VPS_USER@$VPS_IP" "ls -lh $REMOTE_DIR/"
