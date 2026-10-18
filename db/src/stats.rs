//! Document frequencies of hashes: how many packages contain each exact hash
//! and fingerprint. Hashes found in many packages (getters, `equals`,
//! license headers turned into code, ...) are no evidence of copying.

use pelicode_core::{Hash128, Level};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::{Database, Result};

/// State of the statistics.
#[derive(Debug, Clone, Serialize)]
pub struct HashStatsInfo {
    /// Packages with at least one indexed file.
    pub packages: usize,
    /// When the statistics were last rebuilt.
    pub updated_at: Option<String>,
    /// Whether something was indexed after the last rebuild (or never rebuilt).
    pub stale: bool,
    /// Exact hashes found in two or more packages.
    pub common_exact_hashes: usize,
    /// Fingerprints found in two or more packages.
    pub common_fingerprints: usize,
    /// Whether [`Database::refresh_hash_stats`] can recount only the hashes of
    /// what was indexed since the last statistics. Not before the first
    /// statistics, nor for a database upgraded while they were out of date.
    pub incremental: bool,
    /// Hashes waiting to be recounted by an incremental refresh.
    pub pending_hashes: usize,
}

/// A hash shared by several packages, with one function that has it.
#[derive(Debug, Clone, Serialize)]
pub struct CommonHash {
    /// Normalization level of the hash.
    pub level: Level,
    /// The hash (hex).
    pub hash: String,
    /// Packages containing it.
    pub package_count: usize,
    /// Functions containing it.
    pub function_count: usize,
    /// `ecosystem:package@version path:start-end name` of one function with the hash.
    pub sample: Option<String>,
}

impl Database {
    /// Brings the document frequencies up to date with the functions currently
    /// linked to packages: incrementally when possible (see
    /// [`HashStatsInfo::incremental`]), otherwise by recounting everything.
    /// Both give the same counts.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read or written.
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
    /// # let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// # let package = |name: &str| PackageInfo {
    /// #     ecosystem: "pypi".into(), name: name.into(), version: "1.0".into(), ..PackageInfo::default()
    /// # };
    /// db.index(&package("calc"), &[file.clone()]).unwrap();
    /// assert!(db.hash_stats_info().unwrap().stale);
    ///
    /// // The first statistics are counted from scratch.
    /// assert!(!db.hash_stats_info().unwrap().incremental);
    /// let info = db.refresh_hash_stats().unwrap();
    /// assert!(!info.stale);
    /// assert_eq!(info.common_exact_hashes, 0);
    ///
    /// // Later ones recount only the hashes of what was indexed since.
    /// db.index(&package("copy"), &[file]).unwrap();
    /// assert!(db.hash_stats_info().unwrap().incremental);
    /// let info = db.refresh_hash_stats().unwrap();
    /// assert_eq!(info.packages, 2);
    /// assert!(info.common_exact_hashes > 0);
    /// ```
    pub fn refresh_hash_stats(&mut self) -> Result<HashStatsInfo> {
        if self.hash_stats_info()?.incremental {
            self.refresh_pending_hash_stats()
        } else {
            self.rebuild_hash_stats()
        }
    }

    /// Recounts the document frequencies of every hash, whatever was indexed
    /// since the last statistics.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read or written.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let mut db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// let info = db.rebuild_hash_stats().unwrap();
    /// assert!(info.updated_at.is_some());
    /// ```
    pub fn rebuild_hash_stats(&mut self) -> Result<HashStatsInfo> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DROP TABLE IF EXISTS temp.function_package;
             CREATE TEMP TABLE function_package AS
                 SELECT DISTINCT f.id AS function_id, pv.package_id AS package_id
                 FROM function f
                 JOIN version_file vf ON vf.source_file_id = f.source_file_id
                 JOIN package_version pv ON pv.id = vf.package_version_id;
             CREATE INDEX temp.function_package_function ON function_package(function_id);

             DELETE FROM exact_stats;
             INSERT INTO exact_stats (level, hash, package_count, function_count)
                 SELECT e.level, e.hash, COUNT(DISTINCT fp.package_id), COUNT(DISTINCT e.function_id)
                 FROM exact_hash e JOIN function_package fp ON fp.function_id = e.function_id
                 GROUP BY e.level, e.hash
                 HAVING COUNT(DISTINCT fp.package_id) >= 2;

             DELETE FROM fingerprint_stats;
             INSERT INTO fingerprint_stats (level, hash, package_count, function_count)
                 SELECT f.level, f.hash, COUNT(DISTINCT fp.package_id), COUNT(DISTINCT f.function_id)
                 FROM fingerprint f JOIN function_package fp ON fp.function_id = f.function_id
                 GROUP BY f.level, f.hash
                 HAVING COUNT(DISTINCT fp.package_id) >= 2;

             DROP TABLE temp.function_package;",
        )?;
        finish_refresh(&tx)?;
        tx.commit()?;
        self.hash_stats_info()
    }

    /// Recounts only the hashes of the files linked to or unlinked from a
    /// version since the last statistics; the counts of the other hashes
    /// cannot have changed.
    fn refresh_pending_hash_stats(&mut self) -> Result<HashStatsInfo> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM exact_stats
                 WHERE (level, hash) IN (SELECT level, hash FROM stats_pending_exact);
             INSERT INTO exact_stats (level, hash, package_count, function_count)
                 SELECT e.level, e.hash, COUNT(DISTINCT pv.package_id), COUNT(DISTINCT e.function_id)
                 FROM stats_pending_exact p
                 JOIN exact_hash e ON e.level = p.level AND e.hash = p.hash
                 JOIN function f ON f.id = e.function_id
                 JOIN version_file vf ON vf.source_file_id = f.source_file_id
                 JOIN package_version pv ON pv.id = vf.package_version_id
                 GROUP BY e.level, e.hash
                 HAVING COUNT(DISTINCT pv.package_id) >= 2;

             DELETE FROM fingerprint_stats
                 WHERE (level, hash) IN (SELECT level, hash FROM stats_pending_fingerprint);
             INSERT INTO fingerprint_stats (level, hash, package_count, function_count)
                 SELECT g.level, g.hash, COUNT(DISTINCT pv.package_id), COUNT(DISTINCT g.function_id)
                 FROM stats_pending_fingerprint p
                 JOIN fingerprint g ON g.level = p.level AND g.hash = p.hash
                 JOIN function f ON f.id = g.function_id
                 JOIN version_file vf ON vf.source_file_id = f.source_file_id
                 JOIN package_version pv ON pv.id = vf.package_version_id
                 GROUP BY g.level, g.hash
                 HAVING COUNT(DISTINCT pv.package_id) >= 2;",
        )?;
        finish_refresh(&tx)?;
        tx.commit()?;
        self.hash_stats_info()
    }

    /// State of the hash statistics, without rebuilding them.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// let info = db.hash_stats_info().unwrap();
    /// assert_eq!(info.updated_at, None);
    /// assert_eq!(info.packages, 0);
    /// ```
    pub fn hash_stats_info(&self) -> Result<HashStatsInfo> {
        let meta = self.meta()?;
        let count = |table: &str| -> Result<usize> {
            let n: i64 =
                self.conn
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
            Ok(n as usize)
        };
        let generation = meta.get("index_generation").map_or("0", String::as_str);
        let updated_at = meta.get("stats_updated_at").cloned();
        Ok(HashStatsInfo {
            packages: self.package_count()?,
            stale: meta.get("stats_generation").map(String::as_str) != Some(generation),
            common_exact_hashes: count("exact_stats")?,
            common_fingerprints: count("fingerprint_stats")?,
            incremental: updated_at.is_some()
                && meta.get(PENDING_COMPLETE).map(String::as_str) == Some("1"),
            pending_hashes: count("stats_pending_exact")? + count("stats_pending_fingerprint")?,
            updated_at,
        })
    }

    /// Packages with at least one indexed file.
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
    /// # let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// // The same function in two packages.
    /// for name in ["calc", "copy"] {
    ///     let package = PackageInfo {
    ///         ecosystem: "pypi".into(),
    ///         name: name.into(),
    ///         version: "1.0".into(),
    ///         ..PackageInfo::default()
    ///     };
    ///     db.index(&package, &[file.clone()]).unwrap();
    /// }
    /// assert_eq!(db.package_count().unwrap(), 2);
    /// ```
    pub fn package_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT pv.package_id)
             FROM version_file vf JOIN package_version pv ON pv.id = vf.package_version_id",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Packages containing a function with this exact hash (1 when not common).
    ///
    /// Counts as of the last [`Database::refresh_hash_stats`].
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
    /// # let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// // The same function in two packages.
    /// for name in ["calc", "copy"] {
    ///     let package = PackageInfo {
    ///         ecosystem: "pypi".into(),
    ///         name: name.into(),
    ///         version: "1.0".into(),
    ///         ..PackageInfo::default()
    ///     };
    ///     db.index(&package, &[file.clone()]).unwrap();
    /// }
    /// use pelicode_core::Level;
    ///
    /// db.refresh_hash_stats().unwrap();
    /// let hash = &file.functions[0].exact_hashes[&Level::N1];
    /// assert_eq!(db.exact_package_count(Level::N1, hash).unwrap(), 2);
    /// ```
    pub fn exact_package_count(&self, level: Level, hash: &Hash128) -> Result<usize> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT package_count FROM exact_stats WHERE level = ?1 AND hash = ?2",
        )?;
        let n: Option<i64> = stmt
            .query_row(params![level.as_u8(), &hash.0[..]], |r| r.get(0))
            .optional()?;
        Ok(n.unwrap_or(1) as usize)
    }

    /// Packages containing this fingerprint (1 when not common).
    ///
    /// Counts as of the last [`Database::refresh_hash_stats`].
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
    /// # let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// // The same function in two packages.
    /// for name in ["calc", "copy"] {
    ///     let package = PackageInfo {
    ///         ecosystem: "pypi".into(),
    ///         name: name.into(),
    ///         version: "1.0".into(),
    ///         ..PackageInfo::default()
    ///     };
    ///     db.index(&package, &[file.clone()]).unwrap();
    /// }
    /// use pelicode_core::Level;
    ///
    /// db.refresh_hash_stats().unwrap();
    /// let fingerprint = file.functions[0].features[&Level::N1].fingerprints[0].hash;
    /// assert_eq!(db.fingerprint_package_count(Level::N1, fingerprint).unwrap(), 2);
    /// assert_eq!(db.fingerprint_package_count(Level::N1, 42).unwrap(), 1);
    /// ```
    pub fn fingerprint_package_count(&self, level: Level, hash: u64) -> Result<usize> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT package_count FROM fingerprint_stats WHERE level = ?1 AND hash = ?2",
        )?;
        let n: Option<i64> = stmt
            .query_row(params![level.as_u8(), hash as i64], |r| r.get(0))
            .optional()?;
        Ok(n.unwrap_or(1) as usize)
    }

    /// The exact hashes at `level` of functions with at least `min_tokens`
    /// tokens, shared by the most packages.
    ///
    /// Counts as of the last [`Database::refresh_hash_stats`].
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
    /// # let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// // The same function in two packages.
    /// for name in ["calc", "copy"] {
    ///     let package = PackageInfo {
    ///         ecosystem: "pypi".into(),
    ///         name: name.into(),
    ///         version: "1.0".into(),
    ///         ..PackageInfo::default()
    ///     };
    ///     db.index(&package, &[file.clone()]).unwrap();
    /// }
    /// use pelicode_core::Level;
    ///
    /// db.refresh_hash_stats().unwrap();
    /// let common = db.common_exact_hashes(Level::N1, 0, 10).unwrap();
    /// assert_eq!(common[0].package_count, 2);
    /// assert!(common[0].sample.as_deref().unwrap().contains("total"));
    /// ```
    pub fn common_exact_hashes(
        &self,
        level: Level,
        min_tokens: usize,
        limit: usize,
    ) -> Result<Vec<CommonHash>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.hash, s.package_count, s.function_count FROM exact_stats s
             WHERE s.level = ?1 AND EXISTS (
                 SELECT 1 FROM exact_hash e JOIN function f ON f.id = e.function_id
                 WHERE e.level = s.level AND e.hash = s.hash AND f.token_count >= ?3)
             ORDER BY s.package_count DESC, s.function_count DESC LIMIT ?2",
        )?;
        let rows: Vec<(Vec<u8>, i64, i64)> = stmt
            .query_map(
                params![level.as_u8(), limit as i64, min_tokens as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(hash, packages, functions)| {
                let function: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT e.function_id FROM exact_hash e JOIN function f ON f.id = e.function_id
                         WHERE e.level = ?1 AND e.hash = ?2 AND f.token_count >= ?3 LIMIT 1",
                        params![level.as_u8(), &hash, min_tokens as i64],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(CommonHash {
                    level,
                    hash: pelicode_core::hash::to_hex(&hash),
                    package_count: packages as usize,
                    function_count: functions as usize,
                    sample: match function {
                        Some(id) => self.sample(id, None)?,
                        None => None,
                    },
                })
            })
            .collect()
    }

    /// The fingerprints at `level` shared by the most packages.
    ///
    /// Counts as of the last [`Database::refresh_hash_stats`].
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
    /// # let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// // The same function in two packages.
    /// for name in ["calc", "copy"] {
    ///     let package = PackageInfo {
    ///         ecosystem: "pypi".into(),
    ///         name: name.into(),
    ///         version: "1.0".into(),
    ///         ..PackageInfo::default()
    ///     };
    ///     db.index(&package, &[file.clone()]).unwrap();
    /// }
    /// use pelicode_core::Level;
    ///
    /// db.refresh_hash_stats().unwrap();
    /// let common = db.common_fingerprints(Level::N1, 3).unwrap();
    /// assert!(!common.is_empty() && common.len() <= 3);
    /// assert!(common.iter().all(|c| c.package_count == 2));
    /// ```
    pub fn common_fingerprints(&self, level: Level, limit: usize) -> Result<Vec<CommonHash>> {
        let mut stmt = self.conn.prepare(
            "SELECT hash, package_count, function_count FROM fingerprint_stats
             WHERE level = ?1 ORDER BY package_count DESC, function_count DESC LIMIT ?2",
        )?;
        let rows: Vec<(i64, i64, i64)> = stmt
            .query_map(params![level.as_u8(), limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows.into_iter()
            .map(|(hash, packages, functions)| {
                let hit: Option<(i64, u32, u32)> = self
                    .conn
                    .query_row(
                        "SELECT function_id, start_line, end_line FROM fingerprint
                         WHERE level = ?1 AND hash = ?2 LIMIT 1",
                        params![level.as_u8(), hash],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                Ok(CommonHash {
                    level,
                    hash: format!("{:016x}", hash as u64),
                    package_count: packages as usize,
                    function_count: functions as usize,
                    sample: match hit {
                        Some((id, start, end)) => self.sample(id, Some((start, end)))?,
                        None => None,
                    },
                })
            })
            .collect()
    }

    fn sample(&self, function_id: i64, lines: Option<(u32, u32)>) -> Result<Option<String>> {
        let info = self.function_info(function_id)?;
        let Some(o) = self.occurrences(function_id)?.into_iter().next() else {
            return Ok(None);
        };
        let (start, end) = lines.unwrap_or((info.start_line as u32, info.end_line as u32));
        Ok(Some(format!(
            "{}:{}@{} {}:{start}-{end} {}",
            o.ecosystem, o.package, o.version, o.path, info.name
        )))
    }
}

/// Key of `meta` telling that the pending tables hold every hash whose
/// package count may have changed since the last statistics.
const PENDING_COMPLETE: &str = "stats_pending_complete";

/// Records the end of a refresh: no hash is pending, the statistics match
/// the current index generation, and the pending tables are complete again.
fn finish_refresh(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM stats_pending_exact;
         DELETE FROM stats_pending_fingerprint;
         INSERT INTO meta (key, value)
             VALUES ('stats_generation',
                     COALESCE((SELECT value FROM meta WHERE key = 'index_generation'), '0'))
             ON CONFLICT (key) DO UPDATE SET value = excluded.value;
         INSERT INTO meta (key, value)
             VALUES ('stats_updated_at', strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (key) DO UPDATE SET value = excluded.value;",
    )?;
    set_pending_complete(conn, true)
}

/// Whether indexing must record pending hashes: only once statistics exist
/// and every change since them is recorded.
pub(crate) fn pending_complete(conn: &Connection) -> Result<bool> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            [PENDING_COMPLETE],
            |r| r.get(0),
        )
        .optional()?;
    Ok(value.as_deref() == Some("1"))
}

pub(crate) fn set_pending_complete(conn: &Connection, complete: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![PENDING_COMPLETE, if complete { "1" } else { "0" }],
    )?;
    Ok(())
}

/// Marks the hashes of the files currently linked to a package version as
/// pending.
pub(crate) fn mark_pending(conn: &Connection, version_id: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO stats_pending_exact (level, hash)
             SELECT e.level, e.hash
             FROM version_file vf
             JOIN function f ON f.source_file_id = vf.source_file_id
             JOIN exact_hash e ON e.function_id = f.id
             WHERE vf.package_version_id = ?1",
        [version_id],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO stats_pending_fingerprint (level, hash)
             SELECT g.level, g.hash
             FROM version_file vf
             JOIN function f ON f.source_file_id = vf.source_file_id
             JOIN fingerprint g ON g.function_id = f.id
             WHERE vf.package_version_id = ?1",
        [version_id],
    )?;
    Ok(())
}
