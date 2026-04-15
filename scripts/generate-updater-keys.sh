#!/usr/bin/env bash
# Generate a Tauri v2 updater signing key pair.
#
# Usage:
#   ./scripts/generate-updater-keys.sh
#
# This creates a key pair for signing auto-update bundles.
# After running:
#   1. Copy the PUBLIC key into tauri.conf.json > plugins > updater > pubkey
#   2. Add the PRIVATE key as a GitHub Actions secret: TAURI_SIGNING_PRIVATE_KEY
#   3. (Optional) Add the password as: TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#
# The private key is printed to stdout — do NOT commit it to the repo.

set -euo pipefail

if ! command -v pnpm &>/dev/null; then
  echo "Error: pnpm is required" >&2
  exit 1
fi

echo "Generating Tauri updater signing keys..."
echo "You will be prompted for an optional password."
echo ""

pnpm tauri signer generate -w ~/.tauri/sqail.key

echo ""
echo "========================================"
echo "Keys written to:"
echo "  Private: ~/.tauri/sqail.key"
echo "  Public:  ~/.tauri/sqail.key.pub"
echo ""
echo "Next steps:"
echo "  1. Copy the contents of ~/.tauri/sqail.key.pub into"
echo "     tauri.conf.json > plugins > updater > pubkey"
echo ""
echo "  2. Add the contents of ~/.tauri/sqail.key as a GitHub"
echo "     Actions secret named TAURI_SIGNING_PRIVATE_KEY"
echo ""
echo "  3. If you set a password, add it as"
echo "     TAURI_SIGNING_PRIVATE_KEY_PASSWORD"
echo "========================================"
