//! UI-independent file browser logic for macsplorer.
//!
//! Behaviors marked as Explorer-derived are documented, with their evidence,
//! in `docs/file-browser-findings.md`.

pub mod associations;
pub mod autosuggest;
pub mod bplist;
pub mod breadcrumb;
pub mod columns;
pub mod copy_path;
pub mod copy_progress;
pub mod dirscan;
pub mod dragdrop;
pub mod fileops;
pub mod focus;
pub mod folder_style;
pub mod fonts;
pub mod git;
pub mod highlight;
pub mod hls;
pub mod history;
pub mod isearch;
pub mod layout;
pub mod listing;
pub mod marquee;
pub mod menu_delay;
pub mod natural;
pub mod places;
pub mod preferences;
pub mod properties;
pub mod rename;
pub mod seti;
pub mod slow_click;
pub mod text_size;
pub mod tree;
pub mod undo;
pub mod watch;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod xattr;
