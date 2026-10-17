#!/bin/sh
# Builds target/release/macsplorer-linux-<arch>.tar.gz: the binary, a desktop
# entry, the app icons, an install script and the font license.
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
cp assets/macsplorer.desktop "$dir/macsplorer.desktop"
cp assets/fonts/LICENSE-dejavu.txt "$dir/LICENSE-dejavu.txt"
# The icon in the standard hicolor sizes (assets/hicolor, made from the
# by cargo run -p make-icon; the macOS iconset isn't in git): desktops
# don't look in a 1024x1024 folder, and with no size they can use they show the
# generic executable icon.
cp -R assets/hicolor "$dir/icons"
cp assets/icon.png "$dir/macsplorer.png"
cat > "$dir/install.sh" <<'SH'
#!/bin/sh
# Installs Macsplorer for the current user: the program, its icons and a menu
# entry, which is what gives the dock and the launcher the Macsplorer icon.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin=$HOME/.local/bin
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$bin"
install -m755 "$here/macsplorer" "$bin/macsplorer"
for icon in "$here"/icons/hicolor/*/apps/macsplorer.png; do
    size=$(basename "$(dirname "$(dirname "$icon")")")
    mkdir -p "$data/icons/hicolor/$size/apps"
    install -m644 "$icon" "$data/icons/hicolor/$size/apps/macsplorer.png"
done
# The menu entry names the program by its full path: a launcher doesn't always
# have ~/.local/bin on its PATH.
sed "s|^Exec=.*|Exec=$bin/macsplorer %f|" "$here/macsplorer.desktop" > "$here/.macsplorer.desktop.tmp"
mkdir -p "$data/applications"
install -m644 "$here/.macsplorer.desktop.tmp" "$data/applications/macsplorer.desktop"
rm -f "$here/.macsplorer.desktop.tmp"
update-desktop-database "$data/applications" 2>/dev/null || true
gtk-update-icon-cache -f -t "$data/icons/hicolor" 2>/dev/null || true
echo "Installed. Start Macsplorer from the menu, or close and reopen any running window to get the icon."
SH
chmod +x "$dir/install.sh"
cat > "$dir/README.txt" <<TXT
Macsplorer for Linux

Run ./macsplorer [folder] to try it. To install it for your user, with the
icon in the dock and the application menu:

  ./install.sh

The desktop matches a window to its icon through macsplorer.desktop, so the
icon only shows when that file and the icons are installed. Log out and in
(or restart the shell) if the dock still shows a generic icon.

Tahoma isn't bundled. If it is installed (for example from Wine or the
msttcorefonts package), Macsplorer uses it; otherwise it uses DejaVu Sans
Condensed, which is built into the binary (license: LICENSE-dejavu.txt).
TXT

tar -C "$out" -czf "$dir.tar.gz" "$(basename "$dir")"
echo "$dir.tar.gz"
