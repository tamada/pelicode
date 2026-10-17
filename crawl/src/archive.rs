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

/// License files larger than this are not read.
const MAX_LICENSE_SIZE: u64 = 256 * 1024;

/// Format of a source archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// `.zip`, `.jar`, `.whl`, Go module zips.
    Zip,
    /// `.tar.gz`, `.tgz`.
    TarGz,
    /// RubyGems `.gem`: a plain tar whose `data.tar.gz` holds the files.
    Gem,
}

impl ArchiveKind {
    /// The format of an archive from its file name; `None` when unsupported.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_crawl::archive::ArchiveKind;
    ///
    /// assert_eq!(ArchiveKind::from_file_name("demo-1.0.tar.gz"), Some(ArchiveKind::TarGz));
    /// assert_eq!(ArchiveKind::from_file_name("gson-2.11.0-sources.jar"), Some(ArchiveKind::Zip));
    /// assert_eq!(ArchiveKind::from_file_name("rack-3.1.8.gem"), Some(ArchiveKind::Gem));
    /// assert_eq!(ArchiveKind::from_file_name("demo.tar.bz2"), None);
    /// ```
    pub fn from_file_name(name: &str) -> Option<ArchiveKind> {
        let name = name.to_ascii_lowercase();
        if name.ends_with(".zip") || name.ends_with(".jar") || name.ends_with(".whl") {
            Some(ArchiveKind::Zip)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            Some(ArchiveKind::TarGz)
        } else if name.ends_with(".gem") {
            Some(ArchiveKind::Gem)
        } else {
            None
        }
    }
}

/// A source file of a supported language read from an archive.
pub struct Entry {
    /// Path of the entry in the archive, with `/` as the separator.
    pub path: PathBuf,
    /// Language of the entry.
    pub language: Language,
    /// Content of the entry.
    pub content: Vec<u8>,
}

/// What pelicode needs from an archive.
#[derive(Default)]
pub struct Contents {
    /// Source files of supported languages, sorted by path.
    pub sources: Vec<Entry>,
    /// Texts of top-level `LICENSE*`, `LICENCE*` and `COPYING*` files.
    pub license_texts: Vec<String>,
}

/// Reads `bytes`, dropping the first `strip` path components of every entry
/// (`name-1.0/` of an sdist, `module@version/` of a Go module).
///
/// Entries larger than [`MAX_ENTRY_SIZE`], and entries whose path is absolute or
/// goes up with `..`, are skipped.
///
/// # Errors
///
/// [`Error::Archive`] when the archive cannot be read, or holds more than
/// [`MAX_TOTAL_SIZE`] bytes of source files.
///
/// # Examples
///
/// ```
/// use std::io::Write;
/// use pelicode_crawl::archive::{ArchiveKind, read};
///
/// let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
/// for (name, content) in [("demo-1.0/demo/core.py", "x = 1"), ("demo-1.0/LICENSE", "MIT License")] {
///     zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
///     zip.write_all(content.as_bytes()).unwrap();
/// }
/// let bytes = zip.finish().unwrap().into_inner();
///
/// let contents = read(ArchiveKind::Zip, &bytes, 1).unwrap();
/// assert_eq!(contents.sources[0].path, std::path::Path::new("demo/core.py"));
/// assert_eq!(contents.license_texts, ["MIT License"]);
/// ```
pub fn read(kind: ArchiveKind, bytes: &[u8], strip: usize) -> Result<Contents, Error> {
    let mut collector = Collector {
        strip,
        total: 0,
        contents: Contents::default(),
    };
    match kind {
        ArchiveKind::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(archive_error)?;
            for i in 0..zip.len() {
                let mut file = zip.by_index(i).map_err(archive_error)?;
                if !file.is_file() {
                    continue;
                }
                let name = PathBuf::from(file.name().map_err(archive_error)?.into_owned());
                let size = file.size();
                collector.accept(&name, size, &mut file)?;
            }
        }
        ArchiveKind::TarGz => {
            let mut tar = tar::Archive::new(GzDecoder::new(bytes));
            for entry in tar.entries().map_err(archive_error)? {
                let mut entry = entry.map_err(archive_error)?;
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                let name = entry.path().map_err(archive_error)?.into_owned();
                let size = entry.size();
                collector.accept(&name, size, &mut entry)?;
            }
        }
        ArchiveKind::Gem => {
            let mut tar = tar::Archive::new(bytes);
            for entry in tar.entries().map_err(archive_error)? {
                let mut entry = entry.map_err(archive_error)?;
                if entry.path().map_err(archive_error)?.as_ref() != Path::new("data.tar.gz") {
                    continue;
                }
                let mut data = Vec::new();
                entry.read_to_end(&mut data).map_err(archive_error)?;
                return read(ArchiveKind::TarGz, &data, strip);
            }
            return Err(Error::Archive("no data.tar.gz in gem".into()));
        }
    }
    collector
        .contents
        .sources
        .sort_by(|a, b| a.path.cmp(&b.path));
    Ok(collector.contents)
}

fn archive_error(e: impl std::fmt::Display) -> Error {
    Error::Archive(e.to_string())
}

struct Collector {
    strip: usize,
    total: u64,
    contents: Contents,
}

impl Collector {
    fn accept(&mut self, raw: &Path, size: u64, read: &mut dyn Read) -> Result<(), Error> {
        let Some(path) = sanitize(raw, self.strip) else {
            return Ok(());
        };
        if is_license_file(&path) {
            if size <= MAX_LICENSE_SIZE {
                let mut text = String::new();
                if read
                    .take(MAX_LICENSE_SIZE)
                    .read_to_string(&mut text)
                    .is_ok()
                {
                    self.contents.license_texts.push(text);
                }
            }
            return Ok(());
        }
        let Some(language) = Language::from_path(&path) else {
            return Ok(());
        };
        if size > MAX_ENTRY_SIZE {
            return Ok(());
        }
        self.total += size;
        if self.total > MAX_TOTAL_SIZE {
            return Err(Error::Archive(format!(
                "more than {MAX_TOTAL_SIZE} bytes of source files"
            )));
        }
        let mut content = Vec::with_capacity(size as usize);
        read.take(MAX_ENTRY_SIZE + 1)
            .read_to_end(&mut content)
            .map_err(archive_error)?;
        if content.len() as u64 <= MAX_ENTRY_SIZE {
            self.contents.sources.push(Entry {
                path,
                language,
                content,
            });
        }
        Ok(())
    }
}

fn is_license_file(path: &Path) -> bool {
    path.components().count() == 1
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| {
                let upper = name.to_ascii_uppercase();
                ["LICENSE", "LICENCE", "COPYING"]
                    .iter()
                    .any(|prefix| upper.starts_with(prefix))
            })
}

/// A relative path without `..`, or `None` for anything suspicious.
fn sanitize(path: &Path, strip: usize) -> Option<PathBuf> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let parts = parts.get(strip..)?;
    if parts.is_empty() {
        return None;
    }
    // `/` on every platform, like the paths of local files (see `slash_path`).
    let joined: Vec<_> = parts.iter().map(|p| p.to_string_lossy()).collect();
    Some(PathBuf::from(joined.join("/")))
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
        let entries = read(ArchiveKind::Zip, &bytes, 0).unwrap().sources;
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
        let entries = read(ArchiveKind::TarGz, &bytes, 1).unwrap().sources;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, Path::new("demo/__init__.py"));
        assert_eq!(entries[0].content, b"x = 1");
    }

    #[test]
    fn rejects_suspicious_paths() {
        assert_eq!(sanitize(Path::new("../etc/x.py"), 0), None);
        assert_eq!(sanitize(Path::new("/abs/x.py"), 0), None);
        assert_eq!(sanitize(Path::new("top"), 1), None);
        assert_eq!(
            sanitize(Path::new("./a/b.py"), 0),
            Some(PathBuf::from("a/b.py"))
        );
        assert_eq!(
            sanitize(Path::new("github.com/x/y@v1.0.0/a.go"), 3),
            Some(PathBuf::from("a.go"))
        );
    }

    #[test]
    fn reads_gems_and_license_files() {
        let data = tar_gz_of(&[
            ("lib/rack.rb", "module Rack; end"),
            ("MIT-LICENSE", "not matched"),
            ("LICENSE.md", "MIT License"),
            ("lib/LICENSE", "nested, ignored"),
        ]);
        let mut gem = tar::Builder::new(Vec::new());
        for (name, content) in [("metadata.gz", &b"x"[..]), ("data.tar.gz", &data[..])] {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            gem.append_data(&mut header, name, content).unwrap();
        }
        let gem = gem.into_inner().unwrap();
        let contents = read(ArchiveKind::Gem, &gem, 0).unwrap();
        assert_eq!(contents.sources.len(), 1);
        assert_eq!(contents.sources[0].path, Path::new("lib/rack.rb"));
        assert_eq!(contents.license_texts, ["MIT License"]);
    }
}
