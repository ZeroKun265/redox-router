#!/usr/bin/env bash
set -euo pipefail

# RedoxRouter Uninstall Script
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
            exit 0
            ;;
    esac
done

echo "Removing RedoxRouter from $PREFIX..."
rm -f "$PREFIX/bin/redox-router"
rm -f "$PREFIX/share/applications/redox-router.desktop"
rm -f "$PREFIX/share/icons/hicolor/scalable/apps/redox-router.svg"
rm -f "$PREFIX/share/icons/hicolor/128x128/apps/redox-router.png"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$PREFIX/share/applications" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$PREFIX/share/icons/hicolor" || true
fi

echo "Done! RedoxRouter removed from $PREFIX"

