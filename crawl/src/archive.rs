//! Reading source files out of package archives, in memory only.
//!
//! Archives are never unpacked to disk, so hostile entry names cannot write
//! outside a directory, and no copy of the source code is kept.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use pelicode_core::Language;

use crate::Error;

/// Entries larger than this (often generated code) are skipped.
pub const MAX_ENTRY_SIZE: u64 = 8 * 1024 * 1024;
/// Total uncompressed size read from one archive.
pub const MAX_TOTAL_SIZE: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// `.zip`, `.jar`, `.whl`.
    Zip,
    /// `.tar.gz`, `.tgz`.
    TarGz,
}

impl ArchiveKind {
    pub fn from_file_name(name: &str) -> Option<ArchiveKind> {
        let name = name.to_ascii_lowercase();
        if name.ends_with(".zip") || name.ends_with(".jar") || name.ends_with(".whl") {
            Some(ArchiveKind::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(ArchiveKind::TarGz)
        } else {
            None
        }
    }
}

/// A source file of a supported language read from an archive.
pub struct Entry {
    pub path: PathBuf,
    pub language: Language,
    pub content: Vec<u8>,
}

/// Source files of supported languages in `bytes`. With `strip_top`, the
/// first path component (`name-1.0/` of an sdist) is removed.
pub fn read(kind: ArchiveKind, bytes: &[u8], strip_top: bool) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    let mut total = 0u64;
    let mut accept = |raw: &Path, size: u64, read: &mut dyn Read| -> Result<(), Error> {
        let Some(path) = sanitize(raw, strip_top) else {
            return Ok(());
        };
        let Some(language) = Language::from_path(&path) else {
            return Ok(());
        };
        if size > MAX_ENTRY_SIZE {
            return Ok(());
        }
        total += size;
        if total > MAX_TOTAL_SIZE {
            return Err(Error::Archive(format!(
                "more than {MAX_TOTAL_SIZE} bytes of source files"
            )));
        }
        let mut content = Vec::with_capacity(size as usize);
        read.take(MAX_ENTRY_SIZE + 1)
            .read_to_end(&mut content)
            .map_err(|e| Error::Archive(e.to_string()))?;
        if content.len() as u64 <= MAX_ENTRY_SIZE {
            entries.push(Entry {
                path,
                language,
                content,
            });
        }
        Ok(())
    };

    match kind {
        ArchiveKind::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
                .map_err(|e| Error::Archive(e.to_string()))?;
            for i in 0..zip.len() {
                let mut file = zip.by_index(i).map_err(|e| Error::Archive(e.to_string()))?;
                if !file.is_file() {
                    continue;
                }
                let name = PathBuf::from(
                    file.name()
                        .map_err(|e| Error::Archive(e.to_string()))?
                        .into_owned(),
                );
                let size = file.size();
                accept(&name, size, &mut file)?;
            }
        }
        ArchiveKind::TarGz => {
            let mut tar = tar::Archive::new(GzDecoder::new(bytes));
            for entry in tar.entries().map_err(|e| Error::Archive(e.to_string()))? {
                let mut entry = entry.map_err(|e| Error::Archive(e.to_string()))?;
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                let name = entry
                    .path()
                    .map_err(|e| Error::Archive(e.to_string()))?
                    .into_owned();
                let size = entry.size();
                accept(&name, size, &mut entry)?;
            }
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// A relative path without `..`, or `None` for anything suspicious.
fn sanitize(path: &Path, strip_top: bool) -> Option<PathBuf> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let parts = if strip_top {
        parts.get(1..)?
    } else {
        &parts[..]
    };
    if parts.is_empty() {
        return None;
    }
    Some(parts.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, content) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
        out.into_inner()
    }

    fn tar_gz_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, content.as_bytes())
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn reads_supported_files_from_zip() {
        let bytes = zip_of(&[
            ("META-INF/MANIFEST.MF", "x"),
            ("org/example/A.java", "class A {}"),
            ("org/example/B.kt", "class B"),
        ]);
        let entries = read(ArchiveKind::Zip, &bytes, false).unwrap();
        let paths: Vec<_> = entries.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(paths, ["org/example/A.java", "org/example/B.kt"]);
        assert_eq!(entries[1].language, Language::Kotlin);
    }

    #[test]
    fn strips_sdist_top_directory() {
        let bytes = tar_gz_of(&[
            ("demo-1.0/demo/__init__.py", "x = 1"),
            ("demo-1.0/PKG-INFO", ""),
        ]);
        let entries = read(ArchiveKind::TarGz, &bytes, true).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, Path::new("demo/__init__.py"));
        assert_eq!(entries[0].content, b"x = 1");
    }

    #[test]
    fn rejects_suspicious_paths() {
        assert_eq!(sanitize(Path::new("../etc/x.py"), false), None);
        assert_eq!(sanitize(Path::new("/abs/x.py"), false), None);
        assert_eq!(sanitize(Path::new("top"), true), None);
        assert_eq!(
            sanitize(Path::new("./a/b.py"), false),
            Some(PathBuf::from("a/b.py"))
        );
    }
}
