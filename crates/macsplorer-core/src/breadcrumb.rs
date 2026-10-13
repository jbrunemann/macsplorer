//! Breadcrumb segments for the address bar (`CBreadcrumbBar`).

use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub path: PathBuf,
}

/// Splits a path into clickable segments, root first.
///
/// Paths inside the home folder start at the home folder, the way Explorer
/// roots user folders under the user's name instead of `C:\Users`.
pub fn crumbs(path: &Path, home: Option<&Path>) -> Vec<Crumb> {
    let (mut acc, rest) = match home.and_then(|h| path.strip_prefix(h).ok().map(|r| (h, r))) {
        Some((h, rest)) => {
            let label = h.file_name().map_or("Home".into(), |n| n.to_string_lossy().into_owned());
            (vec![Crumb { label, path: h.to_path_buf() }], rest)
        }
        None => (Vec::new(), path),
    };
    let mut current = acc.last().map_or_else(PathBuf::new, |c| c.path.clone());
    for comp in rest.components() {
        current.push(comp);
        let label = match comp {
            Component::RootDir => "/".to_string(),
            Component::Normal(n) => n.to_string_lossy().into_owned(),
            _ => continue,
        };
        acc.push(Crumb { label, path: current.clone() });
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(v: &[Crumb]) -> Vec<&str> {
        v.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn absolute_path() {
        let c = crumbs(Path::new("/Volumes/Data/x"), None);
        assert_eq!(labels(&c), ["/", "Volumes", "Data", "x"]);
        assert_eq!(c[2].path, PathBuf::from("/Volumes/Data"));
    }

    #[test]
    fn home_relative() {
        let home = Path::new("/Users/me");
        let c = crumbs(Path::new("/Users/me/Documents/a"), Some(home));
        assert_eq!(labels(&c), ["me", "Documents", "a"]);
        assert_eq!(c[1].path, PathBuf::from("/Users/me/Documents"));
    }

    #[test]
    fn home_itself() {
        let home = Path::new("/Users/me");
        assert_eq!(labels(&crumbs(home, Some(home))), ["me"]);
    }
}
