#!/bin/bash
# Build Spacious.app into dist/: a universal binary (Apple silicon and Intel).
#
#   ./bundle.sh                               ad-hoc signed, for this Mac
#   SIGN_ID="Developer ID Application: …" ./bundle.sh
#                                             signed with hardened runtime, ready to share
#   SIGN_ID=… NOTARY_PROFILE=<profile> ./bundle.sh
#                                             also notarized and stapled (profile made with
#                                             `xcrun notarytool store-credentials`)
#
# The shareable zip is dist/Spacious-<version>-macos-universal.zip.
set -euo pipefail
cd "$(dirname "$0")"

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

APP=dist/Spacious.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create \
  target/aarch64-apple-darwin/release/spacious \
  target/x86_64-apple-darwin/release/spacious \
  -output "$APP/Contents/MacOS/Spacious"

ICONSET=target/Spacious.iconset
rm -rf "$ICONSET"
"$APP/Contents/MacOS/Spacious" --iconset "$ICONSET"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/icon.icns"
cp "$ICONSET/icon_512x512@2x.png" dist/icon.png

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Spacious</string>
  <key>CFBundleDisplayName</key><string>Spacious</string>
  <key>CFBundleIdentifier</key><string>xyz.jaafer.spacious</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>Spacious</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>MIT licensed</string>
</dict></plist>
PLIST

ZIP="dist/Spacious-$VERSION-macos-universal.zip"
rm -f "$ZIP"
if [ -n "${SIGN_ID:-}" ]; then
  codesign --force --options runtime --timestamp -s "$SIGN_ID" "$APP"
  codesign --verify --strict --verbose=2 "$APP"
  ditto -c -k --keepParent "$APP" "$ZIP"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$APP"
    spctl --assess --type execute --verbose=2 "$APP"
    # Zip again so the download carries the stapled ticket.
    rm -f "$ZIP"
    ditto -c -k --keepParent "$APP" "$ZIP"
  fi
else
  codesign --force -s - "$APP"
  ditto -c -k --keepParent "$APP" "$ZIP"
fi
echo "Built $(pwd)/$APP"
echo "Zip   $(pwd)/$ZIP"
