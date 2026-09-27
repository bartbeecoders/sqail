#!/usr/bin/env bash
# Build a release tarball: dist/sqail-<version>-linux-<arch>.tar.gz
# containing both binaries, desktop entry, icons, systemd user unit and install.sh.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
need tar "install tar"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
ARCH="$(uname -m)"
NAME="sqail-$VERSION-linux-$ARCH"
STAGE="$ROOT/dist/$NAME"

info "building release binaries"
cargo build --release -p sqail-ui -p sqail-service
info "staging $NAME"
rm -rf "$STAGE" && mkdir -p "$STAGE"
install -Dm755 target/release/sqail "$STAGE/bin/sqail"
install -Dm755 target/release/sqail-service "$STAGE/bin/sqail-service"
install -Dm644 packaging/linux/sqail.desktop "$STAGE/share/applications/sqail.desktop"
for s in 16 32 48 64 128 256 512; do
    install -Dm644 "packaging/icons/sqail-$s.png" "$STAGE/share/icons/hicolor/${s}x${s}/apps/sqail.png"
done
install -Dm644 packaging/icons/sqail.svg "$STAGE/share/icons/hicolor/scalable/apps/sqail.svg"
install -Dm644 packaging/linux/sqail-service.service "$STAGE/lib/systemd/user/sqail-service.service"
install -Dm755 packaging/linux/install.sh "$STAGE/install.sh"
install -Dm644 LICENSE "$STAGE/LICENSE"
install -Dm644 dev/sqail-service.example.toml "$STAGE/share/doc/sqail/sqail-service.example.toml"
install -Dm644 crates/sqail-ui/assets/fonts/Inter-OFL.txt "$STAGE/share/doc/sqail/licenses/Inter-OFL.txt"
install -Dm644 crates/sqail-ui/assets/fonts/JetBrainsMono-OFL.txt "$STAGE/share/doc/sqail/licenses/JetBrainsMono-OFL.txt"
for d in docs/user-guide.md docs/operations.md docs/security.md docs/api.md; do
    [[ -f "$d" ]] && install -Dm644 "$d" "$STAGE/share/doc/sqail/$(basename "$d")"
done
cat > "$STAGE/README.txt" <<TXT
sqail $VERSION: a fast SQL editor backed by sqail-service.

Install for your user (no root):   ./install.sh
System-wide:                       sudo ./install.sh --prefix /usr
Uninstall:                         ./install.sh --uninstall

Then start "sqail" from your launcher. On first run choose
"Use the local service"; it creates a token for you automatically.
Documentation: share/doc/sqail/
TXT
info "archiving"
tar -C "$ROOT/dist" -czf "$ROOT/dist/$NAME.tar.gz" "$NAME"
(cd "$ROOT/dist" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
ok "dist/$NAME.tar.gz ($(du -h "$ROOT/dist/$NAME.tar.gz" | cut -f1))"
