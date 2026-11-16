#!/bin/sh
set -eu

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_DIR="$PREFIX/share/icons/hicolor/256x256/apps"

if [ ! -x ./koyo ]; then
    echo "Run this installer from the extracted Koyo archive." >&2
    exit 1
fi

install -d "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
install -m 755 ./koyo "$BIN_DIR/koyo"
sed "s|^Exec=.*|Exec=\"$BIN_DIR/koyo\"|" ./koyo.desktop > "$APP_DIR/koyo.desktop"
chmod 644 "$APP_DIR/koyo.desktop"
install -m 644 ./koyo.png "$ICON_DIR/koyo.png"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "Installed Koyo to $BIN_DIR/koyo"
echo "If $BIN_DIR is not in PATH, add it or run $BIN_DIR/koyo."
