//! The few things that differ per OS: opening and revealing files, ejecting
//! volumes, wording, and where fonts live. macOS uses `open`/`diskutil`,
//! Linux the freedesktop tools.

use std::io;
use std::path::Path;
use std::process::Command;

use eframe::egui;

/// Name of the modifier shown in tooltips (egui's `command` is Cmd on macOS,
/// Ctrl elsewhere).
pub const CMD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

/// The New Window shortcut as menus print it. Explorer's is Win+E, so Linux
/// uses the Super key (egui has no modifier flag for it: see `Explorer::keyboard`).
pub const NEW_WINDOW_KEYS: &str = if cfg!(target_os = "macos") { "⌘\u{2009}E" } else { "Super+E" };

/// The Go To Folder shortcut as menus print it: Finder's Shift+Cmd+G.
pub fn go_to_keys() -> String {
    if cfg!(target_os = "macos") { "⇧\u{2009}⌘\u{2009}G".to_owned() } else { format!("{CMD}Shift+G") }
}

/// A keyboard shortcut as menus print it: `⌥⌘C` on macOS, `Ctrl+Alt+C` elsewhere.
/// `key` is the key's own label (`"C"`, `"⌫"`); `alt` adds Option (Alt).
pub fn shortcut(alt: bool, key: &str) -> String {
    match (cfg!(target_os = "macos"), alt) {
        (true, true) => format!("⌥\u{2009}{CMD}\u{2009}{key}"),
        (_, true) => format!("{CMD}Alt+{key}"),
        (true, false) => format!("{CMD}\u{2009}{key}"),
        _ => format!("{CMD}{key}"),
    }
}

/// Label of the context menu entry that reveals an item in the file manager.
pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") { "Show in Finder" } else { "Show in File Manager" };

/// Where the system keeps Tahoma, regular then bold. The first path that
/// exists wins. Tahoma isn't bundled; elsewhere egui's default font is used.
#[cfg(target_os = "macos")]
pub const TAHOMA: &[&str] = &["/System/Library/Fonts/Supplemental/Tahoma.ttf"];
#[cfg(target_os = "macos")]
pub const TAHOMA_BOLD: &[&str] = &["/System/Library/Fonts/Supplemental/Tahoma Bold.ttf"];

/// Fallback for the Mac key symbols menus print (⌥ and ⌫ are in neither Tahoma
/// nor egui's fonts). Linux menus spell the keys out.
#[cfg(target_os = "macos")]
pub const SYMBOL_FONT: &[&str] = &["/System/Library/Fonts/Apple Symbols.ttf"];
#[cfg(not(target_os = "macos"))]
pub const SYMBOL_FONT: &[&str] = &[];

/// Linux has no Tahoma of its own: use one the user installed (fonts folder,
/// Wine, a Windows partition).
#[cfg(target_os = "linux")]
pub const TAHOMA: &[&str] = &[
    "/usr/share/fonts/truetype/tahoma/tahoma.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/Tahoma.ttf",
    "/usr/share/fonts/TTF/tahoma.ttf",
    "/usr/share/fonts/tahoma.ttf",
    "/usr/share/wine/fonts/tahoma.ttf",
    "/usr/lib/wine/fonts/tahoma.ttf",
];
#[cfg(target_os = "linux")]
pub const TAHOMA_BOLD: &[&str] = &[
    "/usr/share/fonts/truetype/tahoma/tahomabd.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/Tahoma_Bold.ttf",
    "/usr/share/fonts/TTF/tahomabd.ttf",
    "/usr/share/fonts/tahomabd.ttf",
    "/usr/share/wine/fonts/tahomabd.ttf",
    "/usr/lib/wine/fonts/tahomabd.ttf",
];

/// DejaVu Sans Condensed, the UI font when the system has no Tahoma. Built into
/// the Linux binary only; macOS ships Tahoma (`assets/fonts`, license beside it).
#[cfg(target_os = "linux")]
pub fn bundled_ui_font(bold: bool) -> &'static [u8] {
    if bold {
        include_bytes!("../../../assets/fonts/DejaVuSansCondensed-Bold.ttf")
    } else {
        include_bytes!("../../../assets/fonts/DejaVuSansCondensed.ttf")
    }
}

/// `errno` 1, "Operation not permitted".
const EPERM: i32 = 1;

/// The message for a folder that can't be listed. On macOS "Operation not
/// permitted" (EPERM) is the privacy protection (TCC) that guards the Trash,
/// Mail and similar folders; only the user can lift it, in System Settings.
pub fn open_error(dir: &Path, e: &io::Error) -> String {
    let mut msg = format!("Can't open {}: {e}", dir.display());
    if cfg!(target_os = "macos") && e.raw_os_error() == Some(EPERM) {
        msg.push_str(
            "\n\nmacOS protects this folder. To open it, give Macsplorer (or the terminal it was started from) \
             access in System Settings > Privacy & Security > Full Disk Access, then restart it.",
        );
    }
    msg
}

/// Opens `path` in its default application without waiting for it.
pub fn open_default(path: &Path) -> io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(opener).arg(path).spawn().map(drop)
}

/// Opens a web address in the default browser without waiting for it.
pub fn open_url(url: &str) -> io::Result<()> {
    open_default(Path::new(url))
}

/// Opens a text file in the default text editor without waiting for it. On
/// macOS `open -t` is the editor for plain text, whatever app owns `.json`.
pub fn open_text(path: &Path) -> io::Result<()> {
    if cfg!(target_os = "macos") {
        Command::new("open").arg("-t").arg(path).spawn().map(drop)
    } else {
        open_default(path)
    }
}

/// Environment variable carrying the opener's window frame (`x,y,w,h`, points)
/// to a new window's process, so the new window can open offset from it.
const WINDOW_ENV: &str = "MACSPLORER_WINDOW";

/// How far a new window is offset from the one that opened it.
const CASCADE: f32 = 24.0;

/// Opens another Macsplorer window on `folder`. Each window is its own
/// process, so they share nothing (clipboard, undo, settings). `frame` is the
/// opener's window rect, which the new window cascades from.
pub fn new_window(folder: &Path, frame: Option<egui::Rect>) -> io::Result<()> {
    let frame = frame.map(|r| format!("{},{},{},{}", r.left() + CASCADE, r.top() + CASCADE, r.width(), r.height()));
    let exe = std::env::current_exe()?;
    // Launched through Launch Services when bundled, so the new window comes
    // to the front like any app opening; a bare binary is just started.
    #[cfg(target_os = "macos")]
    if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) {
        let mut open = Command::new("open");
        open.arg("-n").arg("-a").arg(bundle);
        if let Some(frame) = &frame {
            open.arg("--env").arg(format!("{WINDOW_ENV}={frame}"));
        }
        return open.arg("--args").arg(folder).spawn().map(drop);
    }
    let mut cmd = Command::new(exe);
    cmd.arg(folder);
    if let Some(frame) = frame {
        cmd.env(WINDOW_ENV, frame);
    }
    cmd.spawn().map(drop)
}

/// The frame a window opened by [`new_window`] should take: position and size.
pub fn inherited_frame() -> Option<([f32; 2], [f32; 2])> {
    parse_frame(&std::env::var(WINDOW_ENV).ok()?)
}

fn parse_frame(text: &str) -> Option<([f32; 2], [f32; 2])> {
    let n: Vec<f32> = text.split(',').map(|v| v.trim().parse().ok()).collect::<Option<_>>()?;
    match n[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Some(([x, y], [w, h])),
        _ => None,
    }
}

/// Shows `path` selected in the file manager. Blocks briefly; call it off
/// the UI thread.
#[cfg_attr(not(unix), allow(unused_variables))]
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).status();
    }
    #[cfg(target_os = "linux")]
    {
        // The freedesktop FileManager1 interface selects the item; without
        // it (or without dbus-send), open the containing folder instead.
        let uri = format!("file://{}", percent_encode(&path.to_string_lossy()));
        let shown = Command::new("dbus-send")
            .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "--type=method_call"])
            .args(["/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
            .arg(format!("array:string:{uri}"))
            .arg("string:")
            .output()
            .is_ok_and(|o| o.status.success());
        if !shown {
            let _ = Command::new("xdg-open").arg(path.parent().unwrap_or(path)).status();
        }
    }
}

/// Opens a terminal window in `folder` without waiting for it. `preferred` is
/// the user's choice from the settings file: on macOS an app name, which gets
/// the folder like Finder's drop on the Dock icon (Terminal, iTerm and Ghostty
/// open a window there); on Linux a command. The error is what to tell the user.
#[cfg(unix)]
pub fn open_terminal(folder: &Path, preferred: Option<&str>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // macOS has no default terminal setting to read. `open` returns once
        // the launch is requested, so waiting for it is quick, and it is
        // where a misspelled app name shows up.
        let out = Command::new("open").arg("-a").arg(preferred.unwrap_or("Terminal")).arg(folder).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_owned()) }
    }
    #[cfg(target_os = "linux")]
    {
        // No single terminal on Linux: the settings file's, $TERMINAL, then the
        // Debian alternative, then the common ones. Most start in the
        // working directory they inherit; gnome-terminal's server needs the
        // folder as an argument.
        let env = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
        let known = ["x-terminal-emulator", "gnome-terminal", "konsole", "xfce4-terminal", "alacritty", "kitty", "xterm"];
        for terminal in preferred.into_iter().chain(env.as_deref()).chain(known) {
            let mut cmd = Command::new(terminal);
            cmd.current_dir(folder);
            if terminal.ends_with("gnome-terminal") {
                cmd.arg(format!("--working-directory={}", folder.display()));
            }
            match cmd.spawn() {
                Ok(_) => return Ok(()),
                // Not installed: try the next one.
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("no terminal found; set $TERMINAL".into())
    }
}

/// Unmounts and ejects a mounted volume. Blocks; call it off the UI thread.
/// The error is the tool's own message.
#[cfg(unix)]
pub fn eject(volume: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let attempts: [(&str, &[&str]); 1] = [("diskutil", &["eject"])];
    // gio goes through the desktop's mount service (udisks), so it works for
    // removable drives without root; umount covers a plain mount.
    #[cfg(target_os = "linux")]
    let attempts: [(&str, &[&str]); 2] = [("gio", &["mount", "--eject"]), ("umount", &[])];

    let mut last = String::from("no tool available to eject");
    for (tool, args) in attempts {
        match Command::new(tool).args(args).arg(volume).output() {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => last = String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            // The tool isn't installed: try the next one.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// Percent-encodes a path for a `file://` URI, keeping `/`.
#[cfg(any(target_os = "linux", test))]
fn percent_encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_text_is_parsed() {
        assert_eq!(parse_frame("100,50.5,1000,680"), Some(([100.0, 50.5], [1000.0, 680.0])));
        assert_eq!(parse_frame("1,2,3"), None);
        assert_eq!(parse_frame("1,2,0,4"), None);
        assert_eq!(parse_frame("a,b,c,d"), None);
    }

    #[test]
    fn uri_paths_are_percent_encoded() {
        assert_eq!(percent_encode("/home/me/My Files/a#b.txt"), "/home/me/My%20Files/a%23b.txt");
        assert_eq!(percent_encode("/tmp/ü"), "/tmp/%C3%BC");
    }
}
