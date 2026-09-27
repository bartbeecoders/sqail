#!/usr/bin/env bash
# Clones llama.cpp at a pinned tag and builds llama-server with CUDA support.
# Output binaries land in .cache/inline-ai/bin/ so they are gitignored and
# easy to clean up.
#
# Requirements on Linux: cmake, gcc, nvcc (CUDA 12.x), git.
#
# Re-running this script is a no-op once the build artifacts exist. Pass
# FORCE=1 to rebuild from scratch.
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE_DIR="$PROJECT_ROOT/.cache/inline-ai"
SRC_DIR="$CACHE_DIR/src/llama.cpp"
BUILD_DIR="$CACHE_DIR/src/llama.cpp/build"
BIN_DIR="$CACHE_DIR/bin"

# Pin to a known-good release tag (bump deliberately).
LLAMA_TAG="${LLAMA_TAG:-b8815}"

mkdir -p "$CACHE_DIR/src" "$BIN_DIR"

if [[ "${FORCE:-0}" == "1" ]]; then
  rm -rf "$SRC_DIR" "$BIN_DIR"/llama-*
fi

if [[ ! -d "$SRC_DIR/.git" ]]; then
  echo "=== Cloning llama.cpp $LLAMA_TAG ==="
  git clone --depth 1 --branch "$LLAMA_TAG" \
    https://github.com/ggml-org/llama.cpp.git "$SRC_DIR"
else
  echo "=== Reusing existing clone at $SRC_DIR ==="
  (cd "$SRC_DIR" && git fetch --depth 1 origin "$LLAMA_TAG" && git checkout "$LLAMA_TAG")
fi

if [[ -x "$BIN_DIR/llama-server" && "${FORCE:-0}" != "1" ]]; then
  echo "=== llama-server already built at $BIN_DIR/llama-server ==="
  "$BIN_DIR/llama-server" --version || true
  exit 0
fi

echo "=== Configuring CUDA build ==="
# Target architectures: 89 = Ada Lovelace (RTX 4080 Super). We stay narrow
# on purpose so the build is fast for a spike — widen for a release build.
cmake -S "$SRC_DIR" -B "$BUILD_DIR" \
  -DCMAKE_BUILD_TYPE=Release \
  -DGGML_CUDA=ON \
  -DCMAKE_CUDA_ARCHITECTURES="89" \
  -DLLAMA_BUILD_SERVER=ON \
  -DLLAMA_BUILD_TESTS=OFF \
  -DLLAMA_BUILD_EXAMPLES=OFF \
  -DLLAMA_CURL=OFF

echo "=== Building (this can take 10-20 min) ==="
cmake --build "$BUILD_DIR" --config Release \
  --target llama-server llama-cli llama-bench \
  --parallel "$(nproc)"

echo "=== Installing into $BIN_DIR ==="
for exe in llama-server llama-cli llama-bench; do
  src="$BUILD_DIR/bin/$exe"
  [[ -x "$src" ]] || { echo "missing built binary: $src"; exit 1; }
  cp "$src" "$BIN_DIR/$exe"
done

# Copy libggml* / libllama* shared libs alongside the binaries so the
# binaries work without LD_LIBRARY_PATH gymnastics.
find "$BUILD_DIR/bin" -maxdepth 1 -name '*.so*' -exec cp -P {} "$BIN_DIR/" \;

echo "=== Done. Built binaries: ==="
ls -lh "$BIN_DIR"/llama-* 2>/dev/null | awk '{print $NF, "(" $5 ")"}'
"$BIN_DIR/llama-server" --version 2>&1 | head -3 || true
