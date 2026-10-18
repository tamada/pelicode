//! Changed files and lines of a git revision range.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Lines added or modified in each file, as 1-based inclusive ranges.
pub struct Changes {
    pub files: BTreeMap<PathBuf, Vec<(usize, usize)>>,
    /// Where the new content is.
    head: Head,
    repo: PathBuf,
}

/// The new side of the changes.
enum Head {
    /// A revision.
    Revision(String),
    /// The working tree.
    WorkTree,
    /// The index (staged changes).
    Index,
}

impl Changes {
    /// Changes of `range` (`BASE`, `BASE..HEAD` or `BASE...HEAD`) in the repository at `repo`.
    pub fn of(repo: &Path, range: &str) -> Result<Changes> {
        let head = range
            .split_once("..")
            .map(|(_, head)| head.trim_start_matches('.'))
            .map(|head| if head.is_empty() { "HEAD" } else { head }.to_string())
            .map_or(Head::WorkTree, Head::Revision);
        Changes::diff(repo, &[range], head)
    }

    /// Changes staged for the next commit: the index against `HEAD`, or
    /// everything staged in a repository without commits.
    pub fn staged(repo: &Path) -> Result<Changes> {
        Changes::diff(repo, &["--cached"], Head::Index)
    }

    fn diff(repo: &Path, what: &[&str], head: Head) -> Result<Changes> {
        let mut args = vec![
            "diff",
            "--unified=0",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            "--diff-filter=AMR",
        ];
        args.extend_from_slice(what);
        let diff = git(repo, &args)?;
        Ok(Changes {
            files: parse_diff(&String::from_utf8_lossy(&diff)),
            head,
            repo: repo.to_path_buf(),
        })
    }

    /// Content of `path` at the new side of the range.
    pub fn content(&self, path: &Path) -> Result<Vec<u8>> {
        match &self.head {
            Head::Revision(head) => {
                git(&self.repo, &["show", &format!("{head}:{}", path.display())])
            }
            // `:path` is the staged content.
            Head::Index => git(&self.repo, &["show", &format!(":{}", path.display())]),
            Head::WorkTree => {
                let full = self.repo.join(path);
                std::fs::read(&full).with_context(|| format!("cannot read {}", full.display()))
            }
        }
    }

    /// Whether lines `start..=end` of `path` overlap a change.
    pub fn touches(&self, path: &Path, start: usize, end: usize) -> bool {
        self.files
            .get(path)
            .is_some_and(|ranges| ranges.iter().any(|&(s, e)| s <= end && start <= e))
    }
}

/// Runs `git -C repo args...` and returns its standard output; fails when git does.
fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .output()
        .context("cannot run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// New-side line ranges of each file of a `git diff --unified=0` output.
fn parse_diff(diff: &str) -> BTreeMap<PathBuf, Vec<(usize, usize)>> {
    let mut files: BTreeMap<PathBuf, Vec<(usize, usize)>> = BTreeMap::new();
    let mut current: Option<PathBuf> = None;
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            current = path.strip_prefix("b/").map(PathBuf::from);
        } else if let Some(hunk) = line.strip_prefix("@@ ")
            && let Some(path) = &current
            && let Some(range) = hunk_range(hunk)
        {
            files.entry(path.clone()).or_default().push(range);
        }
    }
    files
}

/// The new-side range of a hunk header such as `-10,2 +12,3 @@`.
/// A pure deletion marks the line where the deletion happened.
fn hunk_range(hunk: &str) -> Option<(usize, usize)> {
    let new = hunk.split_whitespace().find_map(|t| t.strip_prefix('+'))?;
    let (start, count) = match new.split_once(',') {
        Some((s, c)) => (s.parse::<usize>().ok()?, c.parse::<usize>().ok()?),
        None => (new.parse::<usize>().ok()?, 1),
    };
    let start = start.max(1);
    Some((start, start + count.max(1) - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_diff() {
        let diff = "\
diff --git a/src/A.java b/src/A.java
--- a/src/A.java
+++ b/src/A.java
@@ -3,0 +4,2 @@ class A {
+    int x;
+    int y;
@@ -20 +22 @@ class A {
-old
+new
@@ -30,2 +31,0 @@
-gone
-gone
diff --git a/new.py b/new.py
--- /dev/null
+++ b/new.py
@@ -0,0 +1,3 @@
";
        let files = parse_diff(diff);
        assert_eq!(files[Path::new("src/A.java")], [(4, 5), (22, 22), (31, 31)]);
        assert_eq!(files[Path::new("new.py")], [(1, 3)]);
    }
}
