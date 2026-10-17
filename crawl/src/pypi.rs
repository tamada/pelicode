//! PyPI: releases via the JSON API, sources from the sdist (or a wheel).

use serde_json::Value;

use crate::archive::ArchiveKind;
use crate::http::Client;
use crate::{Digest, Error, Release};
use pelicode_db::PackageInfo;

pub const ECOSYSTEM: &str = "pypi";

const API: &str = "https://pypi.org/pypi";
/// Download ranking maintained by Hugo van Kemenade, built from the public PyPI BigQuery data.
const TOP_PACKAGES: &str = "https://hugovk.github.io/top-pypi-packages/top-pypi-packages.min.json";
const METADATA_LIMIT: u64 = 64 * 1024 * 1024;

/// PEP 503 normalized name.
pub fn normalize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut separator = false;
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            separator = true;
        } else {
            if separator && !out.is_empty() {
                out.push('-');
            }
            separator = false;
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// The release `version` of `name`, or its latest release.
pub fn resolve(client: &Client, name: &str, version: Option<&str>) -> Result<Release, Error> {
    let url = match version {
        Some(v) => format!("{API}/{name}/{v}/json"),
        None => format!("{API}/{name}/json"),
    };
    let response = client.get(&url, METADATA_LIMIT)?;
    let json: Value = serde_json::from_slice(&response.body)
        .map_err(|e| Error::Metadata(format!("{url}: {e}")))?;
    parse_release(&json)
}

/// The `n` most downloaded projects of the last 30 days.
pub fn top_packages(client: &Client, n: usize) -> Result<Vec<String>, Error> {
    let response = client.get(TOP_PACKAGES, METADATA_LIMIT)?;
    let json: Value = serde_json::from_slice(&response.body)
        .map_err(|e| Error::Metadata(format!("{TOP_PACKAGES}: {e}")))?;
    let rows = json["rows"]
        .as_array()
        .ok_or_else(|| Error::Metadata(format!("{TOP_PACKAGES}: no rows")))?;
    Ok(rows
        .iter()
        .filter_map(|r| r["project"].as_str())
        .take(n)
        .map(String::from)
        .collect())
}

pub fn parse_release(json: &Value) -> Result<Release, Error> {
    let info = &json["info"];
    let name = info["name"]
        .as_str()
        .ok_or_else(|| Error::Metadata("no info.name".into()))?;
    let version = info["version"]
        .as_str()
        .ok_or_else(|| Error::Metadata("no info.version".into()))?;
    let files: Vec<&Value> = json["urls"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|f| f["yanked"] != Value::Bool(true))
                .collect()
        })
        .unwrap_or_default();

    let filename = |f: &Value| f["filename"].as_str().unwrap_or("").to_string();
    let sdist = files.iter().find(|f| {
        f["packagetype"] == "sdist" && ArchiveKind::from_file_name(&filename(f)).is_some()
    });
    let wheels: Vec<&&Value> = files
        .iter()
        .filter(|f| f["packagetype"] == "bdist_wheel")
        .collect();
    let wheel = wheels
        .iter()
        .find(|f| filename(f).ends_with("-none-any.whl"))
        .or(wheels.first())
        .copied();
    let (file, strip_top) = match (sdist, wheel) {
        (Some(f), _) => (*f, true),
        (None, Some(f)) => (*f, false),
        (None, None) => {
            return Err(Error::NoSources(format!(
                "{name} {version}: no sdist or wheel"
            )));
        }
    };
    let archive_kind = ArchiveKind::from_file_name(&filename(file))
        .ok_or_else(|| Error::NoSources(filename(file)))?;
    let url = file["url"]
        .as_str()
        .ok_or_else(|| Error::Metadata("file without url".into()))?;

    Ok(Release {
        package: PackageInfo {
            ecosystem: ECOSYSTEM.into(),
            name: normalize(name),
            version: version.into(),
            license_spdx: license(info),
            repository_url: repository(info),
            published_at: file["upload_time_iso_8601"].as_str().map(String::from),
            archive_url: Some(url.into()),
            archive_sha256: None,
        },
        archive_kind,
        strip_top,
        digest: file["digests"]["sha256"]
            .as_str()
            .map(|d| Digest::Sha256(d.to_ascii_lowercase())),
    })
}

/// License as an SPDX expression where possible; otherwise the declared text when short.
fn license(info: &Value) -> Option<String> {
    if let Some(expr) = info["license_expression"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        return Some(expr.into());
    }
    let classifiers: Vec<&str> = info["classifiers"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for classifier in classifiers {
        if let Some(name) = classifier.strip_prefix("License :: OSI Approved :: ") {
            return Some(spdx_of_classifier(name).unwrap_or(name).into());
        }
    }
    info["license"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 100 && !s.contains('\n'))
        .map(String::from)
}

fn spdx_of_classifier(name: &str) -> Option<&'static str> {
    Some(match name {
        "MIT License" => "MIT",
        "Apache Software License" => "Apache-2.0",
        "BSD License" => "BSD-3-Clause",
        "ISC License (ISCL)" => "ISC",
        "Python Software Foundation License" => "PSF-2.0",
        "Mozilla Public License 2.0 (MPL 2.0)" => "MPL-2.0",
        "GNU General Public License v2 (GPLv2)" => "GPL-2.0-only",
        "GNU General Public License v3 (GPLv3)" => "GPL-3.0-only",
        "GNU Lesser General Public License v3 (LGPLv3)" => "LGPL-3.0-only",
        "The Unlicense (Unlicense)" => "Unlicense",
        _ => return None,
    })
}

fn repository(info: &Value) -> Option<String> {
    let urls = info["project_urls"].as_object()?;
    [
        "Source",
        "Source Code",
        "Repository",
        "Code",
        "GitHub",
        "Homepage",
    ]
    .iter()
    .find_map(|key| {
        urls.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .and_then(|(_, v)| v.as_str())
    })
    .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize("Foo.Bar__baz"), "foo-bar-baz");
        assert_eq!(normalize("requests"), "requests");
    }

    #[test]
    fn prefers_sdist() {
        let json = serde_json::json!({
            "info": {
                "name": "Demo_Pkg",
                "version": "1.2.0",
                "license": "",
                "classifiers": ["License :: OSI Approved :: MIT License"],
                "project_urls": {"Homepage": "https://example.org", "Source": "https://github.com/x/demo"}
            },
            "urls": [
                {"packagetype": "bdist_wheel", "filename": "demo_pkg-1.2.0-py3-none-any.whl",
                 "url": "https://files/demo.whl", "digests": {"sha256": "AA"}, "yanked": false},
                {"packagetype": "sdist", "filename": "demo_pkg-1.2.0.tar.gz",
                 "url": "https://files/demo.tar.gz", "digests": {"sha256": "BB"},
                 "upload_time_iso_8601": "2024-01-02T03:04:05.000000Z", "yanked": false}
            ]
        });
        let r = parse_release(&json).unwrap();
        assert_eq!(r.package.name, "demo-pkg");
        assert_eq!(r.package.license_spdx.as_deref(), Some("MIT"));
        assert_eq!(
            r.package.repository_url.as_deref(),
            Some("https://github.com/x/demo")
        );
        assert_eq!(
            r.package.archive_url.as_deref(),
            Some("https://files/demo.tar.gz")
        );
        assert_eq!(r.archive_kind, ArchiveKind::TarGz);
        assert!(r.strip_top);
        assert_eq!(r.digest, Some(Digest::Sha256("bb".into())));
    }

    #[test]
    fn falls_back_to_a_wheel() {
        let json = serde_json::json!({
            "info": {"name": "w", "version": "1", "license_expression": "Apache-2.0"},
            "urls": [
                {"packagetype": "bdist_wheel", "filename": "w-1-cp312-cp312-linux_x86_64.whl",
                 "url": "https://files/linux.whl", "digests": {}},
                {"packagetype": "bdist_wheel", "filename": "w-1-py3-none-any.whl",
                 "url": "https://files/any.whl", "digests": {}}
            ]
        });
        let r = parse_release(&json).unwrap();
        assert_eq!(
            r.package.archive_url.as_deref(),
            Some("https://files/any.whl")
        );
        assert_eq!(r.package.license_spdx.as_deref(), Some("Apache-2.0"));
        assert!(!r.strip_top);
    }
}
