//! Go modules: versions and zips from the module proxy, checksums from the
//! checksum database (sum.golang.org).

use std::io::{Cursor, Read};

use base64::Engine as _;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::archive::ArchiveKind;
use crate::http::Client;
use crate::{Digest, Error, Release};
use pelicode_db::PackageInfo;

/// Name of the Go module proxy in the database.
pub const ECOSYSTEM: &str = "go";

const PROXY: &str = "https://proxy.golang.org";
const SUMDB: &str = "https://sum.golang.org";
const METADATA_LIMIT: u64 = 1024 * 1024;

/// Module path or version with upper-case letters escaped as `!` + lower case,
/// as the module proxy protocol requires.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::go::escape;
///
/// assert_eq!(escape("github.com/BurntSushi/toml"), "github.com/!burnt!sushi/toml");
/// ```
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('!');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The release `version` of `module`, or its latest release, with the `h1:` hash
/// from the checksum database (sum.golang.org) to check the module zip against.
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
/// use pelicode_crawl::{Client, go};
///
/// let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
/// let release = go::resolve(&client, "github.com/google/uuid", Some("v1.6.0"))?;
/// println!("{} {}", release.package.name, release.package.version);
/// # Ok::<(), pelicode_crawl::Error>(())
/// ```
pub fn resolve(client: &Client, module: &str, version: Option<&str>) -> Result<Release, Error> {
    if module.is_empty() || module.contains("..") || module.starts_with('/') {
        return Err(Error::InvalidPackage(module.into()));
    }
    let path = escape(module);
    let info_url = match version {
        Some(v) => format!("{PROXY}/{path}/@v/{}.info", escape(v)),
        None => format!("{PROXY}/{path}/@latest"),
    };
    let info: Value = serde_json::from_slice(&client.get(&info_url, METADATA_LIMIT)?.body)
        .map_err(|e| Error::Metadata(format!("{info_url}: {e}")))?;
    let version = info["Version"]
        .as_str()
        .ok_or_else(|| Error::Metadata(format!("{info_url}: no Version")))?
        .to_string();

    let lookup = format!("{SUMDB}/lookup/{path}@{}", escape(&version));
    let sums = String::from_utf8_lossy(&client.get(&lookup, METADATA_LIMIT)?.body).into_owned();
    let h1 = parse_lookup(&sums, module, &version)
        .ok_or_else(|| Error::Metadata(format!("{lookup}: no h1 checksum")))?;

    Ok(Release {
        package: PackageInfo {
            ecosystem: ECOSYSTEM.into(),
            name: module.into(),
            version: version.clone(),
            // Go modules declare no license; it is identified from LICENSE files.
            license_spdx: None,
            repository_url: info["Origin"]["URL"]
                .as_str()
                .map(String::from)
                .or_else(|| repository_of(module)),
            published_at: info["Time"].as_str().map(String::from),
            archive_url: Some(format!("{PROXY}/{path}/@v/{}.zip", escape(&version))),
            archive_sha256: None,
        },
        archive_kind: ArchiveKind::Zip,
        // Entries are named `<module>@<version>/...`.
        strip_components: module.split('/').count(),
        digest: Some(Digest::GoH1(h1)),
    })
}

/// The `h1:` hash of the module zip in a checksum database lookup response.
///
/// # Examples
///
/// ```
/// use pelicode_crawl::go::parse_lookup;
///
/// let body = "22152757\n\
///     github.com/google/uuid v1.6.0 h1:NIvaJDMOsjHA8n1jAhLSgzrAzy1Hgr+hNrb57e+94F0=\n\
///     github.com/google/uuid v1.6.0/go.mod h1:TIyPZe4MgqvfeYDBFedMoGGpEw/LqOeaOT+nhxU+yHo=\n";
/// assert_eq!(
///     parse_lookup(body, "github.com/google/uuid", "v1.6.0").as_deref(),
///     Some("h1:NIvaJDMOsjHA8n1jAhLSgzrAzy1Hgr+hNrb57e+94F0=")
/// );
/// ```
pub fn parse_lookup(body: &str, module: &str, version: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            [m, v, h] if *m == module && *v == version && h.starts_with("h1:") => {
                Some(h.to_string())
            }
            _ => None,
        }
    })
}

/// Repository of a module hosted on a well-known forge.
fn repository_of(module: &str) -> Option<String> {
    let parts: Vec<&str> = module.split('/').collect();
    match parts.as_slice() {
        [
            host @ ("github.com" | "gitlab.com" | "bitbucket.org"),
            owner,
            repo,
            ..,
        ] => Some(format!("https://{host}/{owner}/{repo}")),
        _ => None,
    }
}

/// The `h1:` directory hash of a module zip (golang.org/x/mod/sumdb/dirhash.Hash1):
/// SHA-256 over the sorted lines `"<sha256 hex>  <file name>\n"` of every file, base64-encoded.
///
/// # Errors
///
/// [`Error::Archive`] when the zip cannot be read, or a file name contains a newline.
///
/// # Examples
///
/// ```
/// use std::io::Write;
/// use pelicode_crawl::go::dirhash_h1;
///
/// let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
/// zip.start_file("example.com/m@v1.0.0/go.mod", zip::write::SimpleFileOptions::default()).unwrap();
/// zip.write_all(b"module example.com/m\n").unwrap();
/// let bytes = zip.finish().unwrap().into_inner();
///
/// let hash = dirhash_h1(&bytes).unwrap();
/// assert!(hash.starts_with("h1:"));
/// ```
pub fn dirhash_h1(zip_bytes: &[u8]) -> Result<String, Error> {
    let archive_error = |e: &dyn std::fmt::Display| Error::Archive(e.to_string());
    let mut zip = zip::ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| archive_error(&e))?;
    let mut lines: Vec<(String, String)> = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| archive_error(&e))?;
        if !file.is_file() {
            continue;
        }
        let name = file.name().map_err(|e| archive_error(&e))?.into_owned();
        if name.contains('\n') {
            return Err(Error::Archive(format!("file name with newline: {name:?}")));
        }
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .map_err(|e| archive_error(&e))?;
        lines.push((name, pelicode_core::hash::to_hex(&Sha256::digest(&content))));
    }
    lines.sort();
    let mut summary = Sha256::new();
    for (name, hash) in &lines {
        summary.update(format!("{hash}  {name}\n").as_bytes());
    }
    Ok(format!(
        "h1:{}",
        base64::engine::general_purpose::STANDARD.encode(summary.finalize())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn escapes_upper_case() {
        assert_eq!(
            escape("github.com/BurntSushi/toml"),
            "github.com/!burnt!sushi/toml"
        );
        assert_eq!(escape("v1.0.0"), "v1.0.0");
    }

    #[test]
    fn parses_checksum_lookup() {
        let body = "22152757\n\
            github.com/google/uuid v1.6.0 h1:NIvaJDMOsjHA8n1jAhLSgzrAzy1Hgr+hNrb57e+94F0=\n\
            github.com/google/uuid v1.6.0/go.mod h1:TIyPZe4MgqvfeYDBFedMoGGpEw/LqOeaOT+nhxU+yHo=\n\n\
            go.sum database tree\n";
        assert_eq!(
            parse_lookup(body, "github.com/google/uuid", "v1.6.0").as_deref(),
            Some("h1:NIvaJDMOsjHA8n1jAhLSgzrAzy1Hgr+hNrb57e+94F0=")
        );
        assert_eq!(parse_lookup(body, "github.com/google/uuid", "v1.5.0"), None);
    }

    #[test]
    fn repository_of_forges() {
        assert_eq!(
            repository_of("github.com/google/uuid/v2").as_deref(),
            Some("https://github.com/google/uuid")
        );
        assert_eq!(repository_of("golang.org/x/mod"), None);
    }

    #[test]
    fn dirhash_is_order_independent() {
        let zip_of = |files: &[(&str, &str)]| {
            let mut out = Cursor::new(Vec::new());
            let mut zip = zip::ZipWriter::new(&mut out);
            for (name, content) in files {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
            out.into_inner()
        };
        let a = zip_of(&[("m@v1/a.go", "package a"), ("m@v1/go.mod", "module m")]);
        let b = zip_of(&[("m@v1/go.mod", "module m"), ("m@v1/a.go", "package a")]);
        let h = dirhash_h1(&a).unwrap();
        assert!(h.starts_with("h1:"));
        assert_eq!(h, dirhash_h1(&b).unwrap());
    }
}
