#!/usr/bin/env bash
# Build Falcon.app and wrap it in a drag-to-Applications .dmg.
#
#   scripts/build_macos_dmg.sh [target]     # default: aarch64-apple-darwin
#
# Writes target/<target>/release/Falcon.app and
# target/<target>/release/falcon-gui-macos-<arch>.dmg.
#
# Uses only what ships with macOS and Xcode's command-line tools (sips,
# iconutil, codesign, hdiutil), so the release runner needs nothing installed
# beyond Rust. `cargo bundle` would also work but is one more tool to pin.
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
case "$TARGET" in
  aarch64-apple-darwin) ARCH=arm64 ;;
  x86_64-apple-darwin) ARCH=x86_64 ;;
  *) echo "build_macos_dmg.sh: not a macOS target: $TARGET" >&2; exit 2 ;;
esac

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION=$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "falcon_mdf_gui"))')
# Matches `identifier` in gui/Cargo.toml's [package.metadata.bundle], so a
# cargo-bundle build and this one share eframe's per-app storage directory.
BUNDLE_ID="io.github.mohammad-albarham.falcon"

cargo build --release -p falcon_mdf_gui --target "$TARGET"

OUT="target/$TARGET/release"
APP="$OUT/Falcon.app"
DMG="$OUT/falcon-gui-macos-$ARCH.dmg"
rm -rf "$APP" "$DMG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$OUT/falcon" "$APP/Contents/MacOS/falcon"

# iconutil wants every slot filled from 16 px to 512@2x; the source is 512 px,
# so the 1024 px slot is the one upscale.
ICONSET="$(mktemp -d)/Falcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" gui/assets/icon.png --out "$ICONSET/icon_${size}x${size}.png" > /dev/null
  double=$((size * 2))
  sips -z "$double" "$double" gui/assets/icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Falcon.icns"
rm -rf "$(dirname "$ICONSET")"

# NSHighResolutionCapable matters: without it a bundled app renders at 1x and
# is upscaled on every Retina display, so the plot text goes soft.
# No CFBundleDocumentTypes on purpose: Finder's "Open With" delivers the file
# as an Apple Event, not argv, and the viewer does not handle that event yet.
# Declaring .mf4 here would offer an action that opens an empty window.
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Falcon</string>
  <key>CFBundleDisplayName</key><string>Falcon</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>falcon</string>
  <key>CFBundleIconFile</key><string>Falcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT OR Apache-2.0</string>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist" > /dev/null

# Ad-hoc signature, not a Developer ID one. Apple silicon refuses to run
# unsigned code at all, and the linker's own ad-hoc signature covers only the
# binary, not the bundle around it. This does not satisfy Gatekeeper for a
# downloaded copy; FIRST-RUN-MACOS.md covers that.
codesign --force --deep --sign - "$APP"
codesign --verify --strict "$APP"

# The image holds the app, an /Applications link to drag it onto, and the two
# guides a downloader needs before the app will open. macOS 27 warns that
# `hdiutil create` is deprecated in favour of `diskutil image`, but the release
# runners' older macOS has no `diskutil image`, so hdiutil stays.
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp gui/FIRST-RUN-MACOS.md "$STAGE/FIRST-RUN.md"
cp gui/RUNNING.md "$STAGE/RUNNING.md"
hdiutil create -volname "Falcon $VERSION" -srcfolder "$STAGE" \
  -fs HFS+ -format UDZO -ov "$DMG" > /dev/null
rm -rf "$STAGE"
hdiutil verify "$DMG" > /dev/null

echo "$APP"
echo "$DMG"
