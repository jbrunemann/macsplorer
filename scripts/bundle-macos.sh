#!/bin/sh
# Builds target/release/Macsplorer.app with the app icon.
# Usage: scripts/bundle-macos.sh [--open] [--universal]
#   --universal  build for Apple silicon and Intel and merge them with lipo
#                (needs both rustup targets; used by the release workflow)
set -eu
cd "$(dirname "$0")/.."

open_app=0
universal=0
for arg in "$@"; do
  case "$arg" in
    --open) open_app=1 ;;
    --universal) universal=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

if [ "$universal" = 1 ]; then
  cargo build --release -p macsplorer --target aarch64-apple-darwin
  cargo build --release -p macsplorer --target x86_64-apple-darwin
  lipo -create -output target/release/macsplorer \
    target/aarch64-apple-darwin/release/macsplorer \
    target/x86_64-apple-darwin/release/macsplorer
else
  cargo build --release -p macsplorer
fi
[ -f assets/macsplorer.icns ] || cargo run -q -p make-icon

version=$(cargo pkgid -p macsplorer | sed 's/.*[#@]//')
app=target/release/Macsplorer.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/macsplorer "$app/Contents/MacOS/macsplorer"
cp assets/macsplorer.icns "$app/Contents/Resources/macsplorer.icns"
# English is the app's only language: Foundation then formats dates and
# numbers for the user's region (en_NL, en_DE) with English words, and Kind
# names stay English too.
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Macsplorer</string>
  <key>CFBundleDisplayName</key><string>Macsplorer</string>
  <key>CFBundleIdentifier</key><string>dev.macsplorer.ai</string>
  <key>CFBundleExecutable</key><string>macsplorer</string>
  <key>CFBundleIconFile</key><string>macsplorer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string></array>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSLocalNetworkUsageDescription</key><string>Macsplorer looks for LocalSend devices on your network so you can send them files.</string>
</dict>
</plist>
PLIST
# Ad-hoc sign so Gatekeeper treats the bundle as one app.
codesign --force --sign - "$app" >/dev/null 2>&1 || true
# Make Finder pick up the new icon right away.
touch "$app"
echo "built $app"
[ "$open_app" = 1 ] && open "$app"
exit 0
