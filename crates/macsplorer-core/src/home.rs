//! The home folder as `~`: how a path is shown and how a typed one is read.
//!
//! Not from Explorer (it has no `~`): this follows what shells and terminal
//! file managers do. Only a lone `~` and `~/…` mean the home folder. `~name`
//! (another user's home) is left as typed, as the address bar can't tell
//! which folder it should be.

use std::path::{Path, PathBuf};

/// Where macOS shows the data volume's folders a second time (a firmlink), so
/// the same home folder can arrive as `/System/Volumes/Data/Users/me`.
const FIRMLINK: &str = "/System/Volumes/Data";

/// The user's home folder and the other spellings of its path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Home {
    path: PathBuf,
    /// The path with its symlinks resolved, when that is another one (the
    /// home folder is on another volume, or behind a link).
    real: Option<PathBuf>,
}

impl Home {
    /// Looks the real path up on disk, so call it once, not per frame.
    pub fn new(path: PathBuf) -> Self {
        let real = path.canonicalize().ok().filter(|real| *real != path);
        Self { path, real }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What `path` is below the home folder, when it is in it (empty for the
    /// home folder itself), whichever way the path is spelled.
    pub fn within<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        let bases = || std::iter::once(&self.path).chain(&self.real);
        bases().find_map(|base| path.strip_prefix(base).ok()).or_else(|| {
            // Left of the firmlink the path is relative, so are the bases.
            let data = path.strip_prefix(FIRMLINK).ok()?;
            bases().find_map(|base| data.strip_prefix(base.strip_prefix("/").ok()?).ok())
        })
    }
}

/// How a path is shown: inside the home folder as `~` or `~/Documents`, any
/// other path as it is.
pub fn abbreviate(path: &Path, home: Option<&Home>) -> String {
    match home.and_then(|h| h.within(path)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The path a typed address means: `~` and `~/…` start at the home folder,
/// everything else (`~name` too) is taken as typed.
pub fn expand(input: &str, home: Option<&Home>) -> PathBuf {
    let rest = if input == "~" { Some("") } else { input.strip_prefix("~/") };
    match (rest, home) {
        (Some(rest), Some(home)) => home.path.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> Home {
        Home { path: PathBuf::from("/Users/me"), real: None }
    }

    #[test]
    fn the_home_folder_is_a_tilde() {
        let home = me();
        assert_eq!(abbreviate(Path::new("/Users/me"), Some(&home)), "~");
        assert_eq!(abbreviate(Path::new("/Users/me/Work"), Some(&home)), "~/Work");
        assert_eq!(abbreviate(Path::new("/Users/meg/Work"), Some(&home)), "/Users/meg/Work");
        assert_eq!(abbreviate(Path::new("/Volumes/Data"), Some(&home)), "/Volumes/Data");
        assert_eq!(abbreviate(Path::new("/Users/me/Work"), None), "/Users/me/Work");
    }

    #[test]
    fn other_spellings_of_the_home_folder_count() {
        let home = Home { path: PathBuf::from("/Users/me"), real: Some(PathBuf::from("/Volumes/Big/me")) };
        assert_eq!(abbreviate(Path::new("/System/Volumes/Data/Users/me/Work"), Some(&home)), "~/Work");
        assert_eq!(abbreviate(Path::new("/Volumes/Big/me"), Some(&home)), "~");
        assert_eq!(abbreviate(Path::new("/Volumes/Big/me/Work/x"), Some(&home)), "~/Work/x");
        assert_eq!(abbreviate(Path::new("/Volumes/Big/meg"), Some(&home)), "/Volumes/Big/meg");
    }

    #[test]
    fn a_tilde_starts_at_the_home_folder() {
        let home = me();
        assert_eq!(expand("~", Some(&home)), PathBuf::from("/Users/me/"));
        assert_eq!(expand("~/", Some(&home)), PathBuf::from("/Users/me/"));
        assert_eq!(expand("~/Documents", Some(&home)), PathBuf::from("/Users/me/Documents"));
        assert_eq!(expand("~//Documents", Some(&home)), PathBuf::from("/Users/me/Documents"));
    }

    #[test]
    fn another_users_tilde_is_left_alone() {
        let home = me();
        assert_eq!(expand("~jan", Some(&home)), PathBuf::from("~jan"));
        assert_eq!(expand("~jan/Documents", Some(&home)), PathBuf::from("~jan/Documents"));
        assert_eq!(expand("~", None), PathBuf::from("~"));
        assert_eq!(expand("/Users/me", Some(&home)), PathBuf::from("/Users/me"));
    }

    #[test]
    fn the_real_path_is_only_kept_when_it_differs() {
        let dir = std::env::temp_dir();
        assert_eq!(Home::new(dir.canonicalize().unwrap()).real, None);
        assert_eq!(Home::new(PathBuf::from("/no/such/folder")).real, None);
    }
}
