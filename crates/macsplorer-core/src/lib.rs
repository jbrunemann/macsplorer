//! UI-independent file browser logic for macsplorer.
//!
//! Behaviors marked as Explorer-derived are documented, with their evidence,
//! in `docs/file-browser-findings.md`.

pub mod breadcrumb;
pub mod columns;
pub mod copy_path;
pub mod dirscan;
pub mod fileops;
pub mod highlight;
pub mod history;
pub mod isearch;
pub mod layout;
pub mod listing;
pub mod marquee;
pub mod natural;
pub mod places;
pub mod preferences;
pub mod properties;
pub mod rename;
pub mod seti;
pub mod slow_click;
pub mod tree;
pub mod undo;
