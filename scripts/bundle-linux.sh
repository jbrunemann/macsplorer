#!/bin/sh
# Builds target/release/macsplorer-linux-<arch>.tar.gz: the binary, a desktop
# entry, the app icon and the font license.
# Usage: scripts/bundle-linux.sh   (honors CARGO_TARGET_DIR)
set -eu
cd "$(dirname "$0")/.."

out=${CARGO_TARGET_DIR:-target}/release
cargo build --release -p macsplorer

arch=$(uname -m)
dir=$out/macsplorer-linux-$arch
rm -rf "$dir"
mkdir -p "$dir"
cp "$out/macsplorer" "$dir/macsplorer"
cp assets/icon.png "$dir/macsplorer.png"
cp assets/macsplorer.desktop "$dir/macsplorer.desktop"
cp assets/fonts/LICENSE-dejavu.txt "$dir/LICENSE-dejavu.txt"
cat > "$dir/README.txt" <<TXT
Macsplorer for Linux

Run ./macsplorer [folder], or install it for your user:

  install -Dm755 macsplorer ~/.local/bin/macsplorer
  install -Dm644 macsplorer.png ~/.local/share/icons/hicolor/1024x1024/apps/macsplorer.png
  install -Dm644 macsplorer.desktop ~/.local/share/applications/macsplorer.desktop

Tahoma isn't bundled. If it is installed (for example from Wine or the
msttcorefonts package), Macsplorer uses it; otherwise it uses DejaVu Sans
Condensed, which is built into the binary (license: LICENSE-dejavu.txt).
TXT

tar -C "$out" -czf "$dir.tar.gz" "$(basename "$dir")"
echo "$dir.tar.gz"
