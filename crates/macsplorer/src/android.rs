//! What only Android has: the activity, the system bars that cover the window,
//! the "All files access" permission and the app's own folders. The activity's
//! Java side is reached over JNI.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use android_activity::AndroidApp;
use jni::JNIEnv;
use jni::objects::{JClass, JObject};

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Keeps the activity for the calls below. Call once, before the UI starts.
pub fn set_app(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Runs `f` with the activity on this thread, attached to the VM.
fn with_activity<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<T>) -> jni::errors::Result<T> {
    let app = APP.get().ok_or(jni::errors::Error::NullPtr("no activity yet"))?;
    // SAFETY: the pointer is the VM the activity runs in, valid for the life of the process.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    // SAFETY: a global reference android-activity owns; a `JObject` never deletes it.
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    f(&mut env, &activity)
}

/// How far the system covers the window, in pixels.
#[derive(Clone, Copy, Default)]
struct Insets {
    top: i32,
    bottom: i32,
    /// The on-screen keyboard is up (its height is in `bottom`).
    keyboard: bool,
}

/// How far the status bar, the gesture bar and the on-screen keyboard cover the window, in
/// points: (top, bottom). Asked of the window four times a second at most:
/// `AndroidApp::content_rect` reports a top that includes an action bar the app doesn't have.
pub fn system_bar_insets(pixels_per_point: f32) -> (f32, f32) {
    static LAST: Mutex<Option<(Instant, Insets)>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return (0.0, 0.0) };
    let insets = match *last {
        Some((at, insets)) if at.elapsed() < INSETS_POLL => insets,
        _ => {
            let insets = window_insets().unwrap_or_else(|e| {
                log::warn!("window insets: {e}");
                Insets::default()
            });
            KEYBOARD_UP.store(insets.keyboard, Ordering::Relaxed);
            *last = Some((Instant::now(), insets));
            insets
        }
    };
    (insets.top as f32 / pixels_per_point, insets.bottom as f32 / pixels_per_point)
}

/// How long the window's insets are remembered. Also the pace of the repaints while the keyboard
/// is up, so the window grows back soon after the keyboard goes away.
pub const INSETS_POLL: Duration = Duration::from_millis(250);

static KEYBOARD_UP: AtomicBool = AtomicBool::new(false);

/// Whether the last look at the window's insets found the on-screen keyboard up. Nothing wakes
/// the UI when the keyboard goes away (Android sends the app no event the UI loop sees), so the
/// UI keeps repainting while this is true.
pub fn keyboard_up() -> bool {
    KEYBOARD_UP.load(Ordering::Relaxed)
}

/// `activity.getWindow().getDecorView().getRootWindowInsets()`, top and bottom in pixels, and
/// whether the keyboard is up (API 30; before that the UI repaints while a text field has focus).
fn window_insets() -> jni::errors::Result<Insets> {
    with_activity(|env, activity| {
        let window = env.call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
        let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
        let insets = env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
        if insets.is_null() {
            return Ok(Insets::default());
        }
        let top = env.call_method(&insets, "getSystemWindowInsetTop", "()I", &[])?.i()?;
        let bottom = env.call_method(&insets, "getSystemWindowInsetBottom", "()I", &[])?.i()?;
        let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
        let mut keyboard = false;
        if sdk >= 30 {
            let ime = env.call_static_method("android/view/WindowInsets$Type", "ime", "()I", &[])?.i()?;
            let i = env.call_method(&insets, "getInsets", "(I)Landroid/graphics/Insets;", &[ime.into()])?.l()?;
            keyboard = env.get_field(&i, "bottom", "I")?.i()? > 0;
        }
        Ok(Insets { top, bottom, keyboard })
    })
}

/// Whether the app may list and change every file of the shared storage
/// ("All files access", `MANAGE_EXTERNAL_STORAGE`). Without it Android shows
/// only media files and the app's own folder. Remembered for a second.
pub fn all_files_access() -> bool {
    static LAST: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return true };
    if let Some((at, granted)) = *last
        && at.elapsed() < Duration::from_secs(1)
    {
        return granted;
    }
    let granted = storage_manager().unwrap_or_else(|e| {
        log::warn!("all files access: {e}");
        true
    });
    *last = Some((Instant::now(), granted));
    granted
}

/// `Environment.isExternalStorageManager()`; Android 10 and older have no such permission.
fn storage_manager() -> jni::errors::Result<bool> {
    with_activity(|env, _| {
        let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
        if sdk < 30 {
            return Ok(true);
        }
        env.call_static_method("android/os/Environment", "isExternalStorageManager", "()Z", &[])?.z()
    })
}

/// Opens the system page where the user allows "All files access" for Macsplorer.
pub fn request_all_files_access() {
    let opened = with_activity(|env, activity| {
        let package = env.call_method(activity, "getPackageName", "()Ljava/lang/String;", &[])?.l()?;
        let package: String = env.get_string(&package.into())?.into();
        let uri_text = env.new_string(format!("package:{package}"))?;
        let uri = env.call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[(&uri_text).into()])?.l()?;
        let action = env.new_string("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION")?;
        let intent = env.new_object("android/content/Intent", "(Ljava/lang/String;Landroid/net/Uri;)V", &[(&action).into(), (&uri).into()])?;
        env.call_method(activity, "startActivity", "(Landroid/content/Intent;)V", &[(&intent).into()])?;
        Ok(())
    });
    if let Err(e) = opened {
        log::warn!("all files access page: {e}");
    }
}

/// Opens `path` in the app Android picks for its type: `Opener.open` in
/// `crates/macsplorer-android/java` starts the intent, with the file handed over
/// through the `Files` content provider (another app can't read a `file://`
/// address). The error is the reason to show the user.
pub fn open_file(path: &Path) -> Result<(), String> {
    call_opener("open", path)
}

/// Shows the folder holding `path` in the system's Files app (`Opener.reveal`).
pub fn reveal(path: &Path) -> Result<(), String> {
    call_opener("reveal", path)
}

/// Calls `Opener.<method>(activity, path)`, which answers `null` or the reason it failed.
fn call_opener(method: &str, path: &Path) -> Result<(), String> {
    let result = with_activity(|env, activity| {
        let called = (|| {
            // NativeActivity's native thread only finds system classes; the app's own come from its class loader.
            let loader = env.call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string("dev.macsplorer.ai.Opener")?;
            let class = env.call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[(&name).into()])?.l()?;
            let path = env.new_string(path.to_string_lossy())?;
            let reason = env.call_static_method(
                JClass::from(class),
                method,
                "(Landroid/app/Activity;Ljava/lang/String;)Ljava/lang/String;",
                &[activity.into(), (&path).into()],
            )?;
            let reason = reason.l()?;
            if reason.is_null() { Ok(None) } else { env.get_string(&reason.into()).map(|s| Some(String::from(s))) }
        })();
        if called.is_err() {
            let _ = env.exception_clear();
        }
        called
    });
    match result {
        Ok(None) => Ok(()),
        Ok(Some(reason)) => Err(reason),
        Err(e) => Err(e.to_string()),
    }
}

/// Puts the app behind the home screen without ending it
/// (`Activity.moveTaskToBack`), what the back button does on an app's first page.
pub fn to_background() {
    let moved = with_activity(|env, activity| env.call_method(activity, "moveTaskToBack", "(Z)Z", &[true.into()])?.z());
    if let Err(e) = moved {
        log::warn!("to background: {e}");
    }
}

/// The phone's region settings, as Settings > Language and region and
/// Date and time set them. Patterns are ICU's (`M/d/y`, `HH:mm`).
pub struct Region {
    /// "en-US"
    pub tag: String,
    pub group: String,
    pub decimal: String,
    pub short_date: String,
    pub short_time: String,
    pub long_date: String,
    pub long_time: String,
}

/// Reads the region settings: `Locale.getDefault()`, `DecimalFormatSymbols`,
/// `DateFormat.getBestDateTimePattern` and the 24-hour switch.
pub fn region() -> Option<Region> {
    with_activity(|env, activity| {
        let locale = env.call_static_method("java/util/Locale", "getDefault", "()Ljava/util/Locale;", &[])?.l()?;
        let tag = env.call_method(&locale, "toLanguageTag", "()Ljava/lang/String;", &[])?.l()?;
        let tag: String = env.get_string(&tag.into())?.into();
        let symbols = env.call_static_method("java/text/DecimalFormatSymbols", "getInstance", "(Ljava/util/Locale;)Ljava/text/DecimalFormatSymbols;", &[(&locale).into()])?.l()?;
        let mut separator = |name: &str| -> jni::errors::Result<String> {
            let c = env.call_method(&symbols, name, "()C", &[])?.c()?;
            Ok(char::from_u32(c.into()).map(String::from).unwrap_or_default())
        };
        let (group, decimal) = (separator("getGroupingSeparator")?, separator("getDecimalSeparator")?);
        let h24 = env.call_static_method("android/text/format/DateFormat", "is24HourFormat", "(Landroid/content/Context;)Z", &[activity.into()])?.z()?;
        let mut best = |skeleton: &str| -> jni::errors::Result<String> {
            let skeleton = env.new_string(skeleton)?;
            let pattern = env
                .call_static_method("android/text/format/DateFormat", "getBestDateTimePattern", "(Ljava/util/Locale;Ljava/lang/String;)Ljava/lang/String;", &[(&locale).into(), (&skeleton).into()])?
                .l()?;
            Ok(env.get_string(&pattern.into())?.into())
        };
        Ok(Region {
            tag,
            group,
            decimal,
            short_date: best("yMd")?,
            short_time: best(if h24 { "Hm" } else { "hm" })?,
            long_date: best("EEEEyMMMMd")?,
            long_time: best(if h24 { "Hms" } else { "hms" })?,
        })
    })
    .inspect_err(|e| log::warn!("region settings: {e}"))
    .ok()
}

/// The shared storage the user sees as "Internal storage", `/storage/emulated/0`.
pub fn shared_storage() -> Option<PathBuf> {
    with_activity(|env, _| {
        let dir = env.call_static_method("android/os/Environment", "getExternalStorageDirectory", "()Ljava/io/File;", &[])?.l()?;
        let path = env.call_method(&dir, "getAbsolutePath", "()Ljava/lang/String;", &[])?.l()?;
        Ok(PathBuf::from(String::from(env.get_string(&path.into())?)))
    })
    .inspect_err(|e| log::warn!("shared storage: {e}"))
    .ok()
}

/// Where Delete puts items (see `trash.rs`): `Trash` in the app's folder on
/// the shared storage, `Android/data/dev.macsplorer.app/files`.
pub fn trash_dir() -> Option<PathBuf> {
    Some(APP.get()?.external_data_path()?.join("Trash"))
}
