//! Crawler: fetches package sources from public repositories and indexes
//! their functions (spec §5).

pub mod archive;
pub mod crates;
pub mod go;
pub mod http;
pub mod license;
pub mod maven;
pub mod pypi;
pub mod rubygems;

use std::fmt;
use std::str::FromStr;

use pelicode_core::{Config, FileFeatures, extract_bytes};
use pelicode_db::{Database, IndexSummary, PackageInfo};
use rayon::prelude::*;
use sha1::{Digest as _, Sha1};

pub use http::Client;

/// Archives larger than this are not downloaded.
pub const MAX_ARCHIVE_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Http(String, #[source] ureq::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid metadata: {0}")]
    Metadata(String),
    #[error("no source archive: {0}")]
    NoSources(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch { expected: String, actual: String },
    #[error("invalid package: {0}")]
    InvalidPackage(String),
    #[error(transparent)]
    Db(#[from] pelicode_db::Error),
    #[error(transparent)]
    Core(#[from] pelicode_core::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    PyPI,
    Maven,
    Go,
    RubyGems,
    Crates,
}

impl Ecosystem {
    pub fn name(self) -> &'static str {
        match self {
            Ecosystem::PyPI => pypi::ECOSYSTEM,
            Ecosystem::Maven => maven::ECOSYSTEM,
            Ecosystem::Go => go::ECOSYSTEM,
            Ecosystem::RubyGems => rubygems::ECOSYSTEM,
            Ecosystem::Crates => crates::ECOSYSTEM,
        }
    }
}

impl fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Ecosystem {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "pypi" => Ok(Ecosystem::PyPI),
            "maven" => Ok(Ecosystem::Maven),
            "go" => Ok(Ecosystem::Go),
            "rubygems" | "gem" => Ok(Ecosystem::RubyGems),
            "crates" | "crates.io" | "cargo" => Ok(Ecosystem::Crates),
            _ => Err(Error::InvalidPackage(format!("unknown ecosystem: {s}"))),
        }
    }
}

/// Expected digest of an archive, as published by the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Digest {
    Sha256(String),
    Sha1(String),
    /// Go checksum database `h1:` hash of the module zip.
    GoH1(String),
}

/// A package version whose sources can be fetched.
#[derive(Debug, Clone)]
pub struct Release {
    pub package: PackageInfo,
    pub archive_kind: archive::ArchiveKind,
    /// Leading path components to drop from archive entries
    /// (`name-1.0/` of an sdist, `module@version/` of a Go module).
    pub strip_components: usize,
    pub digest: Option<Digest>,
}

/// What happened to one package.
#[derive(Debug)]
pub enum Outcome {
    Indexed {
        package: PackageInfo,
        summary: IndexSummary,
        /// Source files that could not be parsed.
        failed_files: usize,
    },
    /// Already indexed; nothing fetched but metadata.
    Skipped { package: PackageInfo },
}

pub struct Crawler {
    client: Client,
    config: Config,
    force: bool,
}

impl Crawler {
    /// `force` re-indexes versions that are already in the database.
    pub fn new(client: Client, config: Config, force: bool) -> Crawler {
        Crawler {
            client,
            config,
            force,
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Fetches and indexes one package. `spec` is `name[@version]` for PyPI
    /// and `groupId:artifactId[:version]` (or `...@version`) for Maven.
    /// Failures after the version is known are recorded in the database.
    pub fn crawl(
        &self,
        db: &mut Database,
        ecosystem: Ecosystem,
        spec: &str,
    ) -> Result<Outcome, Error> {
        let release = self.resolve(ecosystem, spec)?;
        if !self.force
            && db
                .version_status(
                    &release.package.ecosystem,
                    &release.package.name,
                    &release.package.version,
                )?
                .as_deref()
                == Some("indexed")
        {
            return Ok(Outcome::Skipped {
                package: release.package,
            });
        }
        match self.fetch_and_index(db, release.clone()) {
            Ok(outcome) => Ok(outcome),
            Err(e) => {
                db.record_failure(&release.package, &e.to_string())?;
                Err(e)
            }
        }
    }

    fn resolve(&self, ecosystem: Ecosystem, spec: &str) -> Result<Release, Error> {
        let (name, version) = match spec.split_once('@').or_else(|| spec.split_once("==")) {
            Some((n, v)) => (n, Some(v)),
            None => (spec, None),
        };
        match ecosystem {
            Ecosystem::PyPI => pypi::resolve(&self.client, name, version),
            Ecosystem::Maven => {
                let (coords, v) = maven::Coordinates::parse(name)?;
                let version = version.map(String::from).or(v);
                maven::resolve(&self.client, &coords, version.as_deref())
            }
            Ecosystem::Go => go::resolve(&self.client, name, version),
            Ecosystem::RubyGems => rubygems::resolve(&self.client, name, version),
            Ecosystem::Crates => crates::resolve(&self.client, name, version),
        }
    }

    fn fetch_and_index(&self, db: &mut Database, release: Release) -> Result<Outcome, Error> {
        let mut package = release.package;
        let url = package
            .archive_url
            .clone()
            .ok_or_else(|| Error::NoSources(package.name.clone()))?;
        let response = self.client.get(&url, MAX_ARCHIVE_SIZE)?;
        let sha256 = pelicode_core::extract::sha256_hex(&response.body);
        if let Some(digest) = &release.digest {
            verify(digest, &response.body, &sha256)?;
        }
        package.archive_sha256 = Some(sha256);
        if package.published_at.is_none() {
            package.published_at = response.last_modified;
        }

        let contents = archive::read(
            release.archive_kind,
            &response.body,
            release.strip_components,
        )?;
        drop(response.body);
        if package.license_spdx.is_none() {
            package.license_spdx = license::identify_all(&contents.license_texts);
        }
        let config = &self.config;
        let results: Vec<Result<FileFeatures, pelicode_core::Error>> = contents
            .sources
            .par_iter()
            .map(|e| extract_bytes(&e.path, e.language, &e.content, config))
            .collect();
        let failed_files = results.iter().filter(|r| r.is_err()).count();
        let files: Vec<FileFeatures> = results.into_iter().filter_map(Result::ok).collect();
        let summary = db.index(&package, &files)?;
        Ok(Outcome::Indexed {
            package,
            summary,
            failed_files,
        })
    }
}

/// Checks a downloaded archive against the digest published by its repository.
fn verify(digest: &Digest, body: &[u8], sha256: &str) -> Result<(), Error> {
    let (expected, actual) = match digest {
        Digest::Sha256(expected) => (expected, sha256.to_string()),
        Digest::Sha1(expected) => (expected, pelicode_core::hash::to_hex(&Sha1::digest(body))),
        Digest::GoH1(expected) => (expected, go::dirhash_h1(body)?),
    };
    if *expected != actual {
        return Err(Error::DigestMismatch {
            expected: expected.clone(),
            actual,
        });
    }
    Ok(())
}
