//! Crawler: fetches package sources from public repositories and indexes
//! their functions (specification §6), for
//! [pelicode](https://github.com/tamada/pelicode).
//!
//! # Repositories
//!
//! | [`Ecosystem`] | Package specification | Archive | Digest |
//! | --- | --- | --- | --- |
//! | PyPI ([`pypi`]) | `NAME[@VERSION]` | sdist, or a wheel when there is none | SHA-256 |
//! | Maven Central ([`maven`]) | `GROUP:ARTIFACT[:VERSION]` | `-sources.jar` | SHA-1 |
//! | Go module proxy ([`go`]) | `MODULE[@VERSION]` | module zip | `h1:` from sum.golang.org |
//! | RubyGems ([`rubygems`]) | `NAME[@VERSION]` | `.gem` | SHA-256 |
//! | crates.io ([`crates`]) | `NAME[@VERSION]` | `.crate` | SHA-256 |
//!
//! Without a version, the latest release is fetched.
//!
//! # Steps
//!
//! [`Crawler::crawl`] handles one package:
//!
//! 1. the repository adapter resolves the specification into a [`Release`]:
//!    the version, its license and repository, the archive URL and the
//!    digest published for it;
//! 2. a version already indexed is skipped ([`Outcome::Skipped`]) unless
//!    re-indexing is forced;
//! 3. the archive is downloaded (up to [`MAX_ARCHIVE_SIZE`]) and checked
//!    against the digest ([`Error::DigestMismatch`]);
//! 4. its source files are read in memory ([`archive`]), never unpacked to
//!    disk, and their functions are extracted in parallel;
//! 5. the functions are stored with their provenance ([`Outcome::Indexed`]).
//!
//! Failures after the version is known are recorded in the database, so the
//! next crawl can tell what is missing. For Go modules, whose metadata
//! declares no license, the license file is identified by [`license`].
//!
//! # Being polite
//!
//! The [`Client`] sends one request at a time with a delay between requests,
//! retries with backoff, and identifies itself in the `User-Agent`. Give it
//! a contact URL or e-mail address when crawling many packages; crates.io in
//! particular asks crawlers for one.
//!
//! # Examples
//!
//! ```no_run
//! use std::time::Duration;
//! use pelicode_core::Config;
//! use pelicode_crawl::{Client, Crawler, Ecosystem, Outcome};
//! use pelicode_db::Database;
//!
//! let config = Config::default();
//! let mut db = Database::open("pelicode.db", &config)?;
//! let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
//! let crawler = Crawler::new(client, config, false);
//!
//! let ecosystem: Ecosystem = "crates.io".parse()?;
//! for spec in ["uuid@1.11.0", "serde_json"] {
//!     match crawler.crawl(&mut db, ecosystem, spec)? {
//!         Outcome::Indexed { package, summary, .. } => {
//!             println!("{}@{}: {} function(s)", package.name, package.version, summary.functions)
//!         }
//!         Outcome::Skipped { package } => println!("{}@{}: already indexed", package.name, package.version),
//!     }
//! }
//! # Ok::<(), pelicode_crawl::Error>(())
//! ```
//!
//! To pick packages, [`pypi::top_packages`], [`crates::top_packages`] and
//! [`ecosystems::top_packages`] list the best-known ones of each repository.

#![warn(missing_docs)]

pub mod archive;
pub mod crates;
pub mod ecosystems;
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

/// Errors of crawling.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A request failed; holds the URL.
    #[error("{0}: {1}")]
    Http(String, #[source] ureq::Error),
    /// The package, version or file does not exist (HTTP 404); holds the URL.
    #[error("not found: {0}")]
    NotFound(String),
    /// The metadata of the repository could not be understood.
    #[error("invalid metadata: {0}")]
    Metadata(String),
    /// The release has no archive with source code, or was yanked.
    #[error("no source archive: {0}")]
    NoSources(String),
    /// The archive could not be read, or is too large.
    #[error("archive error: {0}")]
    Archive(String),
    /// The downloaded archive does not match the digest published by the repository.
    #[error("digest mismatch: expected {expected}, got {actual}")]
    DigestMismatch {
        /// Digest published by the repository.
        expected: String,
        /// Digest of the downloaded archive.
        actual: String,
    },
    /// The package name, the package specification or the ecosystem is invalid.
    #[error("invalid package: {0}")]
    InvalidPackage(String),
    /// An error of the database.
    #[error(transparent)]
    Db(#[from] pelicode_db::Error),
    /// An error of the feature extraction.
    #[error(transparent)]
    Core(#[from] pelicode_core::Error),
}

/// A public package repository.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::Ecosystem;
///
/// assert_eq!("crates.io".parse::<Ecosystem>().unwrap(), Ecosystem::Crates);
/// assert_eq!(Ecosystem::Crates.to_string(), "crates");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem {
    /// PyPI, the Python Package Index.
    PyPI,
    /// Maven Central.
    Maven,
    /// The Go module proxy (proxy.golang.org).
    Go,
    /// RubyGems.org.
    RubyGems,
    /// crates.io.
    Crates,
}

impl Ecosystem {
    /// Name of the ecosystem as recorded in the database.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_crawl::Ecosystem;
    ///
    /// assert_eq!(Ecosystem::PyPI.name(), "pypi");
    /// assert_eq!(Ecosystem::RubyGems.name(), "rubygems");
    /// ```
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
    /// SHA-256 of the archive (hex).
    Sha256(String),
    /// SHA-1 of the archive (hex), the only checksum most Maven Central
    /// artifacts have (`.sha256` and `.sha512` are optional and rare).
    Sha1(String),
    /// Go checksum database `h1:` hash of the module zip.
    GoH1(String),
}

/// A package version whose sources can be fetched.
#[derive(Debug, Clone)]
pub struct Release {
    /// The package version; `archive_sha256` is filled once downloaded.
    pub package: PackageInfo,
    /// Format of the source archive.
    pub archive_kind: archive::ArchiveKind,
    /// Leading path components to drop from archive entries
    /// (`name-1.0/` of an sdist, `module@version/` of a Go module).
    pub strip_components: usize,
    /// Digest to check the downloaded archive against.
    pub digest: Option<Digest>,
}

/// What happened to one package.
#[derive(Debug)]
pub enum Outcome {
    /// Fetched and indexed.
    Indexed {
        /// The indexed package version.
        package: PackageInfo,
        /// What was stored.
        summary: IndexSummary,
        /// Source files that could not be parsed.
        failed_files: usize,
    },
    /// Already indexed; nothing fetched but metadata.
    Skipped {
        /// The package version found in the database.
        package: PackageInfo,
    },
}

/// Fetches packages and indexes them into a database.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use pelicode_core::Config;
/// use pelicode_crawl::{Client, Crawler, Ecosystem, Outcome};
/// use pelicode_db::Database;
///
/// let config = Config::default();
/// let mut db = Database::open("pelicode.db", &config)?;
/// let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
/// let crawler = Crawler::new(client, config, false);
/// match crawler.crawl(&mut db, Ecosystem::PyPI, "six@1.17.0")? {
///     Outcome::Indexed { summary, .. } => println!("{} functions", summary.functions),
///     Outcome::Skipped { .. } => println!("already indexed"),
/// }
/// # Ok::<(), pelicode_crawl::Error>(())
/// ```
pub struct Crawler {
    client: Client,
    config: Config,
    force: bool,
}

impl Crawler {
    /// `force` re-indexes versions that are already in the database.
    ///
    /// `config` must match the database the packages are indexed into.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use pelicode_core::Config;
    /// use pelicode_crawl::{Client, Crawler};
    ///
    /// let crawler = Crawler::new(Client::new(None, Duration::from_secs(1)), Config::default(), false);
    /// ```
    pub fn new(client: Client, config: Config, force: bool) -> Crawler {
        Crawler {
            client,
            config,
            force,
        }
    }

    /// The HTTP client, to make other requests with the same pacing.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use pelicode_core::Config;
    /// use pelicode_crawl::{Client, Crawler, pypi};
    ///
    /// let crawler = Crawler::new(Client::new(None, Duration::from_secs(1)), Config::default(), false);
    /// let top = pypi::top_packages(crawler.client(), 10)?;
    /// # Ok::<(), pelicode_crawl::Error>(())
    /// ```
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Fetches and indexes one package. `spec` is `NAME[@VERSION]` for PyPI,
    /// RubyGems and crates.io, `GROUP:ARTIFACT[:VERSION]` (or `...@VERSION`)
    /// for Maven, and `MODULE[@VERSION]` for Go; without a version, the latest
    /// release is fetched. Failures after the version is known are recorded in
    /// the database.
    ///
    /// # Errors
    ///
    /// The errors of the repository adapter (unknown package, network failure,
    /// invalid metadata), [`Error::DigestMismatch`], [`Error::Archive`], and the
    /// errors of the database.
    ///
    /// # Examples
    ///
    /// See [`Crawler`].
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
        // SHA-1 is broken against collisions, but here it only detects corrupt
        // or truncated downloads: the archive and its checksum come from the
        // same server, so no checksum would stop that server from lying, and
        // replacing a published archive with another of the same SHA-1 needs a
        // second preimage. Authenticity would need the PGP signatures (`.asc`).
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
