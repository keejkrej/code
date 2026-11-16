//! Git through the `git` command line: discovery, status, file contents at a revision, and the
//! few writes the Source Control view makes (stage, unstage, commit, init).
//!
//! Paths are relative to the repository's top level and use `/`, as git prints them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// What happened to a file on one side of the index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl Change {
    /// The status letter VS Code shows beside a file.
    pub fn letter(self) -> &'static str {
        match self {
            Change::Modified => "M",
            Change::Added => "A",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Copied => "C",
            Change::TypeChanged => "T",
            Change::Untracked => "U",
            Change::Conflicted => "!",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Change::Modified => "Modified",
            Change::Added => "Added",
            Change::Deleted => "Deleted",
            Change::Renamed => "Renamed",
            Change::Copied => "Copied",
            Change::TypeChanged => "Type changed",
            Change::Untracked => "Untracked",
            Change::Conflicted => "Conflicted",
        }
    }

    fn from_code(c: u8) -> Option<Change> {
        Some(match c {
            b'M' => Change::Modified,
            b'A' => Change::Added,
            b'D' => Change::Deleted,
            b'R' => Change::Renamed,
            b'C' => Change::Copied,
            b'T' => Change::TypeChanged,
            _ => return None,
        })
    }
}

/// One changed path: its change in the index against `HEAD` (`staged`) and in the working tree
/// against the index (`unstaged`). An untracked file is an unstaged [`Change::Untracked`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    /// Where a staged rename or copy came from.
    pub orig_path: Option<String>,
    pub staged: Option<Change>,
    pub unstaged: Option<Change>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The branch name, or `None` on a detached `HEAD`.
    pub branch: Option<String>,
    /// The commit `HEAD` is at, or `None` before the first commit.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileStatus>,
    /// Ignored paths as git reports them: a folder ends in `/` and stands for all it holds.
    pub ignored: Vec<String>,
}

impl Status {
    pub fn staged(&self) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(|f| f.staged.is_some())
    }

    pub fn unstaged(&self) -> impl Iterator<Item = &FileStatus> {
        self.files.iter().filter(|f| f.unstaged.is_some())
    }

    /// What the status bar shows: the branch, or a short commit id when detached.
    pub fn head_label(&self) -> String {
        match (&self.branch, &self.head) {
            (Some(b), _) => b.clone(),
            (None, Some(oid)) => oid.chars().take(7).collect(),
            (None, None) => "HEAD".into(),
        }
    }
}

/// Reads `git status --porcelain=v2 --branch -z` output.
pub fn parse_status(out: &[u8]) -> Status {
    let mut status = Status::default();
    let mut records = out.split(|&b| b == 0).filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        let text = String::from_utf8_lossy(rec);
        if let Some(header) = text.strip_prefix("# ") {
            let (name, value) = header.split_once(' ').unwrap_or((header, ""));
            match name {
                "branch.oid" => status.head = (value != "(initial)").then(|| value.to_owned()),
                "branch.head" => status.branch = (value != "(detached)").then(|| value.to_owned()),
                "branch.upstream" => status.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let (kind, rest) = text.split_once(' ').unwrap_or((&text, ""));
        match kind {
            // 1 XY sub mH mI mW hH hI path
            "1" => {
                let mut f = rest.splitn(8, ' ');
                let xy = f.next().unwrap_or("..").as_bytes().to_vec();
                if let Some(path) = f.nth(6) {
                    status.files.push(entry(&xy, path, None));
                }
            }
            // 2 XY sub mH mI mW hH hI Xscore path NUL origPath
            "2" => {
                let mut f = rest.splitn(9, ' ');
                let xy = f.next().unwrap_or("..").as_bytes().to_vec();
                let path = f.nth(7).map(str::to_owned);
                let orig = records.next().map(|o| String::from_utf8_lossy(o).into_owned());
                if let Some(path) = path {
                    status.files.push(entry(&xy, &path, orig));
                }
            }
            // u XY sub m1 m2 m3 mW h1 h2 h3 path
            "u" => {
                if let Some(path) = rest.splitn(10, ' ').nth(9) {
                    status.files.push(FileStatus {
                        path: path.to_owned(),
                        orig_path: None,
                        staged: None,
                        unstaged: Some(Change::Conflicted),
                    });
                }
            }
            "?" => {
                status.files.push(FileStatus { path: rest.to_owned(), orig_path: None, staged: None, unstaged: Some(Change::Untracked) })
            }
            "!" => status.ignored.push(rest.to_owned()),
            _ => {}
        }
    }
    status
}

fn entry(xy: &[u8], path: &str, orig_path: Option<String>) -> FileStatus {
    let staged = xy.first().copied().and_then(Change::from_code);
    let unstaged = xy.get(1).copied().and_then(Change::from_code);
    // The origin belongs to the staged side; a working-tree rename is never reported.
    let orig_path = orig_path.filter(|_| matches!(staged, Some(Change::Renamed | Change::Copied)));
    FileStatus { path: path.to_owned(), orig_path, staged, unstaged }
}

fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(["-c", "core.quotepath=off"]);
    // Reading status must not rewrite the index: a file watcher would see that as a change.
    cmd.env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0");
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up beside the editor's window.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn run(cmd: &mut Command) -> Result<Output, String> {
    let out = cmd.output().map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        return Ok(out);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    let said = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Err(match (err.is_empty(), said.is_empty()) {
        (false, _) => err,
        (true, false) => said,
        (true, true) => format!("git exited with {}", out.status),
    })
}

/// The top level of the repository `dir` is in, if it is in one (and git is installed).
pub fn discover(dir: &Path) -> Option<PathBuf> {
    let out = run(git(dir).args(["rev-parse", "--show-toplevel"])).ok()?;
    let top = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!top.is_empty()).then(|| PathBuf::from(top))
}

pub fn status(repo: &Path) -> Result<Status, String> {
    let out = run(git(repo).args(["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all", "--ignored=matching"]))?;
    Ok(parse_status(&out.stdout))
}

/// Which stored version of a file to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rev {
    Head,
    Index,
}

/// A file's bytes at `rev`. An error means it isn't there (or git failed).
pub fn show(repo: &Path, rev: Rev, path: &str) -> Result<Vec<u8>, String> {
    let spec = match rev {
        Rev::Head => format!("HEAD:{path}"),
        Rev::Index => format!(":{path}"),
    };
    Ok(run(git(repo).arg("show").arg(spec))?.stdout)
}

pub fn stage(repo: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    run(git(repo).args(["add", "-A", "--"]).args(paths)).map(drop)
}

pub fn stage_all(repo: &Path) -> Result<(), String> {
    run(git(repo).args(["add", "-A"])).map(drop)
}

pub fn unstage(repo: &Path, paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    if has_head(repo) {
        run(git(repo).args(["reset", "-q", "HEAD", "--"]).args(paths)).map(drop)
    } else {
        // Before the first commit there is no HEAD to reset to.
        run(git(repo).args(["rm", "-r", "-q", "--cached", "--"]).args(paths)).map(drop)
    }
}

fn has_head(repo: &Path) -> bool {
    run(git(repo).args(["rev-parse", "--verify", "-q", "HEAD"])).is_ok()
}

/// Commits what is staged.
pub fn commit(repo: &Path, message: &str) -> Result<(), String> {
    run(git(repo).args(["commit", "-q", "-m", message])).map(drop)
}

pub fn init(dir: &Path) -> Result<(), String> {
    run(git(dir).args(["init", "-q"])).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nul(parts: &[&str]) -> Vec<u8> {
        parts.iter().flat_map(|p| p.bytes().chain(std::iter::once(0))).collect()
    }

    #[test]
    fn reads_the_branch_and_how_far_it_is_from_its_upstream() {
        let s = parse_status(&nul(&[
            "# branch.oid 1234567890abcdef1234567890abcdef12345678",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -3",
        ]));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (2, 3));
        assert_eq!(s.head_label(), "main");
    }

    #[test]
    fn a_detached_head_shows_its_short_commit_and_a_new_repository_has_none() {
        let s = parse_status(&nul(&["# branch.oid 1234567890abcdef", "# branch.head (detached)"]));
        assert_eq!(s.branch, None);
        assert_eq!(s.head_label(), "1234567");
        let s = parse_status(&nul(&["# branch.oid (initial)", "# branch.head main"]));
        assert_eq!(s.head, None);
        assert_eq!(s.head_label(), "main");
    }

    #[test]
    fn splits_each_file_into_its_staged_and_unstaged_change() {
        let s = parse_status(&nul(&[
            "1 .M N... 100644 100644 100644 aaa bbb src/main.rs",
            "1 M. N... 100644 100644 100644 aaa bbb src/lib.rs",
            "1 MM N... 100644 100644 100644 aaa bbb a file with spaces.txt",
            "1 A. N... 000000 100644 100644 000 bbb new.txt",
            "1 .D N... 100644 100644 000000 aaa aaa gone.txt",
            "? notes/todo.md",
            "! target/",
            "! .env",
        ]));
        let by = |p: &str| s.files.iter().find(|f| f.path == p).unwrap_or_else(|| panic!("{p} missing"));
        assert_eq!((by("src/main.rs").staged, by("src/main.rs").unstaged), (None, Some(Change::Modified)));
        assert_eq!((by("src/lib.rs").staged, by("src/lib.rs").unstaged), (Some(Change::Modified), None));
        let both = by("a file with spaces.txt");
        assert_eq!((both.staged, both.unstaged), (Some(Change::Modified), Some(Change::Modified)));
        assert_eq!(by("new.txt").staged, Some(Change::Added));
        assert_eq!(by("gone.txt").unstaged, Some(Change::Deleted));
        assert_eq!(by("notes/todo.md").unstaged, Some(Change::Untracked));
        assert_eq!(s.ignored, ["target/", ".env"]);
        assert_eq!(s.staged().count(), 3);
        assert_eq!(s.unstaged().count(), 4);
    }

    #[test]
    fn a_rename_keeps_where_it_came_from_and_does_not_eat_the_next_record() {
        let s = parse_status(&nul(&["2 R. N... 100644 100644 100644 aaa aaa R100 src/new name.rs", "src/old name.rs", "? after.txt"]));
        assert_eq!(s.files.len(), 2);
        assert_eq!(s.files[0].path, "src/new name.rs");
        assert_eq!(s.files[0].orig_path.as_deref(), Some("src/old name.rs"));
        assert_eq!(s.files[0].staged, Some(Change::Renamed));
        assert_eq!(s.files[1].path, "after.txt");
    }

    #[test]
    fn an_unmerged_path_is_a_conflict() {
        let s = parse_status(&nul(&["u UU N... 100644 100644 100644 100644 aaa bbb ccc both changed.txt"]));
        assert_eq!(s.files[0].path, "both changed.txt");
        assert_eq!(s.files[0].unstaged, Some(Change::Conflicted));
        assert_eq!(s.files[0].staged, None);
    }
}
