#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_ROOT"

echo "=== Building sqail for Linux ==="

# Check prerequisites
command -v cargo >/dev/null 2>&1 || { echo "Error: Rust/Cargo not found. Install from https://rustup.rs"; exit 1; }
command -v pnpm >/dev/null 2>&1 || { echo "Error: pnpm not found. Install from https://pnpm.io"; exit 1; }

# Check system dependencies (Debian/Ubuntu)
if command -v dpkg >/dev/null 2>&1; then
  MISSING=()
  for pkg in libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev; do
    dpkg -s "$pkg" &>/dev/null || MISSING+=("$pkg")
  done
  if [ ${#MISSING[@]} -gt 0 ]; then
    echo "Missing system packages: ${MISSING[*]}"
    echo "Install with: sudo apt install ${MISSING[*]}"
    exit 1
  fi
fi

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

if [ -d "$BUNDLE_DIR/appimage" ]; then
  echo "  AppImage (single executable):"
  ls -lh "$BUNDLE_DIR/appimage/"*.AppImage 2>/dev/null || true
fi
if [ -d "$BUNDLE_DIR/deb" ]; then
  echo "  Debian package:"
  ls -lh "$BUNDLE_DIR/deb/"*.deb 2>/dev/null || true
fi
if [ -d "$BUNDLE_DIR/rpm" ]; then
  echo "  RPM package:"
  ls -lh "$BUNDLE_DIR/rpm/"*.rpm 2>/dev/null || true
fi

echo ""
echo "The AppImage is a single portable executable — just chmod +x and run it."
