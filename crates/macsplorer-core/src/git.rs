//! Git working tree information: a folder's branch and the status summary
//! the status bar shows. Not from Explorer, which has no version control;
//! the summary follows posh-git (`GitPrompt.ps1`, `GitUtils.ps1`, its
//! default settings) so it reads the same as in a PowerShell prompt:
//!
//! ```text
//! [main ≡ +0 ~2 -1 | +1 ~1 -0 !]
//! ```
//!
//! The branch name is read from the repository's files, which costs next to
//! nothing; the counts come from `git status`, so [`status`] belongs on a
//! worker thread.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::listing::is_git_repo;

/// The git to run, if this machine has one: found on `PATH` or in the usual
/// places (an app started from the Finder has a short `PATH`, without
/// Homebrew's). Without git the status summary and the Branch column are
/// left out. Looked up once.
pub fn executable() -> Option<&'static Path> {
    static GIT: OnceLock<Option<PathBuf>> = OnceLock::new();
    GIT.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let usual = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from);
        std::env::split_paths(&path).chain(usual).map(|dir| dir.join("git")).find(|git| git.is_file() && runs(git))
    })
    .as_deref()
}

/// False for the `/usr/bin/git` that macOS ships without the developer
/// tools: it doesn't run git, it offers to install them.
fn runs(git: &Path) -> bool {
    if cfg!(target_os = "macos") && git == Path::new("/usr/bin/git") {
        return Command::new("xcode-select").arg("-p").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    }
    true
}

/// The top of the working tree that holds `dir`, if any.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    dir.ancestors().find(|d| is_git_repo(d)).map(Path::to_path_buf)
}

/// The repository's own folder for the working tree at `repo`: `.git`, or
/// where a `.git` file points (linked worktrees and submodules).
fn git_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let text = fs::read_to_string(&dot_git).ok()?;
    let target = Path::new(text.lines().next()?.strip_prefix("gitdir:")?.trim());
    Some(repo.join(target))
}

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// What posh-git's `Get-GitBranch` shows for the working tree at `repo`: the
/// branch, or `(abc1234...)` for a detached HEAD (posh-git looks for a tag
/// first, this doesn't), then `|MERGING`, `|REBASE-i` and the like while one
/// of those is under way.
pub fn branch_label(repo: &Path) -> Option<String> {
    let dir = git_dir(repo)?;
    let mut state = String::new();
    let mut branch = None;
    if dir.join("rebase-merge").exists() {
        state = if dir.join("rebase-merge/interactive").exists() { "|REBASE-i" } else { "|REBASE-m" }.into();
        branch = read(&dir.join("rebase-merge/head-name"));
    } else {
        let mut steps = None;
        if dir.join("rebase-apply").exists() {
            steps = read(&dir.join("rebase-apply/next")).zip(read(&dir.join("rebase-apply/last")));
            state = if dir.join("rebase-apply/rebasing").exists() {
                "|REBASE"
            } else if dir.join("rebase-apply/applying").exists() {
                "|AM"
            } else {
                "|AM/REBASE"
            }
            .into();
        } else if dir.join("MERGE_HEAD").exists() {
            state = "|MERGING".into();
        } else if dir.join("CHERRY_PICK_HEAD").exists() {
            state = "|CHERRY-PICKING".into();
        } else if dir.join("REVERT_HEAD").exists() {
            state = "|REVERTING".into();
        } else if dir.join("BISECT_LOG").exists() {
            state = "|BISECTING".into();
        }
        if let Some((step, total)) = steps.filter(|(s, t)| !s.is_empty() && !t.is_empty()) {
            state.push_str(&format!(" {step}/{total}"));
        }
    }
    let branch = branch.filter(|b| !b.is_empty()).unwrap_or_else(|| match read(&dir.join("HEAD")) {
        Some(head) => match head.strip_prefix("ref: ") {
            Some(name) => name.to_owned(),
            None if head.len() >= 7 && head.is_char_boundary(7) => format!("({}...)", &head[..7]),
            None => "(unknown)".into(),
        },
        None => "(unknown)".into(),
    });
    Some(format!("{}{state}", branch.strip_prefix("refs/heads/").unwrap_or(&branch)))
}

/// Files by what happened to them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
    pub conflicted: u32,
}

impl Counts {
    fn any(self) -> bool {
        self.added + self.modified + self.deleted + self.conflicted > 0
    }
}

/// The branch against its remote counterpart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upstream {
    /// Tracks nothing.
    None,
    /// The tracked branch is gone from the remote.
    Gone,
    Tracking { ahead: u32, behind: u32 },
}

/// What `git status` says about a working tree, reduced to the summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// The top of the working tree.
    pub root: PathBuf,
    /// The branch as [`branch_label`] writes it.
    pub branch: String,
    pub upstream: Upstream,
    /// Staged changes.
    pub index: Counts,
    /// Unstaged changes; untracked files count as added.
    pub working: Counts,
}

/// What a piece of the summary is colored by, the way posh-git colors them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// The brackets and the `|` between index and working directory (yellow).
    Delimiter,
    /// The branch, in sync with its remote or tracking none; also the `~`
    /// that says changes are staged (cyan).
    Branch,
    /// Ahead of the remote (green).
    Ahead,
    /// Behind the remote (red).
    Behind,
    /// Both ahead and behind (yellow).
    Diverged,
    /// Staged changes (dark green).
    Index,
    /// Unstaged changes, and the `!` that says there are some (dark red).
    Working,
}

/// A piece of the summary text and how it is colored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub tone: Tone,
}

impl Status {
    /// Reads the counts and branch comparison from `git status --porcelain
    /// --branch` output, counting files as posh-git's `Get-GitStatus` does,
    /// except that a type change counts as modified there too.
    pub fn parse(root: PathBuf, branch: String, output: &str) -> Status {
        let mut status = Status { root, branch, upstream: Upstream::None, index: Counts::default(), working: Counts::default() };
        for line in output.lines() {
            if let Some(header) = line.strip_prefix("## ") {
                status.upstream = parse_upstream(header);
                continue;
            }
            let mut xy = line.chars();
            let (Some(x), Some(y)) = (xy.next(), xy.next()) else { continue };
            match x {
                'A' => status.index.added += 1,
                'M' | 'R' | 'C' | 'T' => status.index.modified += 1,
                'D' => status.index.deleted += 1,
                'U' => status.index.conflicted += 1,
                _ => {}
            }
            match y {
                '?' | 'A' => status.working.added += 1,
                'M' | 'T' => status.working.modified += 1,
                'D' => status.working.deleted += 1,
                'U' => status.working.conflicted += 1,
                _ => {}
            }
        }
        status
    }

    /// `Write-GitStatus` with posh-git's default settings.
    pub fn segments(&self) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut push = |text: String, tone| out.push(Segment { text, tone });
        push("[".into(), Tone::Delimiter);
        let (tone, remote) = match self.upstream {
            Upstream::None => (Tone::Branch, String::new()),
            Upstream::Gone => (Tone::Branch, " ×".into()),
            Upstream::Tracking { ahead: 0, behind: 0 } => (Tone::Branch, " ≡".into()),
            Upstream::Tracking { ahead, behind: 0 } => (Tone::Ahead, format!(" ↑{ahead}")),
            Upstream::Tracking { ahead: 0, behind } => (Tone::Behind, format!(" ↓{behind}")),
            Upstream::Tracking { ahead, behind } => (Tone::Diverged, format!(" ↓{behind} ↑{ahead}")),
        };
        push(self.branch.clone(), tone);
        if !remote.is_empty() {
            push(remote, tone);
        }
        let (has_index, has_working) = (self.index.any(), self.working.any());
        if has_index {
            push(group(self.index), Tone::Index);
            if has_working {
                push(" |".into(), Tone::Delimiter);
            }
        }
        if has_working {
            push(group(self.working), Tone::Working);
            push(" !".into(), Tone::Working);
        } else if has_index {
            push(" ~".into(), Tone::Branch);
        }
        push("]".into(), Tone::Delimiter);
        out
    }

    /// The summary as plain text.
    pub fn summary(&self) -> String {
        self.segments().into_iter().map(|s| s.text).collect()
    }
}

/// ` +A ~B -C`, and ` !D` if anything is conflicted: posh-git shows zeros
/// (`ShowStatusWhenZero`) except for conflicts.
fn group(c: Counts) -> String {
    let conflicts = if c.conflicted > 0 { format!(" !{}", c.conflicted) } else { String::new() };
    format!(" +{} ~{} -{}{conflicts}", c.added, c.modified, c.deleted)
}

/// The `## main...origin/main [ahead 1, behind 2]` line, after its `## `.
fn parse_upstream(header: &str) -> Upstream {
    let (names, bracket) = match header.rfind(" [").filter(|_| header.ends_with(']')) {
        Some(i) => (&header[..i], Some(&header[i + 2..header.len() - 1])),
        None => (header, None),
    };
    if !names.contains("...") {
        return Upstream::None;
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in bracket.into_iter().flat_map(|b| b.split(", ")) {
        if part == "gone" {
            return Upstream::Gone;
        }
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    Upstream::Tracking { ahead, behind }
}

/// Runs `git status` for the working tree that holds `dir`. `None` outside
/// one, or if git can't be run.
pub fn status(dir: &Path) -> Option<Status> {
    let git = executable()?;
    let root = repo_root(dir)?;
    let output = Command::new(git)
        .args(["--no-optional-locks", "-c", "core.quotepath=false", "-c", "color.status=false"])
        .args(["status", "--porcelain", "--branch", "-unormal"])
        .current_dir(&root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let branch = branch_label(&root)?;
    Some(Status::parse(root, branch, &String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(branch: &str, output: &str) -> Status {
        Status::parse("/repo".into(), branch.into(), output)
    }

    #[test]
    fn summary_matches_posh_git() {
        // 2 modified and 1 deleted staged; 1 untracked, 1 modified not staged.
        let out = "## main...origin/main\nM  a\nM  b\nD  c\n M d\n?? new\n";
        assert_eq!(parse("main", out).summary(), "[main ≡ +0 ~2 -1 | +1 ~1 -0 !]");
    }

    #[test]
    fn clean_tree_and_staged_only() {
        assert_eq!(parse("main", "## main...origin/main\n").summary(), "[main ≡]");
        assert_eq!(parse("main", "## main...origin/main\nA  x\n").summary(), "[main ≡ +1 ~0 -0 ~]");
        assert_eq!(parse("main", "## main...origin/main\n D x\n").summary(), "[main ≡ +0 ~0 -1 !]");
    }

    #[test]
    fn remote_comparison() {
        let s = |h: &str| parse("dev", &format!("## {h}\n")).summary();
        assert_eq!(s("dev"), "[dev]");
        assert_eq!(s("dev...origin/dev [ahead 2]"), "[dev ↑2]");
        assert_eq!(s("dev...origin/dev [behind 3]"), "[dev ↓3]");
        assert_eq!(s("dev...origin/dev [ahead 2, behind 3]"), "[dev ↓3 ↑2]");
        assert_eq!(s("dev...origin/dev [gone]"), "[dev ×]");
        assert_eq!(s("No commits yet on dev"), "[dev]");
    }

    #[test]
    fn conflicts_count_on_both_sides() {
        let st = parse("main", "## main\nUU x\nAA y\n");
        assert_eq!(st.index, Counts { added: 1, modified: 0, deleted: 0, conflicted: 1 });
        assert_eq!(st.working, Counts { added: 1, modified: 0, deleted: 0, conflicted: 1 });
        assert_eq!(st.summary(), "[main +1 ~0 -0 !1 | +1 ~0 -0 !1 !]");
    }

    #[test]
    fn renames_and_type_changes_are_modified_and_ignored_files_are_not_counted() {
        let st = parse("main", "R  old -> new\n T link\n!! target/\n");
        assert_eq!(st.index.modified, 1);
        assert_eq!(st.working.modified, 1);
        assert_eq!(st.working, Counts { modified: 1, ..Counts::default() });
    }

    #[test]
    fn tones_follow_the_remote() {
        let tone = |h: &str| parse("b", &format!("## {h}\n")).segments()[1].tone;
        assert_eq!(tone("b...o/b"), Tone::Branch);
        assert_eq!(tone("b...o/b [ahead 1]"), Tone::Ahead);
        assert_eq!(tone("b...o/b [behind 1]"), Tone::Behind);
        assert_eq!(tone("b...o/b [ahead 1, behind 1]"), Tone::Diverged);
    }

    #[test]
    fn branch_names_from_the_repository_files() {
        let dir = std::env::temp_dir().join(format!("macsplorer-branch-{}", std::process::id()));
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/feature/x\n").unwrap();
        assert_eq!(branch_label(&dir).as_deref(), Some("feature/x"));
        fs::write(git.join("MERGE_HEAD"), "abc\n").unwrap();
        assert_eq!(branch_label(&dir).as_deref(), Some("feature/x|MERGING"));
        fs::remove_file(git.join("MERGE_HEAD")).unwrap();
        fs::write(git.join("HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        assert_eq!(branch_label(&dir).as_deref(), Some("(0123456...)"));
        // A worktree's .git file points at its own git dir.
        let linked = dir.join("linked");
        let target = git.join("worktrees/linked");
        fs::create_dir_all(&linked).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(linked.join(".git"), format!("gitdir: {}\n", target.display())).unwrap();
        fs::write(target.join("HEAD"), "ref: refs/heads/other\n").unwrap();
        assert_eq!(branch_label(&linked).as_deref(), Some("other"));
        assert_eq!(repo_root(&linked.join("sub")), Some(linked.clone()));
        fs::remove_dir_all(&dir).unwrap();
    }
}
