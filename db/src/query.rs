//! Lookups used by the detector.
//!
//! The detector first looks up functions with the same exact hash
//! ([`Database::functions_by_exact_hash`]), then the functions sharing an
//! LSH bucket ([`Database::functions_by_lsh_bucket`]), and compares their
//! fingerprints ([`Database::fingerprints`]). The candidates it keeps are
//! described by [`Database::function_info`] and traced back to the packages
//! they occur in by [`Database::occurrences`].

use pelicode_core::{Hash128, Level};
use rusqlite::params;
use serde::Serialize;

use crate::{Database, Result};

/// A stored function, without its occurrences.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionInfo {
    /// Identifier of the function in the database.
    pub id: i64,
    /// Language of the function.
    pub language: String,
    /// Name of the function (for display).
    pub name: String,
    /// Tree-sitter node kind of the function.
    pub kind: String,
    /// First line (1-based).
    pub start_line: usize,
    /// Last line (1-based, inclusive).
    pub end_line: usize,
    /// Start of the byte range in the file.
    pub start_byte: usize,
    /// End of the byte range in the file (exclusive).
    pub end_byte: usize,
    /// Number of tokens.
    pub token_count: usize,
}

/// Where a stored function appears: a file of a package version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Occurrence {
    /// Ecosystem of the package.
    pub ecosystem: String,
    /// Name of the package.
    pub package: String,
    /// Version of the package.
    pub version: String,
    /// Path of the file in the package.
    pub path: String,
    /// License of the package version.
    pub license: Option<String>,
    /// Publication date and time of the version (RFC 3339).
    pub published_at: Option<String>,
    /// URL of the source repository of the package.
    pub repository_url: Option<String>,
    /// URL the source archive was downloaded from.
    pub archive_url: Option<String>,
    /// SHA-256 of the file content (hex).
    pub content_sha256: String,
}

/// A stored fingerprint with the source lines it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredFingerprint {
    /// Hash of the k-gram.
    pub hash: u64,
    /// First source line covered (1-based).
    pub start_line: u32,
    /// Last source line covered (1-based, inclusive).
    pub end_line: u32,
}

impl Database {
    /// Functions whose exact hash at `level` is `hash`.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config { min_tokens: 0, ..Config::default() };
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// # let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
    /// # let file = extract_bytes("src/calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// # let package = PackageInfo {
    /// #     ecosystem: "pypi".into(), name: "calc".into(), version: "1.0".into(),
    /// #     license_spdx: Some("MIT".into()), ..PackageInfo::default()
    /// # };
    /// # db.index(&package, &[file.clone()]).unwrap();
    /// # let function = &file.functions[0];
    /// use pelicode_core::Level;
    ///
    /// let ids = db.functions_by_exact_hash(Level::N1, &function.exact_hashes[&Level::N1]).unwrap();
    /// assert_eq!(ids.len(), 1);
    /// ```
    pub fn functions_by_exact_hash(&self, level: Level, hash: &Hash128) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT function_id FROM exact_hash WHERE level = ?1 AND hash = ?2")?;
        let ids = stmt.query_map(params![level.as_u8(), &hash.0[..]], |r| r.get(0))?;
        Ok(ids.collect::<rusqlite::Result<_>>()?)
    }

    /// Functions sharing LSH bucket `bucket` of `band` at `level`.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config { min_tokens: 0, ..Config::default() };
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// # let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
    /// # let file = extract_bytes("src/calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// # let package = PackageInfo {
    /// #     ecosystem: "pypi".into(), name: "calc".into(), version: "1.0".into(),
    /// #     license_spdx: Some("MIT".into()), ..PackageInfo::default()
    /// # };
    /// # db.index(&package, &[file.clone()]).unwrap();
    /// # let function = &file.functions[0];
    /// use pelicode_core::Level;
    ///
    /// let buckets = &function.features[&Level::N1].lsh_buckets;
    /// assert_eq!(db.functions_by_lsh_bucket(Level::N1, 0, buckets[0]).unwrap().len(), 1);
    /// ```
    pub fn functions_by_lsh_bucket(
        &self,
        level: Level,
        band: usize,
        bucket: u64,
    ) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT function_id FROM lsh_bucket WHERE level = ?1 AND band = ?2 AND bucket_hash = ?3",
        )?;
        let ids = stmt.query_map(params![level.as_u8(), band as i64, bucket as i64], |r| {
            r.get(0)
        })?;
        Ok(ids.collect::<rusqlite::Result<_>>()?)
    }

    /// Fingerprints of a stored function at `level`.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config { min_tokens: 0, ..Config::default() };
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// # let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
    /// # let file = extract_bytes("src/calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// # let package = PackageInfo {
    /// #     ecosystem: "pypi".into(), name: "calc".into(), version: "1.0".into(),
    /// #     license_spdx: Some("MIT".into()), ..PackageInfo::default()
    /// # };
    /// # db.index(&package, &[file.clone()]).unwrap();
    /// # let function = &file.functions[0];
    /// use pelicode_core::Level;
    ///
    /// let id = db.functions_by_exact_hash(Level::N1, &function.exact_hashes[&Level::N1]).unwrap()[0];
    /// let stored = db.fingerprints(id, Level::N1).unwrap();
    /// assert_eq!(stored.len(), function.features[&Level::N1].fingerprints.len());
    /// assert!(stored.iter().all(|f| (1..=5).contains(&f.start_line)));
    /// ```
    pub fn fingerprints(&self, function_id: i64, level: Level) -> Result<Vec<StoredFingerprint>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT hash, start_line, end_line FROM fingerprint
             WHERE function_id = ?1 AND level = ?2 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![function_id, level.as_u8()], |r| {
            Ok(StoredFingerprint {
                hash: r.get::<_, i64>(0)? as u64,
                start_line: r.get(1)?,
                end_line: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A stored function, without its occurrences.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when no function has the identifier or the database
    /// cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config { min_tokens: 0, ..Config::default() };
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// # let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
    /// # let file = extract_bytes("src/calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// # let package = PackageInfo {
    /// #     ecosystem: "pypi".into(), name: "calc".into(), version: "1.0".into(),
    /// #     license_spdx: Some("MIT".into()), ..PackageInfo::default()
    /// # };
    /// # db.index(&package, &[file.clone()]).unwrap();
    /// # let function = &file.functions[0];
    /// use pelicode_core::Level;
    ///
    /// let id = db.functions_by_exact_hash(Level::N1, &function.exact_hashes[&Level::N1]).unwrap()[0];
    /// let info = db.function_info(id).unwrap();
    /// assert_eq!((info.name.as_str(), info.start_line, info.end_line), ("total", 1, 5));
    /// ```
    pub fn function_info(&self, function_id: i64) -> Result<FunctionInfo> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, sf.language, f.name, f.kind, f.start_line, f.end_line,
                    f.start_byte, f.end_byte, f.token_count
             FROM function f JOIN source_file sf ON sf.id = f.source_file_id
             WHERE f.id = ?1",
        )?;
        Ok(stmt.query_row([function_id], |r| {
            Ok(FunctionInfo {
                id: r.get(0)?,
                language: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                start_line: r.get::<_, i64>(4)? as usize,
                end_line: r.get::<_, i64>(5)? as usize,
                start_byte: r.get::<_, i64>(6)? as usize,
                end_byte: r.get::<_, i64>(7)? as usize,
                token_count: r.get::<_, i64>(8)? as usize,
            })
        })?)
    }

    /// Every place a stored function appears, earliest publication first.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config { min_tokens: 0, ..Config::default() };
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// # let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
    /// # let file = extract_bytes("src/calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// # let package = PackageInfo {
    /// #     ecosystem: "pypi".into(), name: "calc".into(), version: "1.0".into(),
    /// #     license_spdx: Some("MIT".into()), ..PackageInfo::default()
    /// # };
    /// # db.index(&package, &[file.clone()]).unwrap();
    /// # let function = &file.functions[0];
    /// use pelicode_core::Level;
    ///
    /// let id = db.functions_by_exact_hash(Level::N1, &function.exact_hashes[&Level::N1]).unwrap()[0];
    /// let occurrences = db.occurrences(id).unwrap();
    /// assert_eq!(occurrences[0].package, "calc");
    /// assert_eq!(occurrences[0].path, "src/calc.py");
    /// ```
    pub fn occurrences(&self, function_id: i64) -> Result<Vec<Occurrence>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT e.name, p.name, pv.version, vf.path, pv.license_spdx, pv.published_at,
                    p.repository_url, pv.archive_url, sf.content_sha256
             FROM function f
             JOIN source_file sf ON sf.id = f.source_file_id
             JOIN version_file vf ON vf.source_file_id = sf.id
             JOIN package_version pv ON pv.id = vf.package_version_id
             JOIN package p ON p.id = pv.package_id
             JOIN ecosystem e ON e.id = p.ecosystem_id
             WHERE f.id = ?1
             ORDER BY pv.published_at IS NULL, pv.published_at, e.name, p.name, pv.version, vf.path",
        )?;
        let rows = stmt.query_map([function_id], |r| {
            Ok(Occurrence {
                ecosystem: r.get(0)?,
                package: r.get(1)?,
                version: r.get(2)?,
                path: r.get(3)?,
                license: r.get(4)?,
                published_at: r.get(5)?,
                repository_url: r.get(6)?,
                archive_url: r.get(7)?,
                content_sha256: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
