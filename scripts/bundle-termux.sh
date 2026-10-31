#!/bin/sh
# Builds target/release/macsplorer-termux-<arch>.tar.gz: Macsplorer for Termux, shown in a Termux:X11 window
# (a desktop on a Samsung DeX, Chromebook or tablet with a mouse). It is a normal Android (Bionic) program with
# the desktop Linux code and an X11 window, so it runs in plain Termux, with no proot and no glibc.
#
# Usage: scripts/bundle-termux.sh [aarch64|x86_64]
#   On a Mac or Linux machine it cross-compiles with the Android NDK (ANDROID_NDK_HOME, or the newest one in the
#   Android SDK) and `rustup target add aarch64-linux-android` must have been run.
#   In Termux itself (pkg install rust clang) it builds natively.
#
# winit, glutin and a few crates around them turn their X11 code off for every Android target. Their source is
# copied out of the cargo registry and patched by scripts/termux-patch.py for this build only (cargo
# `--config patch`), so the Android app and the other builds are not touched. Cargo.lock is put back afterwards.
set -eu
cd "$(dirname "$0")/.."

arch=${1:-aarch64}
target=$arch-linux-android
out=${CARGO_TARGET_DIR:-target}/release
work=${CARGO_TARGET_DIR:-target}/termux

bin=
if [ "$(uname -o 2>/dev/null)" != Android ]; then
    ndk=${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}
    if [ -z "$ndk" ]; then
        for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Library/Android/sdk" "$HOME/Android/Sdk"; do
            [ -n "$sdk" ] && [ -d "$sdk/ndk" ] && ndk=$(ls -d "$sdk"/ndk/* | sort -V | tail -1) && break
        done
    fi
    [ -n "$ndk" ] || { echo "Android NDK not found: set ANDROID_NDK_HOME" >&2; exit 1; }
    host=$(ls "$ndk/toolchains/llvm/prebuilt")
    bin=$ndk/toolchains/llvm/prebuilt/$host/bin
    # Termux's own packages target API 24 and up.
    cc=$bin/$arch-linux-android24-clang
    var=$(echo "$target" | tr 'a-z-' 'A-Z_')
    export "CC_$(echo "$target" | tr - _)=$cc" "AR_$(echo "$target" | tr - _)=$bin/llvm-ar" "CARGO_TARGET_${var}_LINKER=$cc"
fi

# The program finds Termux's X11 libraries with dlopen, which looks in the RUNPATH of the caller.
export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,/data/data/com.termux/files/usr/lib"

mkdir -p "$work"
cargo fetch --locked --target "$target" >/dev/null
python3 -I scripts/termux-patch.py Cargo.lock "$work/patched" > "$work.list"
set --
while IFS='=' read -r name path; do
    set -- "$@" --config "patch.crates-io.$name.path=\"$path\""
done < "$work.list"

# The patch rewrites Cargo.lock's entries for the patched crates (they lose their registry source).
cp Cargo.lock "$work/Cargo.lock.orig"
trap 'cp "$work/Cargo.lock.orig" Cargo.lock' EXIT
cargo build --release -p macsplorer --bin macsplorer --features termux --target "$target" "$@"

dir=$out/macsplorer-termux-$arch
rm -rf "$dir"
mkdir -p "$dir"
cp "target/$target/release/macsplorer" "$dir/macsplorer"
"${bin:+$bin/llvm-}strip" "$dir/macsplorer"
cp assets/macsplorer.desktop "$dir/macsplorer.desktop"
cp assets/fonts/LICENSE-dejavu.txt "$dir/LICENSE-dejavu.txt"
mkdir -p "$dir/icons"
cp -R assets/hicolor "$dir/icons/hicolor"
cat > "$dir/install.sh" <<'SH'
#!/bin/sh
# Installs Macsplorer into Termux: the X11 libraries it needs, the program, its icons and a menu entry.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
prefix=${PREFIX:-/data/data/com.termux/files/usr}
for pkg in libx11 libxcb libxcursor libxi libxrandr libxkbcommon mesa xdg-utils; do
    pkg install -y "$pkg" >/dev/null 2>&1 || echo "Couldn't install $pkg: install it yourself if Macsplorer won't start."
done
install -Dm755 "$here/macsplorer" "$prefix/bin/macsplorer"
for icon in "$here"/icons/hicolor/*/apps/macsplorer.png; do
    size=$(basename "$(dirname "$(dirname "$icon")")")
    install -Dm644 "$icon" "$prefix/share/icons/hicolor/$size/apps/macsplorer.png"
done
install -Dm644 "$here/macsplorer.desktop" "$prefix/share/applications/macsplorer.desktop"
echo "Installed. Start Termux:X11, then in Termux run: termux-x11 :0 & DISPLAY=:0 macsplorer"
SH
chmod +x "$dir/install.sh"
cat > "$dir/README.txt" <<'TXT'
Macsplorer for Termux

Needs Termux (from F-Droid or GitHub) and the Termux:X11 app. Install it with:

  ./install.sh

Then start the X server and the program:

  termux-x11 :0 &
  DISPLAY=:0 macsplorer [folder]

Open the Termux:X11 app to see the window. Under a desktop environment such as xfce4
(pkg install x11-repo xfce4) the menu has an entry for Macsplorer. The window draws its
own title bar, so a window manager is optional.

Software OpenGL (llvmpipe) is what Mesa uses by default. For the GPU, install
mesa-zink and virglrenderer-android and start the server with
`virgl_test_server_android &` and `GALLIUM_DRIVER=virpipe`.

Tahoma isn't bundled. If it is installed, Macsplorer uses it; otherwise it uses DejaVu Sans
Condensed, which is built into the binary (license: LICENSE-dejavu.txt).
TXT

tar -C "$out" -czf "$dir.tar.gz" "$(basename "$dir")"
echo "$dir.tar.gz"
