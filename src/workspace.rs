//! The folder the editor has open: listing what is in it and reading its files.
//!
//! Paths in a listing are relative to the workspace root and use `/` on every platform.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A listing stops here, so opening a huge folder cannot run away with memory.
pub const MAX_ENTRIES: usize = 100_000;
/// Files larger than this are not loaded into the editor.
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// A file or folder under the workspace root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub dir: bool,
    /// Git ignores it (or a folder above it).
    pub ignored: bool,
    /// A folder whose contents were not listed: an ignored folder (`node_modules`, `target`)
    /// or a link to a folder. The explorer lists it when it is first opened.
    pub unlisted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Every entry, parents before their children.
    pub entries: Vec<Entry>,
    /// The folder holds more than [`MAX_ENTRIES`]; the rest is left out.
    pub truncated: bool,
}

/// Which paths git ignores, from the paths `git status --ignored=matching` reports, made
/// relative to the workspace root.
#[derive(Clone, Debug, Default)]
pub struct Ignored {
    paths: HashSet<String>,
}

impl Ignored {
    /// `reported` paths are relative to the repository; `prefix` is the workspace's own path
    /// inside the repository (`""` when it is the repository's top level, else `sub/dir/`).
    pub fn new<'a>(reported: impl IntoIterator<Item = &'a str>, prefix: &str) -> Ignored {
        let paths = reported
            .into_iter()
            .filter_map(|p| p.trim_end_matches('/').strip_prefix(prefix))
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect();
        Ignored { paths }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
}

/// Lists everything under `root`. `.git` is left out. An ignored folder, and each folder inside
/// one, is listed without its contents unless it is in `open`: build output and dependencies
/// are large and rarely read, so they are listed a folder at a time as the explorer opens them.
pub fn list(root: &Path, ignored: &Ignored, open: &HashSet<String>) -> Listing {
    let mut out = Listing::default();
    // (folder, its path in the listing, whether it lies inside an ignored folder)
    let mut stack: Vec<(PathBuf, String, bool)> = vec![(root.to_path_buf(), String::new(), false)];
    while let Some((dir, rel, inside_ignored)) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else { continue };
        let mut children: Vec<(String, bool, bool)> = read
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name == ".git" {
                    return None;
                }
                let kind = e.file_type().ok()?;
                // A link to a folder shows as a folder, but is never followed while listing:
                // links can form loops.
                let link = kind.is_symlink();
                let is_dir = if link { fs::metadata(e.path()).is_ok_and(|m| m.is_dir()) } else { kind.is_dir() };
                Some((name, is_dir, link))
            })
            .collect();
        children.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, is_dir, link) in children {
            if out.entries.len() >= MAX_ENTRIES {
                out.truncated = true;
                return out;
            }
            let path = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let is_ignored = inside_ignored || ignored.contains(&path);
            let descend = is_dir && (open.contains(&path) || !(link || is_ignored));
            out.entries.push(Entry { path: path.clone(), dir: is_dir, ignored: is_ignored, unlisted: is_dir && !descend });
            if descend {
                stack.push((dir.join(&name), path, is_ignored));
            }
        }
    }
    // Depth-first order visits a folder's children after later siblings; sorting by path
    // components puts every parent directly before its children again.
    out.entries.sort_by(|a, b| a.path.split('/').cmp(b.path.split('/')));
    out
}

/// What a file holds, as far as the editor can show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// Not text: it has NUL bytes.
    Binary,
    TooLarge(u64),
    Missing,
    Error(String),
}

impl Content {
    pub fn from_bytes(bytes: Vec<u8>) -> Content {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Content::TooLarge(bytes.len() as u64);
        }
        if bytes.iter().take(8192).any(|&b| b == 0) {
            return Content::Binary;
        }
        let text = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        };
        Content::Text(text.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(text))
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Content::Text(t) => Some(t),
            _ => None,
        }
    }
}

pub fn read(path: &Path) -> Content {
    match fs::metadata(path) {
        Ok(m) if m.len() > MAX_FILE_BYTES => return Content::TooLarge(m.len()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Content::Missing,
        Err(e) => return Content::Error(e.to_string()),
    }
    match fs::read(path) {
        Ok(bytes) => Content::from_bytes(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Content::Missing,
        Err(e) => Content::Error(e.to_string()),
    }
}

/// `path` relative to `base` with `/` separators, or `None` when it is not under `base`.
pub fn relative(base: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn paths(l: &Listing) -> Vec<&str> {
        l.entries.iter().map(|e| e.path.as_str()).collect()
    }

    #[test]
    fn lists_parents_before_children_and_leaves_git_out() {
        let t = TempDir::new("list");
        t.write("src/main.rs", "fn main() {}");
        t.write("src/ui/view.rs", "");
        t.write("README.md", "# hi");
        t.write(".git/HEAD", "ref: refs/heads/main");
        fs::create_dir_all(t.0.join("empty")).expect("dir");
        let l = list(&t.0, &Ignored::default(), &HashSet::new());
        assert_eq!(paths(&l), ["README.md", "empty", "src", "src/main.rs", "src/ui", "src/ui/view.rs"]);
        assert!(l.entries.iter().find(|e| e.path == "empty").is_some_and(|e| e.dir && !e.unlisted));
        assert!(!l.truncated);
    }

    #[test]
    fn an_ignored_folder_is_listed_closed_until_it_is_opened() {
        let t = TempDir::new("ignored");
        t.write("src/main.rs", "");
        t.write("target/debug/code.exe", "");
        t.write(".env", "");
        let ignored = Ignored::new(["target/", ".env"], "");
        let l = list(&t.0, &ignored, &HashSet::new());
        assert_eq!(paths(&l), [".env", "src", "src/main.rs", "target"]);
        let target = l.entries.iter().find(|e| e.path == "target").expect("target");
        assert!(target.ignored && target.unlisted);
        assert!(l.entries.iter().find(|e| e.path == ".env").is_some_and(|e| e.ignored));
        assert!(l.entries.iter().find(|e| e.path == "src").is_some_and(|e| !e.ignored));

        // Opening it lists one level: the folders inside stay closed until they are opened too.
        let mut open = HashSet::from(["target".to_owned()]);
        let l = list(&t.0, &ignored, &open);
        assert_eq!(paths(&l), [".env", "src", "src/main.rs", "target", "target/debug"]);
        assert!(l.entries.iter().find(|e| e.path == "target").is_some_and(|e| e.ignored && !e.unlisted));
        assert!(l.entries.iter().find(|e| e.path == "target/debug").is_some_and(|e| e.ignored && e.unlisted));

        open.insert("target/debug".to_owned());
        let l = list(&t.0, &ignored, &open);
        assert_eq!(paths(&l), [".env", "src", "src/main.rs", "target", "target/debug", "target/debug/code.exe"]);
        assert!(l.entries.iter().filter(|e| e.path.starts_with("target")).all(|e| e.ignored && !e.unlisted));
    }

    #[test]
    fn ignored_paths_are_read_relative_to_a_workspace_inside_the_repository() {
        let ignored = Ignored::new(["app/dist/", "other/tmp/", "app/"], "app/");
        assert!(ignored.contains("dist"));
        assert!(!ignored.contains("tmp"));
        assert!(!ignored.contains(""));
    }

    #[test]
    fn tells_text_from_binary_and_drops_a_byte_order_mark() {
        assert_eq!(Content::from_bytes(b"hello\n".to_vec()), Content::Text("hello\n".into()));
        assert_eq!(Content::from_bytes(b"\xEF\xBB\xBFhello".to_vec()), Content::Text("hello".into()));
        assert_eq!(Content::from_bytes(vec![0x89, b'P', b'N', b'G', 0, 0]), Content::Binary);
        assert_eq!(Content::from_bytes(vec![b'a', 0xff, b'b']), Content::Text("a\u{fffd}b".into()));
        let t = TempDir::new("read");
        assert_eq!(read(&t.0.join("nope.txt")), Content::Missing);
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let base = Path::new("work").join("repo");
        assert_eq!(relative(&base, &base.join("src").join("main.rs")).as_deref(), Some("src/main.rs"));
        assert_eq!(relative(&base, Path::new("elsewhere")), None);
    }
}
