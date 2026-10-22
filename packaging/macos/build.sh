#!/usr/bin/env bash
# Unsigned macOS packaging. Run from repo root after building both targets:
#   cargo build --release --locked --target aarch64-apple-darwin
#   cargo build --release --locked --target x86_64-apple-darwin
# Usage: build.sh app   -> dist/LumaDisk.app (universal)
#        build.sh dmg   -> dist/lumadisk-macos-universal.dmg (+ .sha256)
set -euo pipefail
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=dist/LumaDisk.app
DMG=dist/lumadisk-macos-universal.dmg
case "${1:-}" in
  app)
    rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS"
    lipo -create -output "$APP/Contents/MacOS/LumaDisk" \
      target/aarch64-apple-darwin/release/lumadisk target/x86_64-apple-darwin/release/lumadisk
    chmod +x "$APP/Contents/MacOS/LumaDisk"
    lipo -info "$APP/Contents/MacOS/LumaDisk"
    sed "s/@VERSION@/$VERSION/" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
    plutil -lint "$APP/Contents/Info.plist"
    ;;
  dmg)
    rm -rf dist/dmg-stage "$DMG"; mkdir -p dist/dmg-stage
    cp -R "$APP" dist/dmg-stage/
    ln -s /Applications dist/dmg-stage/Applications
    hdiutil create -volname LumaDisk -srcfolder dist/dmg-stage -ov -format UDZO "$DMG"
    shasum -a 256 "$DMG" > "$DMG.sha256"
    ;;
  *) echo "usage: $0 app|dmg" >&2; exit 2 ;;
esac
