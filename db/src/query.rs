//! Lookups used by the detector.

use pelicode_core::{Hash128, Level};
use rusqlite::params;
use serde::Serialize;

use crate::{Database, Result};

/// A stored function, without its occurrences.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionInfo {
    pub id: i64,
    pub language: String,
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub token_count: usize,
}

/// Where a stored function appears: a file of a package version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Occurrence {
    pub ecosystem: String,
    pub package: String,
    pub version: String,
    pub path: String,
    pub license: Option<String>,
    pub published_at: Option<String>,
    pub repository_url: Option<String>,
    pub archive_url: Option<String>,
    pub content_sha256: String,
}

/// A stored fingerprint with the source lines it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredFingerprint {
    pub hash: u64,
    pub start_line: u32,
    pub end_line: u32,
}

impl Database {
    /// Functions whose exact hash at `level` is `hash`.
    pub fn functions_by_exact_hash(&self, level: Level, hash: &Hash128) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT function_id FROM exact_hash WHERE level = ?1 AND hash = ?2")?;
        let ids = stmt.query_map(params![level.as_u8(), &hash.0[..]], |r| r.get(0))?;
        Ok(ids.collect::<rusqlite::Result<_>>()?)
    }

    /// Functions sharing LSH bucket `bucket` of `band` at `level`.
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
