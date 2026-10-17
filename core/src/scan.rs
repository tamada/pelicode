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
    pub path: PathBuf,
    /// Path relative to the scanned root (the path itself when a file was given).
    pub relative: PathBuf,
    pub language: Language,
}

/// Lists the files of supported languages under `root`, honoring `.gitignore`.
/// `root` may also be a single file.
pub fn discover(root: &Path) -> Result<Vec<SourceFile>, Error> {
    if root.is_file() {
        return Ok(Language::from_path(root)
            .map(|language| SourceFile {
                path: root.to_path_buf(),
                relative: root.to_path_buf(),
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
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
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
pub fn extract_all(files: &[SourceFile], config: &Config) -> Vec<Result<FileFeatures, Error>> {
    files
        .par_iter()
        .map(|f| extract_file(&f.path, &f.relative, f.language, config))
        .collect()
}
