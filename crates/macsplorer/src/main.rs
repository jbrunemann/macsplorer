// No console window next to the app on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> eframe::Result {
    macsplorer::run()
}
