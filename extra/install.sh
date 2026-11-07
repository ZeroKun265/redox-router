#!/usr/bin/env bash
set -euo pipefail

# RedoxRouter Install Script
# Usage:
#   sudo ./install.sh           # Installs system-wide to /usr/local
#   ./install.sh --user         # Installs for current user to ~/.local
#   PREFIX=/opt/custom ./install.sh

PREFIX="${PREFIX:-/usr/local}"

for arg in "$@"; do
    case "$arg" in
        --user)
            PREFIX="${XDG_DATA_HOME:-$HOME/.local}"
            ;;
        --prefix=*)
            PREFIX="${arg#*=}"
            ;;
        -h|--help)
            echo "Usage: $0 [--user | --prefix=PATH]"
            echo "  --user          Install to ~/.local (no root required)"
            echo "  --prefix=PATH   Install to custom prefix (default: /usr/local)"
            exit 0
            ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_SRC=""

# Detect binary location (current dir, parent dir, or target/release)
if [[ -f "$SCRIPT_DIR/redox-router" ]]; then
    BIN_SRC="$SCRIPT_DIR/redox-router"
elif [[ -f "$SCRIPT_DIR/../redox-router" ]]; then
    BIN_SRC="$SCRIPT_DIR/../redox-router"
elif [[ -f "$SCRIPT_DIR/../target/release/redox-router" ]]; then
    BIN_SRC="$SCRIPT_DIR/../target/release/redox-router"
else
    echo "Error: redox-router binary not found." >&2
    echo "Please build with 'cargo build --release' or run from release archive." >&2
    exit 1
fi

BIN_DIR="$PREFIX/bin"
APP_DIR="$PREFIX/share/applications"
ICON_SCALABLE="$PREFIX/share/icons/hicolor/scalable/apps"
ICON_128="$PREFIX/share/icons/hicolor/128x128/apps"

echo "Installing RedoxRouter to $PREFIX..."
mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_SCALABLE" "$ICON_128"

install -m 755 "$BIN_SRC" "$BIN_DIR/redox-router"
install -m 644 "$SCRIPT_DIR/redox-router.desktop" "$APP_DIR/redox-router.desktop"
install -m 644 "$SCRIPT_DIR/redox-router.svg" "$ICON_SCALABLE/redox-router.svg"
install -m 644 "$SCRIPT_DIR/redox-router.png" "$ICON_128/redox-router.png"

echo "Updating desktop and icon caches (if tools are available)..."
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$PREFIX/share/icons/hicolor" || true
fi

echo "Done! RedoxRouter installed successfully to $BIN_DIR/redox-router"

