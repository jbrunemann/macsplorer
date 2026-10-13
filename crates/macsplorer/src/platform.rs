//! The few things that differ per OS: opening and revealing files, ejecting
//! volumes, wording, and where fonts live. macOS uses `open`/`diskutil`;
//! everything else is treated as a freedesktop Linux desktop.

use std::io;
use std::path::Path;
use std::process::Command;

/// Name of the modifier shown in tooltips (egui's `command` is Cmd on macOS,
/// Ctrl elsewhere).
pub const CMD: &str = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" };

/// Label of the context menu entry that reveals an item in the file manager.
pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") { "Show in Finder" } else { "Show in File Manager" };

/// Where the system keeps Tahoma, regular then bold. The first path that
/// exists wins. Tahoma isn't bundled; elsewhere egui's default font is used.
#[cfg(target_os = "macos")]
pub const TAHOMA: &[&str] = &["/System/Library/Fonts/Supplemental/Tahoma.ttf"];
#[cfg(target_os = "macos")]
pub const TAHOMA_BOLD: &[&str] = &["/System/Library/Fonts/Supplemental/Tahoma Bold.ttf"];

/// Linux has no Tahoma of its own: use one the user installed (fonts folder,
/// Wine, a Windows partition).
#[cfg(not(target_os = "macos"))]
pub const TAHOMA: &[&str] = &[
    "/usr/share/fonts/truetype/tahoma/tahoma.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/Tahoma.ttf",
    "/usr/share/fonts/TTF/tahoma.ttf",
    "/usr/share/fonts/tahoma.ttf",
    "/usr/share/wine/fonts/tahoma.ttf",
    "/usr/lib/wine/fonts/tahoma.ttf",
];
#[cfg(not(target_os = "macos"))]
pub const TAHOMA_BOLD: &[&str] = &[
    "/usr/share/fonts/truetype/tahoma/tahomabd.ttf",
    "/usr/share/fonts/truetype/msttcorefonts/Tahoma_Bold.ttf",
    "/usr/share/fonts/TTF/tahomabd.ttf",
    "/usr/share/fonts/tahomabd.ttf",
    "/usr/share/wine/fonts/tahomabd.ttf",
    "/usr/lib/wine/fonts/tahomabd.ttf",
];

/// Opens `path` in its default application without waiting for it.
pub fn open_default(path: &Path) -> io::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(opener).arg(path).spawn().map(drop)
}

/// Shows `path` selected in the file manager. Blocks briefly; call it off
/// the UI thread.
pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).status();
    }
    #[cfg(not(target_os = "macos"))]
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

/// Unmounts and ejects a mounted volume. Blocks; call it off the UI thread.
/// The error is the tool's own message.
pub fn eject(volume: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let attempts: [(&str, &[&str]); 1] = [("diskutil", &["eject"])];
    // gio goes through the desktop's mount service (udisks), so it works for
    // removable drives without root; umount covers a plain mount.
    #[cfg(not(target_os = "macos"))]
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
#[cfg(any(not(target_os = "macos"), test))]
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
    fn uri_paths_are_percent_encoded() {
        assert_eq!(percent_encode("/home/me/My Files/a#b.txt"), "/home/me/My%20Files/a%23b.txt");
        assert_eq!(percent_encode("/tmp/ü"), "/tmp/%C3%BC");
    }
}
