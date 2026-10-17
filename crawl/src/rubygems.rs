//! RubyGems: releases via the JSON API, sources from the `.gem` file.

use serde_json::Value;

use crate::archive::ArchiveKind;
use crate::http::Client;
use crate::{Digest, Error, Release};
use pelicode_db::PackageInfo;

pub const ECOSYSTEM: &str = "rubygems";

const API: &str = "https://rubygems.org/api";
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;

pub fn resolve(client: &Client, name: &str, version: Option<&str>) -> Result<Release, Error> {
    if name.is_empty() || name.contains('/') {
        return Err(Error::InvalidPackage(name.into()));
    }
    let url = match version {
        Some(v) => format!("{API}/v2/rubygems/{name}/versions/{v}.json"),
        None => format!("{API}/v1/gems/{name}.json"),
    };
    let json: Value = serde_json::from_slice(&client.get(&url, METADATA_LIMIT)?.body)
        .map_err(|e| Error::Metadata(format!("{url}: {e}")))?;
    parse_release(&json)
}

pub fn parse_release(json: &Value) -> Result<Release, Error> {
    let field = |key: &str| {
        json[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let name = field("name").ok_or_else(|| Error::Metadata("no name".into()))?;
    let version = field("version").ok_or_else(|| Error::Metadata("no version".into()))?;
    if json["yanked"] == Value::Bool(true) {
        return Err(Error::NoSources(format!("{name} {version} is yanked")));
    }
    let gem = field("gem_uri").unwrap_or_else(|| match field("platform").filter(|p| p != "ruby") {
        Some(platform) => format!("https://rubygems.org/gems/{name}-{version}-{platform}.gem"),
        None => format!("https://rubygems.org/gems/{name}-{version}.gem"),
    });
    let licenses: Vec<&str> = json["licenses"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Ok(Release {
        package: PackageInfo {
            ecosystem: ECOSYSTEM.into(),
            name,
            version,
            license_spdx: (!licenses.is_empty()).then(|| licenses.join(" OR ")),
            repository_url: field("source_code_uri")
                .or_else(|| {
                    json["metadata"]["source_code_uri"]
                        .as_str()
                        .map(String::from)
                })
                .or_else(|| field("homepage_uri")),
            published_at: field("version_created_at").or_else(|| field("created_at")),
            archive_url: Some(gem),
            archive_sha256: None,
        },
        archive_kind: ArchiveKind::Gem,
        strip_components: 0,
        digest: field("sha").map(|d| Digest::Sha256(d.to_ascii_lowercase())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gem_metadata() {
        let json = serde_json::json!({
            "name": "rack", "version": "3.1.8", "platform": "ruby",
            "licenses": ["MIT"], "sha": "D3FB",
            "created_at": "2024-10-14T01:53:03.127Z",
            "version_created_at": "2024-10-14T01:53:03.127Z",
            "source_code_uri": "https://github.com/rack/rack",
            "homepage_uri": "https://example.org",
            "gem_uri": "https://rubygems.org/gems/rack-3.1.8.gem"
        });
        let r = parse_release(&json).unwrap();
        assert_eq!(r.package.license_spdx.as_deref(), Some("MIT"));
        assert_eq!(
            r.package.repository_url.as_deref(),
            Some("https://github.com/rack/rack")
        );
        assert_eq!(
            r.package.published_at.as_deref(),
            Some("2024-10-14T01:53:03.127Z")
        );
        assert_eq!(r.digest, Some(Digest::Sha256("d3fb".into())));
        assert_eq!(r.archive_kind, ArchiveKind::Gem);
    }

    #[test]
    fn builds_platform_gem_urls() {
        let json = serde_json::json!({
            "name": "nokogiri", "version": "1.16.0", "platform": "x86_64-linux",
            "licenses": [], "homepage_uri": "https://nokogiri.org"
        });
        let r = parse_release(&json).unwrap();
        assert_eq!(
            r.package.archive_url.as_deref(),
            Some("https://rubygems.org/gems/nokogiri-1.16.0-x86_64-linux.gem")
        );
        assert_eq!(r.package.license_spdx, None);
        assert_eq!(
            r.package.repository_url.as_deref(),
            Some("https://nokogiri.org")
        );
    }
}
