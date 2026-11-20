#!/usr/bin/env bash
# AppImage de control : packaging/linux/appimage.sh <version> <binaire> <dossier de sortie>
# Demande appimagetool dans le PATH (la CI le télécharge).
set -euo pipefail
version="$1"; bin="$2"; out="$3"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
appdir="$(mktemp -d)/control.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/512x512/apps"
install -m 755 "$bin" "$appdir/usr/bin/control"
install -m 644 "$here/control.desktop" "$appdir/control.desktop"
install -m 644 "$here/control.desktop" "$appdir/usr/share/applications/control.desktop"
install -m 644 "$root/build/icon.png" "$appdir/control.png"
install -m 644 "$root/build/icon.png" "$appdir/usr/share/icons/hicolor/512x512/apps/control.png"
ln -s usr/bin/control "$appdir/AppRun"
mkdir -p "$out"
ARCH=x86_64 appimagetool --no-appstream "$appdir" "$out/control-${version}-x86_64.AppImage"
