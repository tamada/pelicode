//! Discovery and parallel extraction of source files under a directory.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::Error;
use crate::config::Config;
use crate::extract::{FileFeatures, extract_file};
use crate::language::Language;

/// A source file found by [`discover`].
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Path of the file, as found on disk.
    pub path: PathBuf,
    /// Path relative to the scanned root (the path itself when a file was given),
    /// with `/` as the separator on every platform.
    pub relative: PathBuf,
    /// Language of the file.
    pub language: Language,
}

/// `path` with `/` as the separator, so that the paths recorded in a
/// database and printed in reports are the same on every platform.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use pelicode_core::slash_path;
///
/// assert_eq!(slash_path(Path::new("src/main.rs")), Path::new("src/main.rs"));
/// ```
pub fn slash_path(path: &Path) -> PathBuf {
    if std::path::MAIN_SEPARATOR == '/' {
        path.to_path_buf()
    } else {
        PathBuf::from(
            path.to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/"),
        )
    }
}

/// Lists the files of supported languages under `root`, honoring `.gitignore`.
/// `root` may also be a single file.
///
/// Hidden files are skipped, and so are, in Git repositories, files ignored by
/// `.gitignore`. Files are sorted by their relative path.
///
/// # Errors
///
/// [`Error::Io`] when `root` does not exist, and [`Error::Walk`] when a directory
/// cannot be read.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use pelicode_core::{Language, discover};
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::create_dir(dir.path().join("src")).unwrap();
/// std::fs::write(dir.path().join("src/lib.rs"), "fn f() {}").unwrap();
/// std::fs::write(dir.path().join("README.md"), "# readme").unwrap();
///
/// let files = discover(dir.path()).unwrap();
/// assert_eq!(files.len(), 1);
/// assert_eq!(files[0].relative, Path::new("src/lib.rs"));
/// assert_eq!(files[0].language, Language::Rust);
/// ```
pub fn discover(root: &Path) -> Result<Vec<SourceFile>, Error> {
    if root.is_file() {
        return Ok(Language::from_path(root)
            .map(|language| SourceFile {
                path: root.to_path_buf(),
                relative: slash_path(root),
                language,
            })
            .into_iter()
            .collect());
    }
    if !root.is_dir() {
        return Err(Error::Io(
            root.to_path_buf(),
            std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory"),
        ));
    }
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(root).build() {
        let entry = entry.map_err(|e| Error::Walk(e.to_string()))?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.into_path();
        if let Some(language) = Language::from_path(&path) {
            let relative = slash_path(path.strip_prefix(root).unwrap_or(&path));
            files.push(SourceFile {
                path,
                relative,
                language,
            });
        }
    }
    files.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(files)
}

/// Extracts features from `files` in parallel. Results keep the order of `files`.
///
/// # Examples
///
/// ```
/// use pelicode_core::{Config, discover, extract_all};
///
/// let dir = tempfile::tempdir().unwrap();
/// std::fs::write(dir.path().join("a.py"), "def f(x):\n    return x + 1\n").unwrap();
/// std::fs::write(dir.path().join("b.py"), "def g(y):\n    return y - 1\n").unwrap();
///
/// let files = discover(dir.path()).unwrap();
/// let features: Vec<_> = extract_all(&files, &Config::default())
///     .into_iter()
///     .collect::<Result<_, _>>()
///     .unwrap();
/// assert_eq!(features[0].functions[0].name, "f");
/// assert_eq!(features[1].functions[0].name, "g");
/// ```
pub fn extract_all(files: &[SourceFile], config: &Config) -> Vec<Result<FileFeatures, Error>> {
    files
        .par_iter()
        .map(|f| extract_file(&f.path, &f.relative, f.language, config))
        .collect()
}
