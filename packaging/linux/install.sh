#!/usr/bin/env bash
# Install sqail from this release directory.
#   ./install.sh                  install into ~/.local (no root needed)
#   ./install.sh --prefix /usr    system-wide (run with sudo)
#   ./install.sh --no-service     skip the systemd user unit
#   ./install.sh --uninstall      remove what an earlier run installed
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="$HOME/.local"
SERVICE=1
UNINSTALL=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        --no-service) SERVICE=0; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        -h|--help) sed -n '2,6p' "$0"; exit 0 ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
    esac
done
# User installs put the unit in ~/.config; system installs next to other units.
if [[ "$PREFIX" == "$HOME"* ]]; then
    UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
else
    UNIT_DIR="$PREFIX/lib/systemd/user"
fi
FILES=(
    "$PREFIX/bin/sqail"
    "$PREFIX/bin/sqail-service"
    "$PREFIX/share/applications/sqail.desktop"
)
for s in 16 32 48 64 128 256 512; do FILES+=("$PREFIX/share/icons/hicolor/${s}x${s}/apps/sqail.png"); done
FILES+=("$PREFIX/share/icons/hicolor/scalable/apps/sqail.svg")
# Up to 1.0 the editor was called sqail2; its files are removed on install.
OLD_FILES=("$PREFIX/bin/sqail2" "$PREFIX/share/applications/sqail2.desktop")
for s in 16 32 48 64 128 256 512; do OLD_FILES+=("$PREFIX/share/icons/hicolor/${s}x${s}/apps/sqail2.png"); done
OLD_FILES+=("$PREFIX/share/icons/hicolor/scalable/apps/sqail2.svg")

if [[ $UNINSTALL == 1 ]]; then
    if [[ -f "$UNIT_DIR/sqail-service.service" ]] && command -v systemctl >/dev/null && [[ "$PREFIX" == "$HOME"* ]]; then
        systemctl --user disable --now sqail-service 2>/dev/null || true
    fi
    rm -f "${FILES[@]}" "${OLD_FILES[@]}" "$UNIT_DIR/sqail-service.service"
    echo "Removed sqail from $PREFIX. Your data is kept in ~/.local/share/sqail-service and ~/.config/sqail."
    exit 0
fi

rm -f "${OLD_FILES[@]}"
install -Dm755 "$HERE/bin/sqail" "$PREFIX/bin/sqail"
install -Dm755 "$HERE/bin/sqail-service" "$PREFIX/bin/sqail-service"
install -Dm644 "$HERE/share/applications/sqail.desktop" "$PREFIX/share/applications/sqail.desktop"
for s in 16 32 48 64 128 256 512; do
    install -Dm644 "$HERE/share/icons/hicolor/${s}x${s}/apps/sqail.png" "$PREFIX/share/icons/hicolor/${s}x${s}/apps/sqail.png"
done
install -Dm644 "$HERE/share/icons/hicolor/scalable/apps/sqail.svg" "$PREFIX/share/icons/hicolor/scalable/apps/sqail.svg"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true

if [[ $SERVICE == 1 ]]; then
    sed "s|%h/.local/bin/sqail-service|$PREFIX/bin/sqail-service|" "$HERE/lib/systemd/user/sqail-service.service" \
        | install -Dm644 /dev/stdin "$UNIT_DIR/sqail-service.service"
    if [[ "$PREFIX" == "$HOME"* ]] && command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; then
        systemctl --user daemon-reload
        systemctl --user enable --now sqail-service
        echo "sqail-service is running (systemctl --user status sqail-service)."
        # First start: the service leaves a one-time admin token in a private
        # file. Turn it into a sign-in link for the admin page, then remove it.
        TOKEN_FILE="${XDG_DATA_HOME:-$HOME/.local/share}/sqail-service/bootstrap-admin-token.txt"
        for _ in $(seq 20); do [[ -f "$TOKEN_FILE" ]] && break; sleep 0.25; done
        if [[ -f "$TOKEN_FILE" ]]; then
            echo
            echo "Finish the setup on the admin page (this link signs you in; it is shown once):"
            echo "    https://127.0.0.1:7443/admin/#token=$(tr -d '[:space:]' < "$TOKEN_FILE")"
            echo "Your browser warns about the self-signed certificate once; continue to the page."
            rm -f "$TOKEN_FILE"
        else
            echo "Admin page: https://127.0.0.1:7443/admin/ (a new sign-in link: sqail-service admin-link)"
        fi
    else
        echo "Installed the unit to $UNIT_DIR; enable it with: systemctl --user enable --now sqail-service"
    fi
fi
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: add $PREFIX/bin to your PATH." ;; esac
echo "Installed sqail into $PREFIX. Start it from your launcher or run: sqail"
