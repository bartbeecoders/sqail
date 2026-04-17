#!/usr/bin/env bash
# Prepare platform-specific `llama-server` binaries for bundling via
# Tauri's `externalBin`. Intended to run before `cargo tauri build`.
#
# Phase B: partially implemented.
#
# Today this script only covers the host platform — Linux x86_64 with
# CUDA — by delegating to scripts/fetch-llama-cpp.sh (build from source)
# and copying the resulting binary into src-tauri/binaries/ under the
# Tauri-expected naming convention:
#
#   src-tauri/binaries/llama-server-<target-triple>[.exe]
#
# To complete externalBin wiring for release (see Phase G), this script
# should additionally:
#   - Download official llama.cpp release assets (tag b8815) for:
#       * cudart-llama-bin-win-cuda-12.4-x64.zip     (win x86_64 CUDA)
#       * llama-b8815-bin-win-cpu-x64.zip            (win x86_64 CPU)
#       * llama-b8815-bin-macos-arm64.tar.gz         (mac aarch64 Metal)
#       * llama-b8815-bin-macos-x64.tar.gz           (mac x86_64 Metal)
#       * llama-b8815-bin-ubuntu-vulkan-x64.tar.gz   (linux x86_64 fallback)
#   - Extract llama-server (+ shared libs) into src-tauri/binaries/
#   - Register each binary in tauri.conf.json under bundle.externalBin.
#
# Official releases don't ship a Linux CUDA build, so the default Linux
# target is always built from source (see scripts/fetch-llama-cpp.sh).
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BINARIES_DIR="$PROJECT_ROOT/src-tauri/binaries"
mkdir -p "$BINARIES_DIR"

# Resolve the Rust host target triple — Tauri's externalBin lookup uses
# this exact string in the bundled filename.
if ! command -v rustc >/dev/null 2>&1; then
  echo "rustc not found; install rust first." >&2
  exit 1
fi
HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"

echo "=== Host target: $HOST_TRIPLE"

case "$HOST_TRIPLE" in
  x86_64-unknown-linux-gnu)
    echo "=== Building llama-server from source (CUDA)"
    "$PROJECT_ROOT/scripts/fetch-llama-cpp.sh"
    SRC="$PROJECT_ROOT/.cache/inline-ai/bin/llama-server"
    if [[ ! -x "$SRC" ]]; then
      echo "Expected $SRC to exist after build" >&2
      exit 1
    fi
    cp "$SRC" "$BINARIES_DIR/llama-server-$HOST_TRIPLE"
    # Copy shared libs into the same dir so they're discoverable at
    # runtime. Tauri copies everything under binaries/ into the bundle.
    find "$PROJECT_ROOT/.cache/inline-ai/bin" -maxdepth 1 -name '*.so*' \
      -exec cp -P {} "$BINARIES_DIR/" \;
    ;;
  *)
    echo "Non-Linux host — this script doesn't yet pull prebuilt binaries." >&2
    echo "See the comment block at the top of this file for the TODO list." >&2
    exit 1
    ;;
esac

echo
echo "=== Bundled binaries ready:"
ls -lh "$BINARIES_DIR"
echo
echo "Next: enable bundle.externalBin in src-tauri/tauri.conf.json, e.g."
cat <<'JSON'
  "bundle": {
    "externalBin": ["binaries/llama-server"]
  }
JSON
