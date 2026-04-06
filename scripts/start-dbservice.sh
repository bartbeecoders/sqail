#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SERVICE_DIR="$PROJECT_ROOT/sqail-dbservice/Sqail.DbService"

MODE="${1:-dev}"

case "$MODE" in
  dev)
    echo "Starting Sqail.DbService in development mode (port 5100)..."
    cd "$SERVICE_DIR"
    dotnet run --environment Development
    ;;
  build)
    echo "Building Sqail.DbService..."
    cd "$SERVICE_DIR"
    dotnet build -c Release
    echo "Build complete."
    ;;
  publish)
    OUT_DIR="$PROJECT_ROOT/sqail-dbservice/publish"
    echo "Publishing Sqail.DbService to $OUT_DIR..."
    cd "$SERVICE_DIR"
    dotnet publish -c Release -o "$OUT_DIR"
    echo "Published to $OUT_DIR"
    ;;
  run)
    echo "Running published Sqail.DbService..."
    BINARY="$PROJECT_ROOT/sqail-dbservice/publish/Sqail.DbService"
    if [ ! -f "$BINARY" ]; then
      echo "Error: published binary not found at $BINARY"
      echo "Run '$0 publish' first."
      exit 1
    fi
    "$BINARY"
    ;;
  *)
    echo "Usage: $0 {dev|build|publish|run}"
    echo "  dev     - Run in development mode with hot reload (default)"
    echo "  build   - Build in Release configuration"
    echo "  publish - Publish self-contained binary to sqail-dbservice/publish/"
    echo "  run     - Run the published binary"
    exit 1
    ;;
esac
