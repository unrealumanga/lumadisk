#!/usr/bin/env bash
# Unsigned Linux packaging: AppImage + .deb. Run from repo root after `cargo build --release --locked`.
# Env: APPIMAGETOOL (path to appimagetool; AppImage skipped if unset).
set -euo pipefail
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
BIN=target/release/lumadisk
P=packaging/linux
mkdir -p dist
python3 packaging/make_icon.py dist/lumadisk.png

# ---- AppImage ----
if [ -n "${APPIMAGETOOL:-}" ]; then
  AD=dist/LumaDisk.AppDir
  rm -rf "$AD"; mkdir -p "$AD/usr/bin"
  install -m755 "$BIN" "$AD/usr/bin/lumadisk-bin"
  install -m755 "$P/lumadisk-launcher" "$AD/usr/bin/lumadisk"
  printf '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/usr/bin/lumadisk" "$@"\n' > "$AD/AppRun"
  chmod +x "$AD/AppRun"
  cp "$P/lumadisk.desktop" "$AD/lumadisk.desktop"
  cp dist/lumadisk.png "$AD/lumadisk.png"
  ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$APPIMAGETOOL" "$AD" dist/lumadisk-linux-x64.AppImage
  sha256sum dist/lumadisk-linux-x64.AppImage > dist/lumadisk-linux-x64.AppImage.sha256
else
  echo "APPIMAGETOOL not set: skipping AppImage"
fi

# ---- .deb (plain dpkg-deb) ----
S=dist/deb-stage
rm -rf "$S"
install -d "$S/DEBIAN" "$S/usr/bin" "$S/usr/lib/lumadisk" "$S/usr/share/applications" \
  "$S/usr/share/icons/hicolor/256x256/apps"
install -m755 "$BIN" "$S/usr/lib/lumadisk/lumadisk-bin"
install -m755 "$P/lumadisk-launcher" "$S/usr/lib/lumadisk/lumadisk"
ln -s ../lib/lumadisk/lumadisk "$S/usr/bin/lumadisk"
install -m644 "$P/lumadisk.desktop" "$S/usr/share/applications/lumadisk.desktop"
install -m644 dist/lumadisk.png "$S/usr/share/icons/hicolor/256x256/apps/lumadisk.png"
SIZE=$(du -sk "$S/usr" | cut -f1)
cat > "$S/DEBIAN/control" <<CTL
Package: lumadisk
Version: $VERSION
Section: utils
Priority: optional
Architecture: amd64
Depends: libc6, libxkbcommon0, libgl1 | libegl1
Installed-Size: $SIZE
Maintainer: LumaDisk <noreply@lumadisk.invalid>
Homepage: https://github.com/unrealumanga/lumadisk
Description: Fast, private disk space analyzer
 Treemap heatmap and duplicate finder. Unsigned build.
CTL
dpkg-deb --root-owner-group --build "$S" dist/lumadisk-linux-x64.deb
sha256sum dist/lumadisk-linux-x64.deb > dist/lumadisk-linux-x64.deb.sha256
