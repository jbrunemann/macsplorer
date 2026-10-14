#!/bin/sh
# Renders the screenshot tests and copies the ones the user manual uses
# into docs/manual/images. Run after a UI change that shows in the manual.
set -eu
cd "$(dirname "$0")/.."

shots="window large-icons list details-columns header-menu column-settings views-menu
view-menu view-menu-text-size item-menu open-with-menu rename search marquee-details
chevron-menu properties-folder properties-file properties-selection hidden-items text-size-larger"

out=docs/manual/images
mkdir -p "$out"

cargo test -p macsplorer -- --ignored
for shot in $shots; do
    cp "target/screenshots/$shot.png" "$out/$shot.png"
done

# Dark mode is a process-wide switch, so it gets a run of its own.
SHOT_DARK=1 cargo test -p macsplorer -- --ignored --exact app::screenshot::window
cp target/screenshots/window.png "$out/window-dark.png"
# Put the light one back for anyone looking in target/screenshots.
cargo test -p macsplorer -- --ignored --exact app::screenshot::window
