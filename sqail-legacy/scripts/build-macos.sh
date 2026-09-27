#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

echo "=== Building sqail for macOS ==="

# Check prerequisites
command -v cargo >/dev/null 2>&1 || { echo "Error: Rust/Cargo not found. Install from https://rustup.rs"; exit 1; }
command -v pnpm >/dev/null 2>&1 || { echo "Error: pnpm not found. Install from https://pnpm.io"; exit 1; }

# Check Xcode CLI tools
xcode-select -p >/dev/null 2>&1 || { echo "Error: Xcode CLI tools not found. Install with: xcode-select --install"; exit 1; }

# Install frontend dependencies
echo "Installing frontend dependencies..."
pnpm install --frozen-lockfile

# Build
echo "Building release..."
pnpm tauri build

# Show output
BUNDLE_DIR="$PROJECT_ROOT/src-tauri/target/release/bundle"
echo ""
echo "=== Build complete ==="
echo "Outputs:"

if [ -d "$BUNDLE_DIR/dmg" ]; then
  echo "  DMG installer:"
  ls -lh "$BUNDLE_DIR/dmg/"*.dmg 2>/dev/null || true
fi
if [ -d "$BUNDLE_DIR/macos" ]; then
  echo "  App bundle:"
  ls -dh "$BUNDLE_DIR/macos/"*.app 2>/dev/null || true
fi

echo ""
echo "The .dmg is a single distributable file. Users open it and drag the app to Applications."
