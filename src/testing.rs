//! Folders and repositories for tests, made under the system's temporary folder.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// A fresh folder, removed when dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("code-test-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        TempDir(dir)
    }

    pub fn write(&self, path: &str, text: &str) {
        let p = self.0.join(path);
        fs::create_dir_all(p.parent().expect("parent")).expect("dirs");
        fs::write(p, text).expect("write");
    }

    pub fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git").arg("-C").arg(&self.0).args(args).output().expect("git runs");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A repository with one commit, then a change of each kind on top of it:
    ///
    /// - `src/main.rs` modified, not staged
    /// - `web/fetch.ts` modified and staged
    /// - `notes.md` untracked
    /// - `target/` and `.env` ignored
    pub fn repo(name: &str) -> TempDir {
        let t = TempDir::new(name);
        t.git(&["init", "-q", "-b", "main"]);
        t.git(&["config", "user.name", "Test"]);
        t.git(&["config", "user.email", "test@example.com"]);
        t.git(&["config", "core.autocrlf", "false"]);
        t.git(&["config", "commit.gpgsign", "false"]);
        t.write("src/main.rs", "fn main() {\n    let wait = 1;\n    println!(\"{wait}\");\n}\n");
        t.write("src/net/retry.rs", "pub const TRIES: u32 = 3;\n");
        t.write("web/fetch.ts", "export const tries = 3\n");
        t.write("README.md", "# fixture\n");
        t.write(".gitignore", "/target\n.env\n");
        t.git(&["add", "-A"]);
        t.git(&["commit", "-q", "-m", "Initial commit"]);
        t.write("src/main.rs", "fn main() {\n    let wait = 2;\n    println!(\"{wait}\");\n}\n");
        t.write("web/fetch.ts", "export const tries = 5\n");
        t.git(&["add", "web/fetch.ts"]);
        t.write("notes.md", "- try jitter\n");
        t.write("target/debug/app", "binary\n");
        t.write(".env", "TOKEN=abc\n");
        t
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
