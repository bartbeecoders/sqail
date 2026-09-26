#!/usr/bin/env bash
# Build a release tarball: dist/sqail2-<version>-linux-<arch>.tar.gz
# containing both binaries, desktop entry, icons, systemd user unit and install.sh.
source "$(dirname "${BASH_SOURCE[0]}")/common.sh"
cd "$ROOT"
need tar "install tar"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
ARCH="$(uname -m)"
NAME="sqail2-$VERSION-linux-$ARCH"
STAGE="$ROOT/dist/$NAME"

info "building release binaries"
cargo build --release -p sqail-ui -p sqail-service
info "staging $NAME"
rm -rf "$STAGE" && mkdir -p "$STAGE"
install -Dm755 target/release/sqail2 "$STAGE/bin/sqail2"
install -Dm755 target/release/sqail-service "$STAGE/bin/sqail-service"
install -Dm644 packaging/linux/sqail2.desktop "$STAGE/share/applications/sqail2.desktop"
for s in 16 32 48 64 128 256 512; do
    install -Dm644 "packaging/icons/sqail2-$s.png" "$STAGE/share/icons/hicolor/${s}x${s}/apps/sqail2.png"
done
install -Dm644 packaging/icons/sqail2.svg "$STAGE/share/icons/hicolor/scalable/apps/sqail2.svg"
install -Dm644 packaging/linux/sqail-service.service "$STAGE/lib/systemd/user/sqail-service.service"
install -Dm755 packaging/linux/install.sh "$STAGE/install.sh"
install -Dm644 LICENSE "$STAGE/LICENSE"
install -Dm644 dev/sqail-service.example.toml "$STAGE/share/doc/sqail2/sqail-service.example.toml"
install -Dm644 crates/sqail-ui/assets/fonts/Inter-OFL.txt "$STAGE/share/doc/sqail2/licenses/Inter-OFL.txt"
install -Dm644 crates/sqail-ui/assets/fonts/JetBrainsMono-OFL.txt "$STAGE/share/doc/sqail2/licenses/JetBrainsMono-OFL.txt"
for d in docs/user-guide.md docs/operations.md docs/security.md docs/api.md; do
    [[ -f "$d" ]] && install -Dm644 "$d" "$STAGE/share/doc/sqail2/$(basename "$d")"
done
cat > "$STAGE/README.txt" <<TXT
sqail2 $VERSION: a fast SQL editor backed by sqail-service.

Install for your user (no root):   ./install.sh
System-wide:                       sudo ./install.sh --prefix /usr
Uninstall:                         ./install.sh --uninstall

Then start "sqail2" from your launcher. On first run choose
"Use the local service"; it creates a token for you automatically.
Documentation: share/doc/sqail2/
TXT
info "archiving"
tar -C "$ROOT/dist" -czf "$ROOT/dist/$NAME.tar.gz" "$NAME"
(cd "$ROOT/dist" && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
ok "dist/$NAME.tar.gz ($(du -h "$ROOT/dist/$NAME.tar.gz" | cut -f1))"
