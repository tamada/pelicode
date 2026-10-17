//! crates.io: releases via the API, sources from the `.crate` archive.
//!
//! The crates.io data access policy (<https://crates.io/data-access>) allows
//! at most one API request per second, with a User-Agent that identifies the
//! application and preferably a way to contact its operator. Archives come
//! from static.crates.io, which has no rate limit.

use serde_json::Value;

use crate::archive::ArchiveKind;
use crate::http::Client;
use crate::{Digest, Error, Release};
use pelicode_db::PackageInfo;

/// Name of crates.io in the database.
pub const ECOSYSTEM: &str = "crates";

const API: &str = "https://crates.io/api/v1";
const STATIC: &str = "https://static.crates.io/crates";
const METADATA_LIMIT: u64 = 64 * 1024 * 1024;

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The release `version` of the crate `name`, or the version crates.io shows by default.
///
/// # Errors
///
/// [`Error::NotFound`] when the package or version does not exist,
/// [`Error::NoSources`] when it has no source archive, [`Error::Metadata`] when
/// the response cannot be understood, and [`Error::Http`] for network failures.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use pelicode_crawl::{Client, crates};
///
/// let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
/// let release = crates::resolve(&client, "uuid", Some("1.28.0"))?;
/// println!("{} {}", release.package.name, release.package.version);
/// # Ok::<(), pelicode_crawl::Error>(())
/// ```
pub fn resolve(client: &Client, name: &str, version: Option<&str>) -> Result<Release, Error> {
    if !valid_name(name) {
        return Err(Error::InvalidPackage(name.into()));
    }
    let Some(version) = version else {
        // One request: the crate's response lists its recent versions.
        let url = format!("{API}/crates/{name}");
        let json = get_json(client, &url)?;
        let latest =
            latest_version(&json).ok_or_else(|| Error::Metadata(format!("{url}: no version")))?;
        if let Some(v) = json["versions"]
            .as_array()
            .and_then(|vs| vs.iter().find(|v| v["num"] == latest.as_str()))
        {
            let mut release = parse_release(&serde_json::json!({ "version": v }))?;
            if release.package.repository_url.is_none() {
                release.package.repository_url =
                    json["crate"]["repository"].as_str().map(String::from);
            }
            return Ok(release);
        }
        return resolve(client, name, Some(&latest));
    };
    parse_release(&get_json(
        client,
        &format!("{API}/crates/{name}/{version}"),
    )?)
}

/// The `n` most downloaded crates of all time.
///
/// # Errors
///
/// [`Error::Metadata`] when the response cannot be understood, and [`Error::Http`]
/// for network failures.
///
/// # Examples
///
/// ```no_run
/// use std::time::Duration;
/// use pelicode_crawl::{Client, crates};
///
/// let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
/// let top = crates::top_packages(&client, 100)?;
/// # Ok::<(), pelicode_crawl::Error>(())
/// ```
pub fn top_packages(client: &Client, n: usize) -> Result<Vec<String>, Error> {
    let mut names = Vec::with_capacity(n);
    let mut page = 1;
    while names.len() < n {
        let url = format!("{API}/crates?sort=downloads&per_page=100&page={page}");
        let json = get_json(client, &url)?;
        let crates = json["crates"]
            .as_array()
            .ok_or_else(|| Error::Metadata(format!("{url}: no crates")))?;
        if crates.is_empty() {
            break;
        }
        names.extend(
            crates
                .iter()
                .filter_map(|c| c["name"].as_str())
                .map(String::from),
        );
        page += 1;
    }
    names.truncate(n);
    Ok(names)
}

fn get_json(client: &Client, url: &str) -> Result<Value, Error> {
    serde_json::from_slice(&client.get(url, METADATA_LIMIT)?.body)
        .map_err(|e| Error::Metadata(format!("{url}: {e}")))
}

/// The version crates.io shows by default: the latest stable one, if any.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::crates::latest_version;
///
/// let json = serde_json::json!({"crate": {"default_version": "1.0.0", "max_version": "2.0.0-rc.1"}});
/// assert_eq!(latest_version(&json).as_deref(), Some("1.0.0"));
/// ```
pub fn latest_version(json: &Value) -> Option<String> {
    let krate = &json["crate"];
    ["default_version", "max_stable_version", "max_version"]
        .iter()
        .find_map(|key| krate[key].as_str().filter(|v| !v.is_empty()))
        .map(String::from)
}

/// A release from the response of `/crates/<name>/<version>`.
///
/// # Errors
///
/// [`Error::Metadata`] when the crate name or the version number is missing, and
/// [`Error::NoSources`] when the version is yanked.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::crates::parse_release;
///
/// let json = serde_json::json!({"version": {
///     "crate": "uuid", "num": "1.28.0", "license": "Apache-2.0 OR MIT", "checksum": "7cc1"
/// }});
/// let release = parse_release(&json).unwrap();
/// assert_eq!(release.package.archive_url.as_deref(), Some("https://static.crates.io/crates/uuid/uuid-1.28.0.crate"));
/// assert_eq!(release.package.license_spdx.as_deref(), Some("Apache-2.0 OR MIT"));
/// ```
pub fn parse_release(json: &Value) -> Result<Release, Error> {
    let v = &json["version"];
    let field = |key: &str| v[key].as_str().filter(|s| !s.is_empty()).map(String::from);
    let name = field("crate").ok_or_else(|| Error::Metadata("no crate name".into()))?;
    let version = field("num").ok_or_else(|| Error::Metadata("no version number".into()))?;
    if v["yanked"] == Value::Bool(true) {
        return Err(Error::NoSources(format!("{name} {version} is yanked")));
    }
    Ok(Release {
        package: PackageInfo {
            ecosystem: ECOSYSTEM.into(),
            archive_url: Some(format!("{STATIC}/{name}/{name}-{version}.crate")),
            name,
            version,
            // Already an SPDX expression; the old `/` separator means OR.
            license_spdx: field("license").map(|l| l.replace('/', " OR ")),
            repository_url: field("repository"),
            published_at: field("created_at"),
            archive_sha256: None,
        },
        archive_kind: ArchiveKind::TarGz,
        // Entries are named `<name>-<version>/...`.
        strip_components: 1,
        digest: field("checksum").map(|d| Digest::Sha256(d.to_ascii_lowercase())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_version() {
        let json = serde_json::json!({"version": {
            "crate": "uuid", "num": "1.28.0", "checksum": "7CC1",
            "license": "Apache-2.0 OR MIT", "created_at": "2026-10-10T00:02:40.455859Z",
            "yanked": false, "repository": "https://github.com/uuid-rs/uuid"
        }});
        let r = parse_release(&json).unwrap();
        assert_eq!(r.package.name, "uuid");
        assert_eq!(
            r.package.archive_url.as_deref(),
            Some("https://static.crates.io/crates/uuid/uuid-1.28.0.crate")
        );
        assert_eq!(r.package.license_spdx.as_deref(), Some("Apache-2.0 OR MIT"));
        assert_eq!(r.digest, Some(Digest::Sha256("7cc1".into())));
        assert_eq!(r.strip_components, 1);
    }

    #[test]
    fn rejects_yanked_versions_and_odd_names() {
        let json = serde_json::json!({"version": {"crate": "x", "num": "1.0.0", "yanked": true}});
        assert!(matches!(parse_release(&json), Err(Error::NoSources(_))));
        assert!(!valid_name("../x"));
        assert!(valid_name("serde_json"));
    }

    #[test]
    fn prefers_the_default_version() {
        let json = serde_json::json!({"crate": {
            "default_version": "1.0.0", "max_stable_version": "1.0.0", "max_version": "2.0.0-rc.1"
        }});
        assert_eq!(latest_version(&json).as_deref(), Some("1.0.0"));
        let old = serde_json::json!({"crate": {"max_stable_version": "", "max_version": "0.1.0"}});
        assert_eq!(latest_version(&old).as_deref(), Some("0.1.0"));
    }

    #[test]
    fn converts_legacy_license_separators() {
        let json = serde_json::json!({"version": {"crate": "x", "num": "1.0.0", "license": "MIT/Apache-2.0"}});
        assert_eq!(
            parse_release(&json)
                .unwrap()
                .package
                .license_spdx
                .as_deref(),
            Some("MIT OR Apache-2.0")
        );
    }
}
