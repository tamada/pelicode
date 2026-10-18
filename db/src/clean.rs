//! Removing data that is no longer referenced.
//!
//! Indexing a package version again replaces its list of files; a file that no
//! other version links to is then left behind with its functions and their
//! features. They never appear in results, since they have no occurrence, but
//! they take space.

use serde::Serialize;

use crate::{Database, Result};

/// What [`Database::clean`] removed, or would remove.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CleanSummary {
    /// Source files no package version links to.
    pub files: usize,
    /// Functions of those files.
    pub functions: usize,
}

const ORPHANS: &str = "
    DROP TABLE IF EXISTS temp.orphan_file;
    CREATE TEMP TABLE orphan_file AS
        SELECT id FROM source_file
        WHERE id NOT IN (SELECT source_file_id FROM version_file);
    DROP TABLE IF EXISTS temp.orphan_function;
    CREATE TEMP TABLE orphan_function AS
        SELECT id FROM function WHERE source_file_id IN (SELECT id FROM orphan_file);
";

impl Database {
    /// The files and functions [`Database::clean`] would remove, without removing them.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read.
    ///
    /// # Examples
    ///
    /// See [`Database::clean`].
    pub fn unreferenced(&self) -> Result<CleanSummary> {
        let count = |sql: &str| -> Result<usize> {
            let n: i64 = self.conn.query_row(sql, [], |r| r.get(0))?;
            Ok(n as usize)
        };
        Ok(CleanSummary {
            files: count(
                "SELECT COUNT(*) FROM source_file
                 WHERE id NOT IN (SELECT source_file_id FROM version_file)",
            )?,
            functions: count(
                "SELECT COUNT(*) FROM function WHERE source_file_id IN (
                     SELECT id FROM source_file
                     WHERE id NOT IN (SELECT source_file_id FROM version_file))",
            )?,
        })
    }

    /// Removes the source files no package version links to, with their
    /// functions and features, in one transaction. The hash statistics are
    /// not affected, since they count linked files only.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when the database cannot be read or written;
    /// nothing is removed then.
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
    /// let package = PackageInfo {
    ///     ecosystem: "local".into(),
    ///     name: "demo".into(),
    ///     version: "1.0".into(),
    ///     ..PackageInfo::default()
    /// };
    /// let file = |source: &[u8]| extract_bytes("a.py".as_ref(), Language::Python, source, &config).unwrap();
    ///
    /// db.index(&package, &[file(b"def f():\n    return 1\n")]).unwrap();
    /// // Indexing the same version again leaves the first file unreferenced.
    /// db.index(&package, &[file(b"def g():\n    return 2\n")]).unwrap();
    /// assert_eq!(db.unreferenced().unwrap().files, 1);
    ///
    /// let removed = db.clean().unwrap();
    /// assert_eq!((removed.files, removed.functions), (1, 1));
    /// assert_eq!(db.unreferenced().unwrap().files, 0);
    /// assert_eq!(db.stats().unwrap().functions, 1);
    /// ```
    pub fn clean(&mut self) -> Result<CleanSummary> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(ORPHANS)?;
        let count = |table: &str| -> Result<usize> {
            let n: i64 = tx.query_row(&format!("SELECT COUNT(*) FROM temp.{table}"), [], |r| {
                r.get(0)
            })?;
            Ok(n as usize)
        };
        let summary = CleanSummary {
            files: count("orphan_file")?,
            functions: count("orphan_function")?,
        };
        tx.execute_batch(
            "DELETE FROM lsh_bucket  WHERE function_id IN (SELECT id FROM temp.orphan_function);
             DELETE FROM minhash     WHERE function_id IN (SELECT id FROM temp.orphan_function);
             DELETE FROM fingerprint WHERE function_id IN (SELECT id FROM temp.orphan_function);
             DELETE FROM exact_hash  WHERE function_id IN (SELECT id FROM temp.orphan_function);
             DELETE FROM function    WHERE id IN (SELECT id FROM temp.orphan_function);
             DELETE FROM source_file WHERE id IN (SELECT id FROM temp.orphan_file);
             DROP TABLE temp.orphan_function;
             DROP TABLE temp.orphan_file;",
        )?;
        tx.commit()?;
        Ok(summary)
    }

    /// Gives the space of removed rows back to the file system (SQLite `VACUUM`),
    /// writing the write-ahead log into the database before and after.
    ///
    /// # Errors
    ///
    /// [`Error`](crate::Error) when another connection uses the database, or
    /// when there is not enough disk space for the temporary copy `VACUUM` makes.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// db.vacuum().unwrap();
    /// ```
    pub fn vacuum(&self) -> Result<()> {
        self.conn
            // In WAL mode, VACUUM writes the new pages to the log too.
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);",
            )?;
        Ok(())
    }
}
