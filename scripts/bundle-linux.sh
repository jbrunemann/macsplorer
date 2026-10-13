#!/bin/sh
# Builds target/release/macsplorer-linux-<arch>.tar.gz: the binary, a desktop
# entry and the app icon.
# Usage: scripts/bundle-linux.sh
set -eu
cd "$(dirname "$0")/.."

cargo build --release -p macsplorer

arch=$(uname -m)
dir=target/release/macsplorer-linux-$arch
rm -rf "$dir"
mkdir -p "$dir"
cp target/release/macsplorer "$dir/macsplorer"
cp assets/icon.png "$dir/macsplorer.png"
cp assets/macsplorer.desktop "$dir/macsplorer.desktop"
cat > "$dir/README.txt" <<TXT
Macsplorer for Linux

Run ./macsplorer [folder], or install it for your user:

  install -Dm755 macsplorer ~/.local/bin/macsplorer
  install -Dm644 macsplorer.png ~/.local/share/icons/hicolor/1024x1024/apps/macsplorer.png
  install -Dm644 macsplorer.desktop ~/.local/share/applications/macsplorer.desktop

Tahoma isn't bundled. If it is installed (for example from Wine or the
msttcorefonts package), Macsplorer uses it; otherwise it falls back to egui's
default font.
TXT

tar -C target/release -czf "$dir.tar.gz" "$(basename "$dir")"
echo "$dir.tar.gz"
