#!/usr/bin/env bash
# Cobserve.app: cobserve in a window of its own, for the Dock and Spotlight.
#
#   ./desktop/bundle.sh              build it into dist/Cobserve.app
#   ./desktop/bundle.sh --install    and copy it to /Applications
#
# The app is the terminal (cobserve-desktop) with cobserve itself beside it, an icon made from
# desktop/icon/cobserve.png, and an ad-hoc signature so macOS opens it without calling it
# damaged. macOS only; elsewhere run target/release/cobserve-desktop as it is.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
[[ "$(uname)" == "Darwin" ]] || { echo "Cobserve.app is for macOS; run target/release/cobserve-desktop elsewhere"; exit 1; }

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
cargo build --release -p cobserve -p cobserve-desktop

APP="$ROOT/dist/Cobserve.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/cobserve-desktop target/release/cobserve "$APP/Contents/MacOS/"

# The icon, at every size macOS asks for.
ICONSET="$(mktemp -d)/cobserve.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z $size $size desktop/icon/cobserve.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) desktop/icon/cobserve.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/cobserve.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Cobserve</string>
  <key>CFBundleDisplayName</key><string>Cobserve</string>
  <key>CFBundleIdentifier</key><string>io.github.abduldjafar.cobserve</string>
  <key>CFBundleExecutable</key><string>cobserve-desktop</string>
  <key>CFBundleIconFile</key><string>cobserve</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || echo "codesign is not here: the app is unsigned (right-click → Open the first time)"
echo "built $APP"

if [[ "${1:-}" == "--install" ]]; then
  rm -rf /Applications/Cobserve.app
  cp -R "$APP" /Applications/
  echo "installed /Applications/Cobserve.app — open it from Spotlight or the Dock"
else
  echo "open it:     open dist/Cobserve.app"
  echo "install it:  ./desktop/bundle.sh --install"
fi
