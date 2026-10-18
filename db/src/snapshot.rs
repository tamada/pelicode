//! Snapshots: copies of a database for checking only (specification §5.6).
//!
//! A snapshot keeps what `scan` reads: packages, files, functions, exact
//! hashes, fingerprints, LSH buckets and the hash statistics. It drops the
//! MinHash signatures, which only indexing and `export` use, and the
//! bookkeeping of incremental statistics. Nothing can be indexed into it.

use std::path::Path;

use serde::Serialize;

use crate::{Database, Error, Result};

/// Key of `meta` marking a snapshot, with the time it was made.
pub(crate) const SNAPSHOT_KEY: &str = "snapshot_created_at";
/// Key of `meta` recording the fingerprint levels, since a snapshot has no
/// MinHash signatures to read them from.
pub(crate) const LEVELS_KEY: &str = "fingerprint_levels";

/// What goes into a snapshot.
#[derive(Debug, Clone, Default)]
pub struct SnapshotOptions {
    /// Only these ecosystems; all when empty.
    pub ecosystems: Vec<String>,
    /// Only the latest indexed version of each package, by publication time
    /// (then crawl time).
    pub latest_only: bool,
}

/// What [`Database::snapshot`] wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SnapshotSummary {
    /// Packages in the snapshot.
    pub packages: usize,
    /// Package versions in the snapshot.
    pub versions: usize,
    /// Functions in the snapshot.
    pub functions: usize,
    /// Size of the snapshot file in bytes.
    pub bytes: u64,
}

impl Database {
    /// Writes a snapshot of this database to `out`, which must not exist.
    /// This database is not changed.
    ///
    /// The snapshot holds the indexed versions selected by `options`, with
    /// hash statistics recounted for them, in a single file (no write-ahead
    /// log). It can be opened like any database, but indexing into it fails
    /// with [`Error::Snapshot`].
    ///
    /// # Errors
    ///
    /// [`Error::Sqlite`] when `out` exists or cannot be written, or when this
    /// database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::{Config, Language, extract_bytes};
    /// use pelicode_db::{Database, Error, PackageInfo, SnapshotOptions};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let config = Config { min_tokens: 0, ..Config::default() };
    /// let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// let file = extract_bytes("a.py".as_ref(), Language::Python, b"def f(x):\n    return x + 1\n", &config).unwrap();
    /// let package = |ecosystem: &str, version: &str| PackageInfo {
    ///     ecosystem: ecosystem.into(),
    ///     name: "demo".into(),
    ///     version: version.into(),
    ///     ..PackageInfo::default()
    /// };
    /// db.index(&package("pypi", "1.0"), &[file.clone()]).unwrap();
    /// db.index(&package("pypi", "1.1"), &[file.clone()]).unwrap();
    /// db.index(&package("crates", "0.1.0"), &[file.clone()]).unwrap();
    ///
    /// let out = dir.path().join("snapshot.db");
    /// let options = SnapshotOptions { ecosystems: vec!["pypi".into()], latest_only: true };
    /// let summary = db.snapshot(&out, &options).unwrap();
    /// assert_eq!((summary.packages, summary.versions), (1, 1));
    ///
    /// let mut snapshot = Database::open(&out, &config).unwrap();
    /// assert!(snapshot.is_snapshot().unwrap());
    /// assert!(matches!(snapshot.index(&package("pypi", "1.2"), &[file]), Err(Error::Snapshot)));
    /// ```
    pub fn snapshot(
        &self,
        out: impl AsRef<Path>,
        options: &SnapshotOptions,
    ) -> Result<SnapshotSummary> {
        let out = out.as_ref();
        let levels = self.fingerprint_levels()?;
        self.conn
            .execute("VACUUM INTO ?1", [out.to_string_lossy().as_ref()])?;

        let mut copy = Database::open_unchecked(out)?;
        copy.select(options)?;
        copy.clean()?;
        copy.rebuild_hash_stats()?;
        copy.conn.execute_batch(
            "DELETE FROM minhash;
             DELETE FROM stats_pending_exact;
             DELETE FROM stats_pending_fingerprint;",
        )?;
        copy.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2), (?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            [LEVELS_KEY, levels.as_str(), SNAPSHOT_KEY],
        )?;
        let stats = copy.stats()?;
        copy.vacuum()?;
        copy.conn.pragma_update(None, "journal_mode", "DELETE")?;
        drop(copy);

        Ok(SnapshotSummary {
            packages: stats.packages,
            versions: stats.versions,
            functions: stats.functions,
            bytes: std::fs::metadata(out).map(|m| m.len()).unwrap_or(0),
        })
    }

    /// Whether this database is a snapshot made by [`Database::snapshot`].
    ///
    /// # Errors
    ///
    /// [`Error::Sqlite`] when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// assert!(!db.is_snapshot().unwrap());
    /// ```
    pub fn is_snapshot(&self) -> Result<bool> {
        Ok(self.meta()?.contains_key(SNAPSHOT_KEY))
    }

    pub(crate) fn refuse_snapshot(&self) -> Result<()> {
        if self.is_snapshot()? {
            return Err(Error::Snapshot);
        }
        Ok(())
    }

    /// The fingerprint levels, as a comma-separated list for `meta`.
    fn fingerprint_levels(&self) -> Result<String> {
        if let Some(levels) = self.meta()?.remove(LEVELS_KEY) {
            return Ok(levels);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT level FROM minhash ORDER BY level")?;
        let levels: Vec<String> = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .map(|l| l.map(|l| l.to_string()))
            .collect::<rusqlite::Result<_>>()?;
        Ok(levels.join(","))
    }

    /// Unlinks the versions `options` leave out, and removes them.
    fn select(&mut self, options: &SnapshotOptions) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DROP TABLE IF EXISTS temp.keep_version;
             CREATE TEMP TABLE keep_version (id INTEGER PRIMARY KEY);
             DROP TABLE IF EXISTS temp.keep_ecosystem;
             CREATE TEMP TABLE keep_ecosystem (name TEXT PRIMARY KEY);",
        )?;
        for ecosystem in &options.ecosystems {
            tx.execute(
                "INSERT OR IGNORE INTO temp.keep_ecosystem (name) VALUES (?1)",
                [ecosystem],
            )?;
        }
        let latest = if options.latest_only {
            "AND pv.id = (SELECT v.id FROM package_version v
                          WHERE v.package_id = pv.package_id AND v.status = 'indexed'
                          ORDER BY v.published_at IS NULL, v.published_at DESC,
                                   v.crawled_at DESC, v.id DESC
                          LIMIT 1)"
        } else {
            ""
        };
        let sql = format!(
            "INSERT INTO temp.keep_version (id)
             SELECT pv.id FROM package_version pv
             JOIN package p ON p.id = pv.package_id
             JOIN ecosystem e ON e.id = p.ecosystem_id
             WHERE pv.status = 'indexed'
               AND (NOT EXISTS (SELECT 1 FROM temp.keep_ecosystem)
                    OR e.name IN (SELECT name FROM temp.keep_ecosystem))
               {latest}"
        );
        tx.execute(&sql, [])?;
        tx.execute_batch(
            "DELETE FROM version_file
                 WHERE package_version_id NOT IN (SELECT id FROM temp.keep_version);
             DELETE FROM package_version WHERE id NOT IN (SELECT id FROM temp.keep_version);
             DELETE FROM package WHERE id NOT IN (SELECT package_id FROM package_version);
             DELETE FROM ecosystem WHERE id NOT IN (SELECT ecosystem_id FROM package);
             DROP TABLE temp.keep_version;
             DROP TABLE temp.keep_ecosystem;",
        )?;
        tx.commit()?;
        Ok(())
    }
}
