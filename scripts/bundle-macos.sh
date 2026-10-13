#!/bin/sh
# Builds target/release/Macsplorer.app with the app icon.
# Usage: scripts/bundle-macos.sh [--open]
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p macsplorer
[ -f assets/macsplorer.icns ] || cargo run -q -p make-icon

version=$(cargo pkgid -p macsplorer | sed 's/.*[#@]//')
app=target/release/Macsplorer.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/macsplorer "$app/Contents/MacOS/macsplorer"
cp assets/macsplorer.icns "$app/Contents/Resources/macsplorer.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Macsplorer</string>
  <key>CFBundleDisplayName</key><string>Macsplorer</string>
  <key>CFBundleIdentifier</key><string>dev.macsplorer.app</string>
  <key>CFBundleExecutable</key><string>macsplorer</string>
  <key>CFBundleIconFile</key><string>macsplorer</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
</dict>
</plist>
PLIST
# Ad-hoc sign so Gatekeeper treats the bundle as one app.
codesign --force --sign - "$app" >/dev/null 2>&1 || true
# Make Finder pick up the new icon right away.
touch "$app"
echo "built $app"
[ "${1:-}" = "--open" ] && open "$app"
exit 0
