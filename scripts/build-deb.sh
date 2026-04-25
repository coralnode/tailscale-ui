#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-0.1.0}"
ARCH="$(dpkg --print-architecture)"
PACKAGE_NAME="tailscale-tray"
STAGE_DIR="$ROOT_DIR/dist/${PACKAGE_NAME}_${VERSION}_${ARCH}"
DEB_FILE="$ROOT_DIR/dist/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
BIN_PATH="$ROOT_DIR/target/release/tailscale-tray"

cargo build --release --locked --manifest-path "$ROOT_DIR/Cargo.toml"

rm -rf "$STAGE_DIR" "$DEB_FILE"
mkdir -p \
  "$STAGE_DIR/DEBIAN" \
  "$STAGE_DIR/usr/bin" \
  "$STAGE_DIR/usr/share/applications" \
  "$STAGE_DIR/usr/share/doc/$PACKAGE_NAME"

install -m 0755 "$BIN_PATH" "$STAGE_DIR/usr/bin/$PACKAGE_NAME"
install -m 0644 "$ROOT_DIR/README.md" "$STAGE_DIR/usr/share/doc/$PACKAGE_NAME/README.md"

cat > "$STAGE_DIR/DEBIAN/control" <<EOF
Package: $PACKAGE_NAME
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Maintainer: Rouzbeh <rouzbeh@example.com>
Depends: tailscale, xdg-utils, libdbus-1-3
Description: Tray controller for Tailscale
 A Rust tray app for controlling Tailscale, exit nodes, and the local web interface.
EOF

cat > "$STAGE_DIR/usr/share/applications/$PACKAGE_NAME.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Tailscale Tray
Comment=Tray controller for Tailscale
Exec=/usr/bin/$PACKAGE_NAME
Terminal=false
NoDisplay=true
Categories=Network;System;
EOF

dpkg-deb --build "$STAGE_DIR" "$DEB_FILE"
printf '%s\n' "$DEB_FILE"
