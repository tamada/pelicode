//! SQLite storage of function features (specification §5).

#![warn(missing_docs)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::str::FromStr;

use pelicode_core::{Config, FileFeatures, Level, NORMALIZER_VERSION};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::Serialize;

/// Errors of the database.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An error of SQLite.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The database was built with other parameters or another table layout than the current ones.
    #[error("database was built with {key} = {stored}, but the current setting is {current}")]
    Incompatible {
        /// Name of the parameter.
        key: String,
        /// Value recorded in the database.
        stored: String,
        /// Value of the current setting.
        current: String,
    },
    /// A parameter recorded in the database cannot be read.
    #[error("invalid value in database meta: {0}")]
    InvalidMeta(String),
    /// The database is a snapshot, which is for checking only.
    #[error("database is a snapshot for checking only; index into the database it was made from")]
    Snapshot,
}

mod clean;
mod query;
mod snapshot;
mod stats;

pub use clean::CleanSummary;
pub use query::{FunctionInfo, Occurrence, StoredFingerprint};
pub use snapshot::{SnapshotOptions, SnapshotSummary};
pub use stats::{CommonHash, HashStatsInfo};

/// Result of the database operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Version of the table layout; databases with another version are rejected.
pub const SCHEMA_VERSION: u32 = 5;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS ecosystem (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS package (
    id             INTEGER PRIMARY KEY,
    ecosystem_id   INTEGER NOT NULL REFERENCES ecosystem(id),
    name           TEXT NOT NULL,
    repository_url TEXT,
    UNIQUE (ecosystem_id, name)
);
CREATE TABLE IF NOT EXISTS package_version (
    id             INTEGER PRIMARY KEY,
    package_id     INTEGER NOT NULL REFERENCES package(id),
    version        TEXT NOT NULL,
    published_at   TEXT,
    license_spdx   TEXT,
    archive_url    TEXT,
    archive_sha256 TEXT,
    crawled_at     TEXT,
    status         TEXT NOT NULL,
    error          TEXT,
    UNIQUE (package_id, version)
);
CREATE TABLE IF NOT EXISTS source_file (
    id             INTEGER PRIMARY KEY,
    content_sha256 TEXT NOT NULL UNIQUE,
    language       TEXT NOT NULL,
    function_count INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS version_file (
    package_version_id INTEGER NOT NULL REFERENCES package_version(id),
    source_file_id     INTEGER NOT NULL REFERENCES source_file(id),
    path               TEXT NOT NULL,
    PRIMARY KEY (package_version_id, path)
);
CREATE INDEX IF NOT EXISTS version_file_source ON version_file(source_file_id);
CREATE TABLE IF NOT EXISTS function (
    id                 INTEGER PRIMARY KEY,
    source_file_id     INTEGER NOT NULL REFERENCES source_file(id),
    parent_function_id INTEGER REFERENCES function(id),
    name               TEXT NOT NULL,
    kind               TEXT NOT NULL,
    start_line         INTEGER NOT NULL,
    end_line           INTEGER NOT NULL,
    start_column       INTEGER NOT NULL,
    end_column         INTEGER NOT NULL,
    start_byte         INTEGER NOT NULL,
    end_byte           INTEGER NOT NULL,
    token_count        INTEGER NOT NULL,
    node_count         INTEGER NOT NULL,
    depth              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS function_source ON function(source_file_id);
CREATE TABLE IF NOT EXISTS exact_hash (
    function_id INTEGER NOT NULL REFERENCES function(id),
    level       INTEGER NOT NULL,
    hash        BLOB NOT NULL,
    PRIMARY KEY (function_id, level)
);
CREATE INDEX IF NOT EXISTS exact_hash_lookup ON exact_hash(level, hash);
CREATE TABLE IF NOT EXISTS fingerprint (
    function_id INTEGER NOT NULL REFERENCES function(id),
    level       INTEGER NOT NULL,
    hash        INTEGER NOT NULL,
    position    INTEGER NOT NULL,
    start_line  INTEGER NOT NULL,
    end_line    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS fingerprint_lookup ON fingerprint(level, hash);
CREATE INDEX IF NOT EXISTS fingerprint_function ON fingerprint(function_id, level);
CREATE TABLE IF NOT EXISTS minhash (
    function_id INTEGER NOT NULL REFERENCES function(id),
    level       INTEGER NOT NULL,
    signature   BLOB NOT NULL,
    PRIMARY KEY (function_id, level)
);
CREATE TABLE IF NOT EXISTS lsh_bucket (
    level       INTEGER NOT NULL,
    band        INTEGER NOT NULL,
    bucket_hash INTEGER NOT NULL,
    function_id INTEGER NOT NULL REFERENCES function(id)
);
CREATE INDEX IF NOT EXISTS lsh_bucket_lookup ON lsh_bucket(level, band, bucket_hash);
-- Document frequencies, rebuilt by `refresh_hash_stats`. Only hashes found
-- in two or more packages are stored; a missing hash counts as one package.
CREATE TABLE IF NOT EXISTS exact_stats (
    level          INTEGER NOT NULL,
    hash           BLOB NOT NULL,
    package_count  INTEGER NOT NULL,
    function_count INTEGER NOT NULL,
    PRIMARY KEY (level, hash)
);
CREATE TABLE IF NOT EXISTS fingerprint_stats (
    level          INTEGER NOT NULL,
    hash           INTEGER NOT NULL,
    package_count  INTEGER NOT NULL,
    function_count INTEGER NOT NULL,
    PRIMARY KEY (level, hash)
);
-- Hashes whose package counts may have changed since the last statistics:
-- those of the files linked to or unlinked from a version since then.
CREATE TABLE IF NOT EXISTS stats_pending_exact (
    level INTEGER NOT NULL,
    hash  BLOB NOT NULL,
    PRIMARY KEY (level, hash)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS stats_pending_fingerprint (
    level INTEGER NOT NULL,
    hash  INTEGER NOT NULL,
    PRIMARY KEY (level, hash)
) WITHOUT ROWID;
"#;

/// Where indexed source files come from.
#[derive(Debug, Clone, Default)]
pub struct PackageInfo {
    /// Ecosystem of the package: `pypi`, `maven`, `go`, `rubygems`, `crates` or `local`.
    pub ecosystem: String,
    /// Name of the package within its ecosystem.
    pub name: String,
    /// Version of the package.
    pub version: String,
    /// License, as an SPDX expression when it could be identified.
    pub license_spdx: Option<String>,
    /// URL of the source repository.
    pub repository_url: Option<String>,
    /// Publication date and time (RFC 3339).
    pub published_at: Option<String>,
    /// URL the source archive was downloaded from.
    pub archive_url: Option<String>,
    /// SHA-256 of the downloaded archive (hex).
    pub archive_sha256: Option<String>,
}

/// Result of [`Database::index`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct IndexSummary {
    /// Files given to [`Database::index`].
    pub files: usize,
    /// Files whose content was not in the database yet.
    pub new_files: usize,
    /// Functions stored from new files.
    pub functions: usize,
    /// Of `functions`, those with fingerprints.
    pub indexed_functions: usize,
}

/// Number of rows of the main tables.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Stats {
    /// Ecosystems.
    pub ecosystems: usize,
    /// Packages.
    pub packages: usize,
    /// Package versions, failed ones included.
    pub versions: usize,
    /// Distinct source files (by content).
    pub source_files: usize,
    /// Stored functions.
    pub functions: usize,
    /// Stored functions with fingerprints.
    pub indexed_functions: usize,
}

/// One occurrence of a stored function, for export.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionRow {
    /// Identifier of the function in the database.
    pub id: i64,
    /// Ecosystem of the package.
    pub ecosystem: String,
    /// Name of the package.
    pub package: String,
    /// Version of the package.
    pub version: String,
    /// License of the package version.
    pub license: Option<String>,
    /// Path of the file in the package.
    pub path: String,
    /// Language of the file.
    pub language: String,
    /// Name of the function (for display).
    pub name: String,
    /// Tree-sitter node kind of the function.
    pub kind: String,
    /// First line (1-based).
    pub start_line: usize,
    /// Last line (1-based, inclusive).
    pub end_line: usize,
    /// 1-based byte column of the first character.
    pub start_column: usize,
    /// 1-based byte column of the last character.
    pub end_column: usize,
    /// Start of the byte range in the file.
    pub start_byte: usize,
    /// End of the byte range in the file (exclusive).
    pub end_byte: usize,
    /// Identifier of the enclosing function, if any.
    pub parent_id: Option<i64>,
    /// Number of tokens.
    pub token_count: usize,
    /// Number of named syntax tree nodes.
    pub node_count: usize,
    /// Depth of the syntax tree.
    pub depth: usize,
    /// Whether the function has fingerprints.
    pub indexed: bool,
    /// Exact hashes per normalization level (hex).
    pub exact_hashes: BTreeMap<Level, String>,
}

/// A pelicode database: an SQLite file holding the features of functions and
/// where they were found, but no source code.
///
/// # Examples
///
/// ```
/// use pelicode_core::{Config, Language, extract_bytes};
/// use pelicode_db::{Database, PackageInfo};
///
/// let dir = tempfile::tempdir().unwrap();
/// let config = Config::default();
/// let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
///
/// let source = b"package demo\nfunc Add(a, b int) int { return a + b }\n";
/// let file = extract_bytes("add.go".as_ref(), Language::Go, source, &config).unwrap();
/// let package = PackageInfo {
///     ecosystem: "local".into(),
///     name: "demo".into(),
///     version: "1.0".into(),
///     ..PackageInfo::default()
/// };
/// db.index(&package, &[file]).unwrap();
/// assert_eq!(db.stats().unwrap().functions, 1);
/// ```
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens (or creates) a database for features computed with `config`.
    /// Fails when the database was built with incompatible parameters.
    ///
    /// # Errors
    ///
    /// [`Error::Incompatible`] when the database was built with other feature
    /// parameters or another table layout, and [`Error::Sqlite`] when the file cannot
    /// be opened.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::{Database, Error};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let path = dir.path().join("pelicode.db");
    /// Database::open(&path, &Config::default()).unwrap();
    ///
    /// let other = Config { kgram: 5, ..Config::default() };
    /// assert!(matches!(Database::open(&path, &other), Err(Error::Incompatible { .. })));
    /// ```
    pub fn open(path: impl AsRef<Path>, config: &Config) -> Result<Database> {
        let db = Database::open_unchecked(path)?;
        db.check_meta(config)?;
        Ok(db)
    }

    /// Opens a database without checking its parameters (for reading).
    ///
    /// The table layout is still checked; use [`Database::config_from_meta`] to featurize
    /// code like the stored functions.
    ///
    /// # Errors
    ///
    /// [`Error::Incompatible`] when the table layout differs, and [`Error::Sqlite`]
    /// when the file cannot be opened.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let path = dir.path().join("pelicode.db");
    /// Database::open(&path, &Config { kgram: 5, ..Config::default() }).unwrap();
    ///
    /// let db = Database::open_unchecked(&path).unwrap();
    /// assert_eq!(db.meta().unwrap()["kgram"], "5");
    /// ```
    pub fn open_unchecked(path: impl AsRef<Path>) -> Result<Database> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        let db = Database { conn };
        db.check_schema()?;
        Ok(db)
    }

    fn check_schema(&self) -> Result<()> {
        let stored = self.meta()?;
        match stored.get("schema_version") {
            Some(v) if *v == SCHEMA_VERSION.to_string() => Ok(()),
            // Version 5 only adds the tables of pending hashes, created above.
            Some(v) if v == "4" => {
                // The statistics can be updated incrementally from now on if
                // nothing was indexed since they were made.
                let current = stored.get("index_generation").map_or("0", String::as_str);
                if stored.get("stats_generation").map(String::as_str) == Some(current) {
                    stats::set_pending_complete(&self.conn, true)?;
                }
                self.conn.execute(
                    "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                    [SCHEMA_VERSION.to_string()],
                )?;
                Ok(())
            }
            None if stored.is_empty() => {
                self.conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    [SCHEMA_VERSION.to_string()],
                )?;
                Ok(())
            }
            other => Err(Error::Incompatible {
                key: "schema_version".into(),
                stored: other.cloned().unwrap_or_else(|| "(none)".into()),
                current: SCHEMA_VERSION.to_string(),
            }),
        }
    }

    /// The parameters recorded in the database.
    ///
    /// The keys are `schema_version`, `normalizer_version`, `ident_mode`, `kgram`,
    /// `window`, `minhash_size` and `bands`, and the bookkeeping of the hash
    /// statistics.
    ///
    /// # Errors
    ///
    /// [`Error::Sqlite`] when the table cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::Config;
    /// # use pelicode_db::Database;
    /// # let dir = tempfile::tempdir().unwrap();
    /// let db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// let meta = db.meta().unwrap();
    /// assert_eq!(meta["kgram"], "10");
    /// assert_eq!(meta["window"], "4");
    /// ```
    pub fn meta(&self) -> Result<BTreeMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM meta")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// `base` with the parameters recorded in the database, so that queries are
    /// featurized exactly like the stored functions. Fingerprint levels become
    /// the levels that actually have fingerprints in the database.
    ///
    /// # Errors
    ///
    /// [`Error::Incompatible`] when the database was built with other normalization
    /// rules, and [`Error::InvalidMeta`] when a recorded parameter cannot be parsed.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let path = dir.path().join("pelicode.db");
    /// Database::open(&path, &Config { kgram: 7, ..Config::default() }).unwrap();
    ///
    /// let db = Database::open_unchecked(&path).unwrap();
    /// let config = db.config_from_meta(&Config::default()).unwrap();
    /// assert_eq!(config.kgram, 7);
    /// ```
    pub fn config_from_meta(&self, base: &Config) -> Result<Config> {
        let stored = self.meta()?;
        fn parse<T: FromStr>(
            stored: &BTreeMap<String, String>,
            key: &str,
            value: &mut T,
        ) -> Result<()> {
            if let Some(s) = stored.get(key) {
                *value = s
                    .parse()
                    .map_err(|_| Error::InvalidMeta(format!("{key} = {s}")))?;
            }
            Ok(())
        }
        let mut version = NORMALIZER_VERSION;
        parse(&stored, "normalizer_version", &mut version)?;
        if version != NORMALIZER_VERSION {
            return Err(Error::Incompatible {
                key: "normalizer_version".into(),
                stored: version.to_string(),
                current: NORMALIZER_VERSION.to_string(),
            });
        }
        let mut config = base.clone();
        parse(&stored, "ident_mode", &mut config.ident_mode)?;
        parse(&stored, "kgram", &mut config.kgram)?;
        parse(&stored, "window", &mut config.window)?;
        parse(&stored, "minhash_size", &mut config.minhash_size)?;
        parse(&stored, "bands", &mut config.bands)?;

        let levels: Vec<u8> = match stored.get(snapshot::LEVELS_KEY) {
            Some(levels) => levels
                .split(',')
                .filter(|l| !l.is_empty())
                .map(|l| l.parse())
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| Error::InvalidMeta(format!("{} = {levels}", snapshot::LEVELS_KEY)))?,
            None => {
                let mut stmt = self
                    .conn
                    .prepare("SELECT DISTINCT level FROM minhash ORDER BY level")?;
                stmt.query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            }
        };
        if !levels.is_empty() {
            config.fingerprint_levels = levels.into_iter().filter_map(Level::from_u8).collect();
        }
        Ok(config)
    }

    fn check_meta(&self, config: &Config) -> Result<()> {
        let current = [
            ("normalizer_version", NORMALIZER_VERSION.to_string()),
            ("ident_mode", config.ident_mode.to_string()),
            ("kgram", config.kgram.to_string()),
            ("window", config.window.to_string()),
            ("minhash_size", config.minhash_size.to_string()),
            ("bands", config.bands.to_string()),
        ];
        let stored = self.meta()?;
        for (key, value) in &current {
            match stored.get(*key) {
                Some(s) if s != value => {
                    return Err(Error::Incompatible {
                        key: key.to_string(),
                        stored: s.clone(),
                        current: value.clone(),
                    });
                }
                Some(_) => {}
                None => {
                    self.conn.execute(
                        "INSERT INTO meta (key, value) VALUES (?1, ?2)",
                        params![key, value],
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Stores `files` as the content of package version `pkg`, replacing what
    /// was previously recorded for that version. Files already in the
    /// database (same content hash) are linked without storing them again.
    ///
    /// # Errors
    ///
    /// [`Error::Snapshot`] when this database is a snapshot, and [`Error::Sqlite`]
    /// when the files cannot be stored; nothing is stored then.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::{Config, Language, extract_bytes};
    /// use pelicode_db::{Database, PackageInfo};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let config = Config::default();
    /// let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// let file = extract_bytes("a.rb".as_ref(), Language::Ruby, b"def one\n  1\nend\n", &config).unwrap();
    /// let package = |version: &str| PackageInfo {
    ///     ecosystem: "rubygems".into(),
    ///     name: "one".into(),
    ///     version: version.into(),
    ///     ..PackageInfo::default()
    /// };
    ///
    /// let first = db.index(&package("1.0"), &[file.clone()]).unwrap();
    /// assert_eq!((first.files, first.new_files, first.functions), (1, 1, 1));
    ///
    /// // The same file in another version is linked, not stored again.
    /// let second = db.index(&package("1.1"), &[file]).unwrap();
    /// assert_eq!(second.new_files, 0);
    /// ```
    pub fn index(&mut self, pkg: &PackageInfo, files: &[FileFeatures]) -> Result<IndexSummary> {
        self.refuse_snapshot()?;
        let tx = self.conn.transaction()?;
        let version_id = upsert_version(&tx, pkg, "indexed", None)?;
        // The hashes of the files unlinked here and linked below may change
        // their package counts; remember them for an incremental refresh.
        let track = stats::pending_complete(&tx)?;
        if track {
            stats::mark_pending(&tx, version_id)?;
        }
        tx.execute(
            "DELETE FROM version_file WHERE package_version_id = ?1",
            [version_id],
        )?;

        let mut summary = IndexSummary {
            files: files.len(),
            ..IndexSummary::default()
        };
        for file in files {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM source_file WHERE content_sha256 = ?1",
                    [&file.content_sha256],
                    |r| r.get(0),
                )
                .optional()?;
            let file_id = match existing {
                Some(id) => id,
                None => {
                    summary.new_files += 1;
                    insert_file(&tx, file, &mut summary)?
                }
            };
            tx.execute(
                "INSERT OR REPLACE INTO version_file (package_version_id, source_file_id, path)
                 VALUES (?1, ?2, ?3)",
                params![version_id, file_id, file.path.to_string_lossy()],
            )?;
        }
        if track {
            stats::mark_pending(&tx, version_id)?;
        }
        // Lets `hash_stats_info` tell whether the statistics are out of date.
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('index_generation', '1')
             ON CONFLICT (key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
            [],
        )?;
        tx.commit()?;
        Ok(summary)
    }

    /// Status of a package version (`indexed` or `failed`), if recorded.
    ///
    /// # Errors
    ///
    /// [`Error::Sqlite`] when the database cannot be read.
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
    /// # db.index(&package, &[file]).unwrap();
    /// assert_eq!(db.version_status("pypi", "calc", "1.0").unwrap().as_deref(), Some("indexed"));
    /// assert_eq!(db.version_status("pypi", "calc", "2.0").unwrap(), None);
    /// ```
    pub fn version_status(
        &self,
        ecosystem: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT pv.status FROM package_version pv
                 JOIN package p ON p.id = pv.package_id
                 JOIN ecosystem e ON e.id = p.ecosystem_id
                 WHERE e.name = ?1 AND p.name = ?2 AND pv.version = ?3",
                params![ecosystem, name, version],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Records that fetching or indexing `pkg` failed, so that it can be retried later.
    /// Files already linked to the version are kept.
    ///
    /// # Errors
    ///
    /// [`Error::Snapshot`] when this database is a snapshot, and [`Error::Sqlite`]
    /// when the failure cannot be stored.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::{Database, PackageInfo};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let mut db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// let package = PackageInfo {
    ///     ecosystem: "pypi".into(),
    ///     name: "broken".into(),
    ///     version: "0.1".into(),
    ///     ..PackageInfo::default()
    /// };
    /// db.record_failure(&package, "digest mismatch").unwrap();
    /// assert_eq!(db.version_status("pypi", "broken", "0.1").unwrap().as_deref(), Some("failed"));
    /// ```
    pub fn record_failure(&mut self, pkg: &PackageInfo, error: &str) -> Result<()> {
        self.refuse_snapshot()?;
        let tx = self.conn.transaction()?;
        upsert_version(&tx, pkg, "failed", Some(error))?;
        tx.commit()?;
        Ok(())
    }

    /// Number of rows of the main tables.
    ///
    /// # Errors
    ///
    /// [`Error::Sqlite`] when the database cannot be read.
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
    /// # db.index(&package, &[file]).unwrap();
    /// let stats = db.stats().unwrap();
    /// assert_eq!((stats.packages, stats.versions, stats.functions), (1, 1, 1));
    /// ```
    pub fn stats(&self) -> Result<Stats> {
        let count = |table: &str| -> Result<usize> {
            let n: i64 =
                self.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
            Ok(n as usize)
        };
        let indexed: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT function_id) FROM minhash", [], |r| {
                    r.get(0)
                })?;
        Ok(Stats {
            ecosystems: count("ecosystem")?,
            packages: count("package")?,
            versions: count("package_version")?,
            source_files: count("source_file")?,
            functions: count("function")?,
            indexed_functions: indexed as usize,
        })
    }

    /// Every occurrence of every stored function, ordered by package and location.
    ///
    /// # Errors
    ///
    /// [`Error::Sqlite`] when the database cannot be read.
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
    /// # db.index(&package, &[file]).unwrap();
    /// let rows = db.functions().unwrap();
    /// assert_eq!(rows[0].name, "total");
    /// assert_eq!(rows[0].path, "src/calc.py");
    /// assert_eq!(rows[0].license.as_deref(), Some("MIT"));
    /// ```
    pub fn functions(&self) -> Result<Vec<FunctionRow>> {
        let mut hashes: HashMap<i64, BTreeMap<Level, String>> = HashMap::new();
        let mut stmt = self
            .conn
            .prepare("SELECT function_id, level, hash FROM exact_hash")?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let level: u8 = row.get(1)?;
            let hash: Vec<u8> = row.get(2)?;
            if let Some(level) = Level::from_u8(level) {
                hashes
                    .entry(row.get(0)?)
                    .or_default()
                    .insert(level, pelicode_core::hash::to_hex(&hash));
            }
        }

        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT function_id FROM minhash")?;
        let indexed: HashSet<i64> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT f.id, e.name, p.name, pv.version, pv.license_spdx, vf.path, sf.language,
                    f.name, f.kind, f.start_line, f.end_line, f.start_column, f.end_column,
                    f.start_byte, f.end_byte, f.parent_function_id,
                    f.token_count, f.node_count, f.depth
             FROM function f
             JOIN source_file sf ON sf.id = f.source_file_id
             JOIN version_file vf ON vf.source_file_id = sf.id
             JOIN package_version pv ON pv.id = vf.package_version_id
             JOIN package p ON p.id = pv.package_id
             JOIN ecosystem e ON e.id = p.ecosystem_id
             ORDER BY e.name, p.name, pv.version, vf.path, f.start_line, f.id",
        )?;
        let rows = stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            Ok(FunctionRow {
                id,
                ecosystem: r.get(1)?,
                package: r.get(2)?,
                version: r.get(3)?,
                license: r.get(4)?,
                path: r.get(5)?,
                language: r.get(6)?,
                name: r.get(7)?,
                kind: r.get(8)?,
                start_line: r.get::<_, i64>(9)? as usize,
                end_line: r.get::<_, i64>(10)? as usize,
                start_column: r.get::<_, i64>(11)? as usize,
                end_column: r.get::<_, i64>(12)? as usize,
                start_byte: r.get::<_, i64>(13)? as usize,
                end_byte: r.get::<_, i64>(14)? as usize,
                parent_id: r.get(15)?,
                token_count: r.get::<_, i64>(16)? as usize,
                node_count: r.get::<_, i64>(17)? as usize,
                depth: r.get::<_, i64>(18)? as usize,
                indexed: indexed.contains(&id),
                exact_hashes: hashes.get(&id).cloned().unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn upsert_version(
    tx: &Transaction<'_>,
    pkg: &PackageInfo,
    status: &str,
    error: Option<&str>,
) -> Result<i64> {
    tx.execute(
        "INSERT OR IGNORE INTO ecosystem (name) VALUES (?1)",
        [&pkg.ecosystem],
    )?;
    let ecosystem_id: i64 = tx.query_row(
        "SELECT id FROM ecosystem WHERE name = ?1",
        [&pkg.ecosystem],
        |r| r.get(0),
    )?;
    let package_id: i64 = tx.query_row(
        "INSERT INTO package (ecosystem_id, name, repository_url) VALUES (?1, ?2, ?3)
         ON CONFLICT (ecosystem_id, name)
         DO UPDATE SET repository_url = COALESCE(excluded.repository_url, repository_url)
         RETURNING id",
        params![ecosystem_id, pkg.name, pkg.repository_url],
        |r| r.get(0),
    )?;
    let version_id: i64 = tx.query_row(
        "INSERT INTO package_version
             (package_id, version, published_at, license_spdx, archive_url, archive_sha256,
              crawled_at, status, error)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?7, ?8)
         ON CONFLICT (package_id, version) DO UPDATE SET
             published_at   = COALESCE(excluded.published_at, published_at),
             license_spdx   = COALESCE(excluded.license_spdx, license_spdx),
             archive_url    = COALESCE(excluded.archive_url, archive_url),
             archive_sha256 = COALESCE(excluded.archive_sha256, archive_sha256),
             crawled_at     = excluded.crawled_at,
             status         = excluded.status,
             error          = excluded.error
         RETURNING id",
        params![
            package_id,
            pkg.version,
            pkg.published_at,
            pkg.license_spdx,
            pkg.archive_url,
            pkg.archive_sha256,
            status,
            error
        ],
        |r| r.get(0),
    )?;
    Ok(version_id)
}

fn insert_file(
    tx: &Transaction<'_>,
    file: &FileFeatures,
    summary: &mut IndexSummary,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO source_file (content_sha256, language, function_count) VALUES (?1, ?2, ?3)",
        params![
            file.content_sha256,
            file.language.name(),
            file.functions.len() as i64
        ],
    )?;
    let file_id = tx.last_insert_rowid();

    let mut insert_function = tx.prepare_cached(
        "INSERT INTO function (source_file_id, parent_function_id, name, kind, start_line,
                               end_line, start_column, end_column, start_byte, end_byte,
                               token_count, node_count, depth)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?;
    let mut insert_exact =
        tx.prepare_cached("INSERT INTO exact_hash (function_id, level, hash) VALUES (?1, ?2, ?3)")?;
    let mut insert_fingerprint = tx.prepare_cached(
        "INSERT INTO fingerprint (function_id, level, hash, position, start_line, end_line)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    let mut insert_minhash = tx.prepare_cached(
        "INSERT INTO minhash (function_id, level, signature) VALUES (?1, ?2, ?3)",
    )?;
    let mut insert_bucket = tx.prepare_cached(
        "INSERT INTO lsh_bucket (level, band, bucket_hash, function_id) VALUES (?1, ?2, ?3, ?4)",
    )?;

    // Parents precede their children, so their ids are always known.
    let mut ids: Vec<i64> = Vec::with_capacity(file.functions.len());
    for function in &file.functions {
        let parent_id = function.parent.map(|i| ids[i]);
        insert_function.execute(params![
            file_id,
            parent_id,
            function.name,
            function.kind,
            function.start_line as i64,
            function.end_line as i64,
            function.start_column as i64,
            function.end_column as i64,
            function.start_byte as i64,
            function.end_byte as i64,
            function.token_count as i64,
            function.node_count as i64,
            function.depth as i64,
        ])?;
        let function_id = tx.last_insert_rowid();
        ids.push(function_id);
        summary.functions += 1;
        if function.is_indexed() {
            summary.indexed_functions += 1;
        }

        for (level, hash) in &function.exact_hashes {
            insert_exact.execute(params![function_id, level.as_u8(), &hash.0[..]])?;
        }
        for (level, features) in &function.features {
            let level = level.as_u8();
            for fp in &features.fingerprints {
                insert_fingerprint.execute(params![
                    function_id,
                    level,
                    fp.hash as i64,
                    fp.position,
                    fp.start_line,
                    fp.end_line
                ])?;
            }
            let signature: Vec<u8> = features
                .minhash
                .iter()
                .flat_map(|h| h.to_le_bytes())
                .collect();
            insert_minhash.execute(params![function_id, level, signature])?;
            for (band, bucket) in features.lsh_buckets.iter().enumerate() {
                insert_bucket.execute(params![level, band as i64, *bucket as i64, function_id])?;
            }
        }
    }
    Ok(file_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pelicode_core::{Language, extract_source};

    const SRC: &str = r#"
class A {
    int sum(int[] values) {
        int total = 0;
        for (int v : values) { total += v; }
        return total;
    }
    Runnable r() { return () -> System.out.println("x"); }
}
"#;

    fn file(path: &str) -> FileFeatures {
        let config = Config {
            min_tokens: 10,
            ..Config::default()
        };
        FileFeatures {
            path: path.into(),
            language: Language::Java,
            content_sha256: pelicode_core::extract::sha256_hex(SRC.as_bytes()),
            functions: extract_source(Language::Java, SRC.as_bytes(), &config).unwrap(),
        }
    }

    fn pkg(version: &str) -> PackageInfo {
        PackageInfo {
            ecosystem: "local".into(),
            name: "demo".into(),
            version: version.into(),
            license_spdx: Some("MIT".into()),
            ..PackageInfo::default()
        }
    }

    #[test]
    fn indexes_and_deduplicates_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("t.db"), &Config::default()).unwrap();

        let first = db.index(&pkg("1.0"), &[file("A.java")]).unwrap();
        assert_eq!(first.new_files, 1);
        assert_eq!(first.functions, 3);

        // Same content in another version and path: stored once, linked twice.
        let second = db.index(&pkg("2.0"), &[file("src/A.java")]).unwrap();
        assert_eq!(second.new_files, 0);

        // Re-indexing a version replaces its file list.
        db.index(&pkg("2.0"), &[file("src/A.java")]).unwrap();

        let stats = db.stats().unwrap();
        assert_eq!(stats.versions, 2);
        assert_eq!(stats.source_files, 1);
        assert_eq!(stats.functions, 3);

        let rows = db.functions().unwrap();
        assert_eq!(rows.len(), 6);
        let lambda = rows.iter().find(|r| r.kind == "lambda_expression").unwrap();
        assert!(lambda.parent_id.is_some());
        assert_eq!(rows[0].exact_hashes.len(), 4);
        let sum = rows.iter().find(|r| r.name == "sum").unwrap();
        assert_eq!(
            &SRC[sum.start_byte..sum.end_byte],
            "int sum(int[] values) {\n        int total = 0;\n        for (int v : values) { total += v; }\n        return total;\n    }"
        );
        assert_eq!((sum.start_column, sum.end_column), (5, 5));
        let lines: (i64, i64) = db
            .conn
            .query_row(
                "SELECT MIN(start_line), MAX(end_line) FROM fingerprint WHERE function_id = ?1",
                [sum.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        // Winnowing need not select the last k-gram, so the range may end before the function.
        assert!(
            lines.0 >= 3 && lines.1 <= 7 && lines.0 < lines.1,
            "{lines:?}"
        );
        assert_eq!(rows[0].license.as_deref(), Some("MIT"));
    }

    #[test]
    fn records_crawl_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("t.db"), &Config::default()).unwrap();
        assert_eq!(db.version_status("local", "demo", "1.0").unwrap(), None);
        db.record_failure(&pkg("1.0"), "network down").unwrap();
        assert_eq!(
            db.version_status("local", "demo", "1.0")
                .unwrap()
                .as_deref(),
            Some("failed")
        );
        db.index(&pkg("1.0"), &[file("A.java")]).unwrap();
        assert_eq!(
            db.version_status("local", "demo", "1.0")
                .unwrap()
                .as_deref(),
            Some("indexed")
        );
        let error: Option<String> = db
            .conn
            .query_row("SELECT error FROM package_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(error, None);
    }

    #[test]
    fn counts_packages_per_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("t.db"), &Config::default()).unwrap();
        let other = |name: &str| PackageInfo {
            name: name.into(),
            ..pkg("1.0")
        };
        // Two versions of one package count once.
        db.index(&pkg("1.0"), &[file("A.java")]).unwrap();
        db.index(&pkg("2.0"), &[file("A.java")]).unwrap();
        assert!(db.hash_stats_info().unwrap().stale);

        let info = db.refresh_hash_stats().unwrap();
        assert!(!info.stale);
        assert_eq!(info.packages, 1);
        assert_eq!(info.common_exact_hashes, 0);

        db.index(&other("copy"), &[file("B.java")]).unwrap();
        assert!(db.hash_stats_info().unwrap().stale);
        let info = db.refresh_hash_stats().unwrap();
        assert_eq!(info.packages, 2);
        assert!(info.common_exact_hashes > 0);
        assert!(info.common_fingerprints > 0);

        let sum = db
            .functions()
            .unwrap()
            .into_iter()
            .find(|r| r.name == "sum")
            .unwrap();
        let hash = pelicode_core::Hash128::from_hex(&sum.exact_hashes[&Level::N1]).unwrap();
        assert_eq!(db.exact_package_count(Level::N1, &hash).unwrap(), 2);
        assert_eq!(
            db.exact_package_count(Level::N1, &pelicode_core::Hash128([0; 16]))
                .unwrap(),
            1
        );
        let common = db.common_exact_hashes(Level::N1, 0, 10).unwrap();
        assert!(
            common
                .iter()
                .all(|c| c.package_count == 2 && c.sample.is_some())
        );
        assert!(!db.common_fingerprints(Level::N1, 10).unwrap().is_empty());
    }

    #[test]
    fn clean_keeps_files_linked_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("t.db"), &Config::default()).unwrap();
        db.index(&pkg("1.0"), &[file("A.java")]).unwrap();
        db.index(&pkg("2.0"), &[file("A.java")]).unwrap();
        // 1.0 no longer has the file, but 2.0 still does.
        db.index(&pkg("1.0"), &[]).unwrap();
        assert_eq!(db.unreferenced().unwrap(), CleanSummary::default());
        assert_eq!(db.clean().unwrap(), CleanSummary::default());
        assert_eq!(db.stats().unwrap().functions, 3);

        db.index(&pkg("2.0"), &[]).unwrap();
        let removed = db.clean().unwrap();
        assert_eq!((removed.files, removed.functions), (1, 3));
        let left: i64 = db
            .conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM exact_hash) + (SELECT COUNT(*) FROM fingerprint)
                      + (SELECT COUNT(*) FROM minhash) + (SELECT COUNT(*) FROM lsh_bucket)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        db.vacuum().unwrap();
    }

    fn stats_tables(db: &Database) -> Vec<(String, u8, Vec<u8>, i64, i64)> {
        let mut rows = Vec::new();
        for (table, hash) in [
            ("exact_stats", "hash"),
            ("fingerprint_stats", "CAST(hash AS BLOB)"),
        ] {
            let mut stmt = db
                .conn
                .prepare(&format!(
                    "SELECT level, {hash}, package_count, function_count FROM {table}
                     ORDER BY level, hash"
                ))
                .unwrap();
            let found = stmt
                .query_map([], |r| {
                    Ok((
                        table.to_string(),
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                    ))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.extend(found);
        }
        rows
    }

    #[test]
    fn incremental_statistics_equal_a_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let mut db = Database::open(dir.path().join("t.db"), &Config::default()).unwrap();
        let config = Config {
            min_tokens: 10,
            ..Config::default()
        };
        let source = |n: usize| {
            let body: String = (0..n)
                .map(|i| format!("        total += values[{i}] * {i};\n"))
                .collect();
            format!(
                "class C{n} {{\n    int f(int[] values) {{\n        int total = 0;\n{body}        return total;\n    }}\n}}\n"
            )
        };
        let file = |name: &str, n: usize| {
            let src = source(n);
            FileFeatures {
                path: name.into(),
                language: Language::Java,
                content_sha256: pelicode_core::extract::sha256_hex(src.as_bytes()),
                functions: extract_source(Language::Java, src.as_bytes(), &config).unwrap(),
            }
        };
        let package = |name: &str, version: &str| PackageInfo {
            ecosystem: "maven".into(),
            name: name.into(),
            version: version.into(),
            ..PackageInfo::default()
        };

        db.index(&package("a", "1"), &[file("A.java", 3), file("B.java", 4)])
            .unwrap();
        db.index(&package("b", "1"), &[file("A.java", 3)]).unwrap();
        db.index(&package("e", "1"), &[file("E.java", 7)]).unwrap();
        db.index(&package("f", "1"), &[file("E.java", 7)]).unwrap();
        let info = db.refresh_hash_stats().unwrap();
        assert!(!info.incremental || info.pending_hashes == 0);

        // New packages, shared files, a version whose files are replaced
        // (unlinking A from package a), and a cleanup in between.
        db.index(&package("c", "1"), &[file("A.java", 3), file("C.java", 5)])
            .unwrap();
        db.index(&package("a", "1"), &[file("B.java", 4)]).unwrap();
        db.index(&package("a", "2"), &[file("C.java", 5)]).unwrap();
        db.index(&package("d", "1"), &[file("B.java", 4), file("D.java", 6)])
            .unwrap();
        // E is no longer shared: its hashes must leave the statistics.
        db.index(&package("e", "1"), &[]).unwrap();
        db.clean().unwrap();

        let info = db.hash_stats_info().unwrap();
        assert!(info.incremental && info.stale && info.pending_hashes > 0);
        db.refresh_hash_stats().unwrap();
        let incremental = stats_tables(&db);
        assert_eq!(db.hash_stats_info().unwrap().pending_hashes, 0);

        db.rebuild_hash_stats().unwrap();
        assert_eq!(incremental, stats_tables(&db));
        assert!(!incremental.is_empty());
        let shared = |db: &Database, path: &str| -> i64 {
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM exact_stats s
                     JOIN exact_hash h ON h.level = s.level AND h.hash = s.hash
                     JOIN function f ON f.id = h.function_id
                     JOIN version_file vf ON vf.source_file_id = f.source_file_id
                     WHERE vf.path = ?1",
                    [path],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(shared(&db, "E.java"), 0);
        assert!(shared(&db, "A.java") > 0);
    }

    #[test]
    fn migrates_version_4_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let downgrade = |stale: bool| {
            let mut db = Database::open(&path, &Config::default()).unwrap();
            db.index(&pkg("1.0"), &[file("A.java")]).unwrap();
            db.rebuild_hash_stats().unwrap();
            if stale {
                db.index(&pkg("2.0"), &[file("A.java")]).unwrap();
            }
            db.conn
                .execute_batch(
                    "UPDATE meta SET value = '4' WHERE key = 'schema_version';
                     DELETE FROM meta WHERE key = 'stats_pending_complete';
                     DROP TABLE stats_pending_exact;
                     DROP TABLE stats_pending_fingerprint;",
                )
                .unwrap();
        };

        downgrade(false);
        let db = Database::open(&path, &Config::default()).unwrap();
        assert_eq!(
            db.meta().unwrap()["schema_version"],
            SCHEMA_VERSION.to_string()
        );
        assert!(db.hash_stats_info().unwrap().incremental);
        drop(db);

        std::fs::remove_file(&path).unwrap();
        downgrade(true);
        let mut db = Database::open(&path, &Config::default()).unwrap();
        // Something was indexed after the statistics without being tracked.
        assert!(!db.hash_stats_info().unwrap().incremental);
        db.refresh_hash_stats().unwrap();
        assert!(db.hash_stats_info().unwrap().incremental);
    }

    #[test]
    fn rejects_incompatible_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        Database::open(&path, &Config::default()).unwrap();
        let other = Config {
            kgram: 7,
            ..Config::default()
        };
        assert!(matches!(
            Database::open(&path, &other),
            Err(Error::Incompatible { .. })
        ));
    }
}
