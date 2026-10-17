//! Maven Central: versions from `maven-metadata.xml`, licenses from the POM
//! (and its parents), sources from `-sources.jar`.

use roxmltree::{Document, Node};

use crate::archive::ArchiveKind;
use crate::http::Client;
use crate::{Digest, Error, Release};
use pelicode_db::PackageInfo;

pub const ECOSYSTEM: &str = "maven";

const REPOSITORY: &str = "https://repo1.maven.org/maven2";
const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
/// How far `<parent>` POMs are followed to find a license.
const MAX_PARENTS: usize = 5;

/// `groupId:artifactId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coordinates {
    pub group: String,
    pub artifact: String,
}

impl Coordinates {
    /// Parses `group:artifact` or `group:artifact:version`.
    pub fn parse(s: &str) -> Result<(Coordinates, Option<String>), Error> {
        let parts: Vec<&str> = s.split(':').collect();
        let (group, artifact, version) = match parts.as_slice() {
            [g, a] => (g, a, None),
            [g, a, v] => (g, a, Some(v.to_string())),
            _ => {
                return Err(Error::InvalidPackage(format!(
                    "{s}: expected groupId:artifactId[:version]"
                )));
            }
        };
        if group.is_empty() || artifact.is_empty() {
            return Err(Error::InvalidPackage(s.into()));
        }
        Ok((
            Coordinates {
                group: group.to_string(),
                artifact: artifact.to_string(),
            },
            version,
        ))
    }

    fn base(&self) -> String {
        format!(
            "{REPOSITORY}/{}/{}",
            self.group.replace('.', "/"),
            self.artifact
        )
    }

    fn file(&self, version: &str, suffix: &str) -> String {
        format!(
            "{}/{version}/{}-{version}{suffix}",
            self.base(),
            self.artifact
        )
    }
}

pub fn resolve(
    client: &Client,
    coords: &Coordinates,
    version: Option<&str>,
) -> Result<Release, Error> {
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            let url = format!("{}/maven-metadata.xml", coords.base());
            latest_version(&text(&client.get(&url, METADATA_LIMIT)?.body, &url)?)
                .ok_or_else(|| Error::Metadata(format!("{url}: no release version")))?
        }
    };
    let pom = effective(client, coords, &version)?;

    let sources = coords.file(&version, "-sources.jar");
    let sha1_url = format!("{sources}.sha1");
    let digest = match client.get(&sha1_url, 1024) {
        Ok(r) => text(&r.body, &sha1_url)?
            .split_whitespace()
            .next()
            .map(|h| Digest::Sha1(h.to_ascii_lowercase())),
        Err(Error::NotFound(_)) => {
            return Err(Error::NoSources(format!(
                "{}:{}:{version}: no sources jar",
                coords.group, coords.artifact
            )));
        }
        Err(e) => return Err(e),
    };

    Ok(Release {
        package: PackageInfo {
            ecosystem: ECOSYSTEM.into(),
            name: format!("{}:{}", coords.group, coords.artifact),
            version,
            license_spdx: pom.license,
            repository_url: pom.repository,
            // Filled from the Last-Modified header of the archive.
            published_at: None,
            archive_url: Some(sources),
            archive_sha256: None,
        },
        archive_kind: ArchiveKind::Zip,
        strip_components: 0,
        digest,
    })
}

fn text(body: &[u8], url: &str) -> Result<String, Error> {
    String::from_utf8(body.to_vec()).map_err(|_| Error::Metadata(format!("{url}: not UTF-8")))
}

pub fn latest_version(metadata: &str) -> Option<String> {
    let doc = Document::parse(metadata).ok()?;
    let versioning = child(doc.root_element(), "versioning")?;
    ["release", "latest"]
        .iter()
        .find_map(|tag| child_text(versioning, tag))
        .or_else(|| {
            child(versioning, "versions")?
                .children()
                .filter(|n| n.has_tag_name("version"))
                .filter_map(|n| n.text())
                .next_back()
                .map(String::from)
        })
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PomInfo {
    pub license: Option<String>,
    pub repository: Option<String>,
    pub parent: Option<(Coordinates, String)>,
}

pub fn parse_pom(pom: &str) -> Result<PomInfo, Error> {
    let doc = Document::parse(pom).map_err(|e| Error::Metadata(format!("POM: {e}")))?;
    let project = doc.root_element();
    let license = child(project, "licenses").and_then(|licenses| {
        let names: Vec<String> = licenses
            .children()
            .filter(|n| n.has_tag_name("license"))
            .filter_map(|l| child_text(l, "name").or_else(|| child_text(l, "url")))
            .map(|name| spdx_of_license_name(&name).map_or(name, String::from))
            .collect();
        (!names.is_empty()).then(|| names.join(" OR "))
    });
    let repository = child(project, "scm")
        .and_then(|scm| child_text(scm, "url"))
        .or_else(|| child_text(project, "url"))
        .filter(|url| !url.contains("${"));
    let parent = child(project, "parent").and_then(|p| {
        Some((
            Coordinates {
                group: child_text(p, "groupId")?,
                artifact: child_text(p, "artifactId")?,
            },
            child_text(p, "version")?,
        ))
    });
    Ok(PomInfo {
        license,
        repository,
        parent,
    })
}

/// POM information with the license and repository inherited from parents when missing.
fn effective(client: &Client, coords: &Coordinates, version: &str) -> Result<PomInfo, Error> {
    let url = coords.file(version, ".pom");
    let mut info = parse_pom(&text(&client.get(&url, METADATA_LIMIT)?.body, &url)?)?;
    let mut next = info.parent.clone();
    for _ in 0..MAX_PARENTS {
        if info.license.is_some() && info.repository.is_some() {
            break;
        }
        let Some((parent, parent_version)) = next else {
            break;
        };
        let url = parent.file(&parent_version, ".pom");
        let Ok(response) = client.get(&url, METADATA_LIMIT) else {
            break;
        };
        let parent_info = parse_pom(&text(&response.body, &url)?)?;
        info.license = info.license.or(parent_info.license);
        info.repository = info.repository.or(parent_info.repository);
        next = parent_info.parent;
    }
    Ok(info)
}

fn spdx_of_license_name(name: &str) -> Option<&'static str> {
    let n = name.to_ascii_lowercase();
    let has = |s: &str| n.contains(s);
    let word = |w: &str| {
        n.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|t| t == w)
    };
    Some(if has("apache") && has("2.0") {
        "Apache-2.0"
    } else if word("mit") {
        "MIT"
    } else if has("eclipse public license") && has("2.0") {
        "EPL-2.0"
    } else if has("eclipse public license") && has("1.0") {
        "EPL-1.0"
    } else if has("bsd") && has("3") {
        "BSD-3-Clause"
    } else if has("bsd") && has("2") {
        "BSD-2-Clause"
    } else if has("lesser general public license") && has("2.1") {
        "LGPL-2.1-only"
    } else if has("mozilla public license") && has("2.0") {
        "MPL-2.0"
    } else {
        return None;
    })
}

fn child<'a, 'i>(node: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.tag_name().name() == tag)
}

fn child_text(node: Node<'_, '_>, tag: &str) -> Option<String> {
    child(node, tag)
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_coordinates() {
        let (c, v) = Coordinates::parse("org.apache.commons:commons-lang3:3.14.0").unwrap();
        assert_eq!(c.group, "org.apache.commons");
        assert_eq!(v.as_deref(), Some("3.14.0"));
        assert_eq!(
            c.file("3.14.0", "-sources.jar"),
            "https://repo1.maven.org/maven2/org/apache/commons/commons-lang3/3.14.0/commons-lang3-3.14.0-sources.jar"
        );
        assert!(Coordinates::parse("nogroup").is_err());
    }

    #[test]
    fn reads_release_version() {
        let xml = r#"<metadata><groupId>g</groupId><versioning>
            <latest>2.0-SNAPSHOT</latest><release>1.9</release>
            <versions><version>1.8</version><version>1.9</version></versions>
        </versioning></metadata>"#;
        assert_eq!(latest_version(xml).as_deref(), Some("1.9"));
        let xml = "<metadata><versioning><versions><version>1</version><version>2</version></versions></versioning></metadata>";
        assert_eq!(latest_version(xml).as_deref(), Some("2"));
    }

    #[test]
    fn parses_pom_with_namespace() {
        let pom = r#"<?xml version="1.0"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <parent><groupId>org.apache.commons</groupId><artifactId>commons-parent</artifactId><version>64</version></parent>
  <url>https://commons.apache.org/${project.artifactId}</url>
  <licenses><license><name>Apache License, Version 2.0</name></license></licenses>
  <scm><url>https://gitbox.apache.org/repos/asf/commons-lang.git</url></scm>
</project>"#;
        let info = parse_pom(pom).unwrap();
        assert_eq!(info.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(
            info.repository.as_deref(),
            Some("https://gitbox.apache.org/repos/asf/commons-lang.git")
        );
        assert_eq!(info.parent.unwrap().1, "64");

        let pom = "<project><url>${x}</url></project>";
        assert_eq!(parse_pom(pom).unwrap(), PomInfo::default());
    }
}
