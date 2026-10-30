# Macsplorer for Android (first draft)

The desktop app in `crates/macsplorer`, run as a NativeActivity through eframe and wgpu (Vulkan). This crate is
only the `cdylib` cargo-apk packs; everything else is the `macsplorer` library, which `run_android` starts. Built after
Nullamp's Android port.

## Build and run

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
sdkmanager --install "ndk;27.3.13750724"      # any recent NDK
scripts/build-android.sh install               # builds target/release/apk/macsplorer.apk, installs it with adb
adb shell am start -n dev.macsplorer.ai/android.app.NativeActivity
```

Debug logs: `adb logcat -s macsplorer`.

## What differs from the desktop

The layout is the desktop's, small buttons included. `platform::TOUCH` switches the few exceptions:

- **The Folders pane is a drawer** over the window from the left (`Explorer::folders_drawer`), closed at the start.
  Folders on the toolbar opens it; a tap on the dimmed part or picking a folder closes it.
- **A long press is the right click**: egui reports it as `secondary_clicked`, so the desktop context menus open.
- **A drag scrolls**: rows and the view background sense clicks only (`platform::item_sense`), so a swipe never
  moves files or draws a selection band.
- **Properties fills the screen** (an `Area` instead of its own window). Its close box closes the dialog only.
- The window keeps clear of the status bar, the gesture bar and the keyboard (`android::system_bar_insets`; the UI repaints
  while the keyboard is up so the window grows back when it goes away), which sit on the Windows 2000
  desktop color (`theme::color::desktop`), and has no caption buttons or size grip. Two taps count as a double tap
  (open) up to 24 points apart (`TOUCH_CLICK_DISTANCE`).
- **The back button** (and gesture) closes what is on top: a menu, a dialog (as Cancel), the drawer, an address being
  typed. Otherwise it goes back in the history, and at the first page puts the app in the background
  (`Explorer::back_button`).
- **A sideways swipe** in the Details view scrolls the columns (`pan_sideways`).

On a narrow window, on any system, Back, Folders, Refresh, Address and Go lose their text, the search box becomes a magnifier
button at the end of the toolbar that opens it under the address field (closed again by Esc, by leaving it empty or by
tapping the button) and the status bar drops the computer name (`TOOLBAR_LABELS_WIDTH` and the constants next to it).

Dates and numbers follow the phone's region settings (`android::region`: date order, 12 or 24 hours, separators,
sorting), formatted by chrono from the ICU patterns (`locale::icu_to_chrono`); month and day names stay English.

The home folder `~` is the shared storage (`/storage/emulated/0`, shown as "Internal storage"); the settings stay in
the app's private folder. Memory cards show under Locations (`places::is_user_mount`).

Listing every folder needs **All files access** (`MANAGE_EXTERNAL_STORAGE`): the system page for it opens at start
until it is allowed. For a quick test: `adb shell appops set dev.macsplorer.ai MANAGE_EXTERNAL_STORAGE allow`.

## Opening files

`platform::open_default` calls `android::open_file`, which runs `Opener.open` (Java, `java/dev/macsplorer/ai`): an
`ACTION_VIEW` intent for the type from the extension. Android refuses a `file://` address across apps, so the file goes
through `Files`, a small content provider (`content://dev.macsplorer.ai.files/<path>`, read only, not exported, only
for addresses the intent grants). cargo-apk can add neither a provider to the manifest nor a `classes.dex`, so
`scripts/package-android.sh` (run by `build-android.sh`) compiles the Java with `javac` and `d8`, adds the provider to
the manifest cargo-apk wrote, and packs, aligns and signs the APK again. It needs a JDK.

## Known gaps

- Delete moves items to a `Trash` folder in the app's own folder (`trash.rs`); nothing empties it.
- Open in Terminal, Eject and New Window don't work. Show in File Manager opens the Files app on the item's folder
  (`Opener.reveal`, a document address of the external storage provider), not on the item itself, and only for shared
  storage and memory cards; a failure is only logged.
- Dialogs other than Properties are still small windows in the middle, and their close box does nothing (use Cancel).
- The font is the bundled DejaVu Sans Condensed with no fallback for other scripts.
- On the emulator's software renderer the first start can take half a minute.
