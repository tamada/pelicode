//! Fetching snapshots from OCI registries (`oci://REGISTRY/REPOSITORY:TAG`).
//!
//! Snapshots are pushed by `just snapshot_push` as OCI artifacts whose single
//! layer is the database compressed with zstd. They are fetched with the OCI
//! distribution API, anonymously for public repositories, verified against
//! the digest of the layer, decompressed, and cached by digest: a second
//! fetch of an unchanged tag downloads only the manifest.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use sha2::{Digest as _, Sha256};
use ureq::Agent;

/// Scheme of snapshot references.
pub const SCHEME: &str = "oci://";
/// Media type of the layer holding a snapshot.
pub const SNAPSHOT_MEDIA_TYPE: &str = "application/vnd.pelicode.snapshot.v1.sqlite+zstd";

/// Prefix of the annotations pelicode gives snapshots.
pub const ANNOTATION_PREFIX: &str = "io.github.tamada.pelicode.";

const MANIFEST_TYPES: &str = "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json";

/// Whether `database` names a snapshot in a registry rather than a file.
pub fn is_reference(database: &Path) -> bool {
    database.to_str().is_some_and(|s| s.starts_with(SCHEME))
}

/// A snapshot in a registry: `REGISTRY/REPOSITORY[:TAG|@DIGEST]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Host (and port) of the registry, such as `quay.io`.
    pub registry: String,
    /// Repository, such as `tama5/pelicode-db`.
    pub repository: String,
    /// Tag or digest (`sha256:...`); `latest` when omitted.
    pub reference: String,
}

impl FromStr for Reference {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Reference> {
        let rest = s.strip_prefix(SCHEME).unwrap_or(s);
        let invalid = || anyhow!("{s}: expected oci://REGISTRY/REPOSITORY[:TAG|@DIGEST]");
        let (registry, path) = rest.split_once('/').ok_or_else(invalid)?;
        let (repository, reference) = match path.split_once('@') {
            Some((repository, digest)) => (repository, digest.to_string()),
            None => match path.rsplit_once(':') {
                Some((repository, tag)) if !tag.contains('/') => (repository, tag.to_string()),
                _ => (path, "latest".to_string()),
            },
        };
        if registry.is_empty() || repository.is_empty() || reference.is_empty() {
            return Err(invalid());
        }
        Ok(Reference {
            registry: registry.to_string(),
            repository: repository.to_string(),
            reference,
        })
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let separator = if self.reference.contains(':') {
            '@'
        } else {
            ':'
        };
        write!(
            f,
            "{SCHEME}{}/{}{separator}{}",
            self.registry, self.repository, self.reference
        )
    }
}

impl Reference {
    /// Base URL of the registry API. Registries on this machine are reached
    /// over plain HTTP, as local test registries usually are.
    fn base(&self) -> String {
        let host = self.registry.split(':').next().unwrap_or_default();
        let scheme = if matches!(host, "localhost" | "127.0.0.1") {
            "http"
        } else {
            "https"
        };
        format!("{scheme}://{}/v2/{}", self.registry, self.repository)
    }

    /// The file remembering the digest this reference pointed to.
    fn ref_file(&self, cache: &Path) -> PathBuf {
        let name = self.reference.replace(':', "-");
        cache
            .join("refs")
            .join(self.registry.replace(':', "_"))
            .join(&self.repository)
            .join(name)
    }
}

/// A snapshot available as a local file.
#[derive(Debug)]
pub struct Fetched {
    /// The decompressed database.
    pub path: PathBuf,
    /// Digest of the compressed layer.
    pub digest: String,
    /// Whether the layer was downloaded now (else it was in the cache).
    pub downloaded: bool,
}

/// Where snapshots are cached: `$PELICODE_CACHE_DIR`, else the cache directory
/// of the user (`~/.cache/pelicode`, `~/Library/Caches/pelicode`, ...).
pub fn cache_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("PELICODE_CACHE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    dirs::cache_dir()
        .map(|d| d.join("pelicode"))
        .ok_or_else(|| anyhow!("no cache directory; set PELICODE_CACHE_DIR"))
}

/// Fetches the snapshot `reference` into `cache` (see [`cache_dir`]), or finds it there.
///
/// When the registry cannot be reached, the snapshot the reference pointed
/// to last time is used, with a warning.
pub fn fetch(reference: &Reference, cache: &Path) -> Result<Fetched> {
    let client = Registry::new(reference);
    let (layer, annotations) = match client.manifest() {
        Ok(found) => found,
        Err(Failure::Unreachable(e)) => return offline(reference, cache, e),
        Err(Failure::Other(e)) => return Err(e),
    };
    check(reference, &annotations)?;
    let path = blob_path(cache, &layer.digest)?;
    let downloaded = if path.exists() {
        false
    } else {
        client.download(&layer, &path)?;
        true
    };
    let ref_file = reference.ref_file(cache);
    if let Some(dir) = ref_file.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&ref_file, &layer.digest)
        .with_context(|| format!("cannot write {}", ref_file.display()))?;
    Ok(Fetched {
        path,
        digest: layer.digest,
        downloaded,
    })
}

fn offline(reference: &Reference, cache: &Path, error: anyhow::Error) -> Result<Fetched> {
    let digest = fs::read_to_string(reference.ref_file(cache))
        .ok()
        .map(|d| d.trim().to_string());
    match digest {
        Some(digest) if blob_path(cache, &digest).is_ok_and(|p| p.exists()) => {
            eprintln!("warning: {reference}: {error:#}; using the cached snapshot {digest}");
            Ok(Fetched {
                path: blob_path(cache, &digest)?,
                digest,
                downloaded: false,
            })
        }
        _ => Err(error.context(format!("cannot fetch {reference}"))),
    }
}

/// The decompressed snapshot of the layer `digest` in the cache.
fn blob_path(cache: &Path, digest: &str) -> Result<PathBuf> {
    let hex = digest
        .strip_prefix("sha256:")
        .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| anyhow!("unsupported digest {digest}"))?;
    Ok(cache.join("snapshots").join(format!("sha256-{hex}.db")))
}

/// A layer of a manifest.
#[derive(Debug, serde::Deserialize)]
struct Layer {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: u64,
}

#[derive(Debug, serde::Deserialize)]
struct Manifest {
    #[serde(default)]
    layers: Vec<Layer>,
    #[serde(default)]
    annotations: BTreeMap<String, String>,
}

/// What the manifest of a snapshot says, before downloading it.
#[derive(Debug, serde::Serialize)]
pub struct Info {
    /// Digest of the compressed snapshot.
    pub digest: String,
    /// Size of the compressed snapshot in bytes.
    pub size: u64,
    /// Annotations of the manifest.
    pub annotations: BTreeMap<String, String>,
}

/// The annotations to give the manifest of a snapshot: its versions,
/// feature parameters, contents and creation time.
pub fn annotations(summary: &pelicode_db::SnapshotSummary) -> BTreeMap<String, String> {
    let mut annotations = BTreeMap::new();
    let mut put = |key: &str, value: String| {
        annotations.insert(format!("{ANNOTATION_PREFIX}{key}"), value);
    };
    put("version", env!("CARGO_PKG_VERSION").to_string());
    for (key, meta) in [
        ("schema-version", "schema_version"),
        ("normalizer-version", "normalizer_version"),
        ("ident-mode", "ident_mode"),
        ("kgram", "kgram"),
        ("window", "window"),
        ("minhash-size", "minhash_size"),
        ("bands", "bands"),
        ("fingerprint-levels", "fingerprint_levels"),
    ] {
        if let Some(value) = summary.meta.get(meta) {
            put(key, value.clone());
        }
    }
    put("ecosystems", summary.ecosystems.join(","));
    put("packages", summary.packages.to_string());
    put("versions", summary.versions.to_string());
    put("functions", summary.functions.to_string());
    if let Some(created) = summary.meta.get("snapshot_created_at") {
        annotations.insert("org.opencontainers.image.created".into(), created.clone());
    }
    annotations.insert(
        "org.opencontainers.image.source".into(),
        "https://github.com/tamada/pelicode".into(),
    );
    annotations
}

/// Whether this pelicode can read the snapshot the annotations describe.
/// Snapshots without annotations are accepted; the database itself is
/// checked when it is opened.
fn check(reference: &Reference, annotations: &BTreeMap<String, String>) -> Result<()> {
    let version = |key: &str| -> Result<Option<u32>> {
        let key = format!("{ANNOTATION_PREFIX}{key}");
        annotations
            .get(&key)
            .map(|v| {
                v.parse()
                    .map_err(|_| anyhow!("{reference}: invalid annotation {key} = {v}"))
            })
            .transpose()
    };
    let newer = annotations
        .get(&format!("{ANNOTATION_PREFIX}version"))
        .map(|v| format!(" (made by pelicode {v})"))
        .unwrap_or_default();
    if let Some(schema) = version("schema-version")? {
        let current = pelicode_db::SCHEMA_VERSION;
        if schema > current {
            bail!(
                "{reference}: the snapshot has schema version {schema}{newer}, but this pelicode \
                 reads up to {current}; update pelicode"
            );
        }
        if schema < pelicode_db::OLDEST_SCHEMA_VERSION {
            bail!(
                "{reference}: the snapshot has schema version {schema}{newer}, which this \
                 pelicode no longer reads; use a newer snapshot"
            );
        }
    }
    if let Some(normalizer) = version("normalizer-version")? {
        let current = pelicode_core::NORMALIZER_VERSION;
        if normalizer > current {
            bail!(
                "{reference}: the snapshot was built with normalizer version {normalizer}{newer}, \
                 but this pelicode has {current}; update pelicode"
            );
        }
        if normalizer < current {
            bail!(
                "{reference}: the snapshot was built with normalizer version {normalizer}{newer}, \
                 but this pelicode has {current}; use a newer snapshot"
            );
        }
    }
    Ok(())
}

/// What the manifest of the snapshot `reference` says, without downloading it.
pub fn info(reference: &Reference) -> Result<Info> {
    let (layer, annotations) = match Registry::new(reference).manifest() {
        Ok(found) => found,
        Err(Failure::Unreachable(e)) | Err(Failure::Other(e)) => return Err(e),
    };
    Ok(Info {
        digest: layer.digest,
        size: layer.size,
        annotations,
    })
}

enum Failure {
    /// The registry could not be reached at all.
    Unreachable(anyhow::Error),
    Other(anyhow::Error),
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Failure {
        Failure::Other(e)
    }
}

struct Registry<'a> {
    reference: &'a Reference,
    agent: Agent,
    token: std::cell::RefCell<Option<String>>,
}

impl<'a> Registry<'a> {
    fn new(reference: &'a Reference) -> Registry<'a> {
        let config = Agent::config_builder()
            .user_agent(format!("pelicode/{}", env!("CARGO_PKG_VERSION")))
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .build();
        Registry {
            reference,
            agent: Agent::new_with_config(config),
            token: std::cell::RefCell::new(None),
        }
    }

    /// The layer of the manifest holding the snapshot, and the annotations.
    fn manifest(&self) -> std::result::Result<(Layer, BTreeMap<String, String>), Failure> {
        let url = format!(
            "{}/manifests/{}",
            self.reference.base(),
            self.reference.reference
        );
        let mut response = self.get(&url, Some(MANIFEST_TYPES)).map_err(|e| match e {
            ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed => {
                Failure::Unreachable(anyhow!(e))
            }
            other => Failure::Other(anyhow!("{url}: {other}")),
        })?;
        let status = response.status();
        if status == 404 {
            return Err(anyhow!("{}: no such snapshot", self.reference).into());
        }
        if !status.is_success() {
            return Err(anyhow!("{url}: HTTP {status}").into());
        }
        let manifest: Manifest = response
            .body_mut()
            .read_json()
            .with_context(|| format!("{url}: invalid manifest"))?;
        let types: Vec<String> = manifest
            .layers
            .iter()
            .map(|l| l.media_type.clone())
            .collect();
        let annotations = manifest.annotations;
        let layer = manifest
            .layers
            .into_iter()
            .find(|l| l.media_type == SNAPSHOT_MEDIA_TYPE)
            .ok_or_else(|| {
                anyhow!(
                    "{}: not a pelicode snapshot (layers: {})",
                    self.reference,
                    if types.is_empty() {
                        "none".into()
                    } else {
                        types.join(", ")
                    }
                )
            })?;
        Ok((layer, annotations))
    }

    /// Downloads `layer`, checks its digest, and decompresses it to `path`.
    fn download(&self, layer: &Layer, path: &Path) -> Result<()> {
        let url = format!("{}/blobs/{}", self.reference.base(), layer.digest);
        let response = self.get(&url, None).with_context(|| url.clone())?;
        if !response.status().is_success() {
            bail!("{url}: HTTP {}", response.status());
        }
        let dir = path.parent().expect("blob paths have a directory");
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        let mut hashing = Hashing::new(response.into_body().into_reader());
        let mut decoder = zstd::stream::read::Decoder::new(&mut hashing)?;
        io::copy(&mut decoder, temp.as_file_mut())
            .with_context(|| format!("{url}: cannot decompress"))?;
        drop(decoder);
        // Anything after the zstd frame still counts for the digest.
        io::copy(&mut hashing, &mut io::sink())?;
        let actual = format!(
            "sha256:{}",
            pelicode_core::hash::to_hex(&hashing.hasher.finalize())
        );
        if actual != layer.digest || hashing.read != layer.size {
            bail!(
                "{url}: digest mismatch (expected {} of {} bytes, got {actual} of {} bytes)",
                layer.digest,
                layer.size,
                hashing.read
            );
        }
        temp.as_file_mut().flush()?;
        temp.persist(path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        Ok(())
    }

    /// GETs `url`, asking the registry for an anonymous token on 401.
    fn get(
        &self,
        url: &str,
        accept: Option<&str>,
    ) -> std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error> {
        let request = |token: Option<&str>| {
            let mut request = self.agent.get(url);
            if let Some(accept) = accept {
                request = request.header("Accept", accept);
            }
            if let Some(token) = token {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            request.call()
        };
        let token = self.token.borrow().clone();
        let response = request(token.as_deref())?;
        if response.status() != 401 || token.is_some() {
            return Ok(response);
        }
        let challenge = response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let Some(token) = challenge.and_then(|c| self.anonymous_token(&c)) else {
            return Ok(response);
        };
        *self.token.borrow_mut() = Some(token.clone());
        request(Some(&token))
    }

    /// A token for pulling, from the realm of a `Bearer` challenge.
    fn anonymous_token(&self, challenge: &str) -> Option<String> {
        let params = parse_challenge(challenge)?;
        let realm = params.iter().find(|(k, _)| k == "realm")?.1.clone();
        let mut request = self.agent.get(&realm).query(
            "scope",
            format!("repository:{}:pull", self.reference.repository),
        );
        if let Some((_, service)) = params.iter().find(|(k, _)| k == "service") {
            request = request.query("service", service);
        }
        let mut response = request.call().ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: serde_json::Value = response.body_mut().read_json().ok()?;
        body.get("token")
            .or_else(|| body.get("access_token"))
            .and_then(|t| t.as_str())
            .map(str::to_string)
    }
}

/// The parameters of `Bearer realm="...",service="...",scope="..."`.
fn parse_challenge(challenge: &str) -> Option<Vec<(String, String)>> {
    let rest = challenge.trim().strip_prefix("Bearer")?.trim();
    let mut params = Vec::new();
    for part in rest.split(',') {
        let (key, value) = part.split_once('=')?;
        params.push((
            key.trim().to_ascii_lowercase(),
            value.trim().trim_matches('"').to_string(),
        ));
    }
    Some(params)
}

/// A reader that hashes and counts what passes through it.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
    read: u64,
}

impl<R: Read> Hashing<R> {
    fn new(inner: R) -> Hashing<R> {
        Hashing {
            inner,
            hasher: Sha256::new(),
            read: 0,
        }
    }
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.read += n as u64;
        Ok(n)
    }
}

/// The database a `--db` names: the file itself, or the fetched snapshot of an
/// `oci://` reference.
pub fn resolve(database: &Path) -> Result<PathBuf> {
    if !is_reference(database) {
        return Ok(database.to_path_buf());
    }
    let reference: Reference = database.to_string_lossy().parse()?;
    let fetched = fetch(&reference, &cache_dir()?)?;
    if fetched.downloaded {
        eprintln!("fetched {reference} ({})", fetched.digest);
    }
    Ok(fetched.path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_references() {
        let r: Reference = "oci://quay.io/tama5/pelicode-db:pypi-latest"
            .parse()
            .unwrap();
        assert_eq!(
            (
                r.registry.as_str(),
                r.repository.as_str(),
                r.reference.as_str()
            ),
            ("quay.io", "tama5/pelicode-db", "pypi-latest")
        );
        assert_eq!(r.to_string(), "oci://quay.io/tama5/pelicode-db:pypi-latest");

        let r: Reference = "oci://localhost:5000/db".parse().unwrap();
        assert_eq!(
            (r.registry.as_str(), r.reference.as_str()),
            ("localhost:5000", "latest")
        );
        assert_eq!(r.base(), "http://localhost:5000/v2/db");

        let digest = format!("sha256:{}", "a".repeat(64));
        let r: Reference = format!("oci://ghcr.io/o/r@{digest}").parse().unwrap();
        assert_eq!(r.reference, digest);
        assert_eq!(r.to_string(), format!("oci://ghcr.io/o/r@{digest}"));

        assert!("oci://quay.io".parse::<Reference>().is_err());
    }

    #[test]
    fn parses_challenges() {
        let params =
            parse_challenge(r#"Bearer realm="https://quay.io/v2/auth",service="quay.io""#).unwrap();
        assert_eq!(
            params,
            [
                ("realm".to_string(), "https://quay.io/v2/auth".to_string()),
                ("service".to_string(), "quay.io".to_string())
            ]
        );
        assert!(parse_challenge("Basic realm=x").is_none());
    }

    #[test]
    fn rejects_other_digests() {
        assert!(blob_path(Path::new("/c"), "md5:00").is_err());
        assert!(blob_path(Path::new("/c"), &format!("sha256:{}", "0".repeat(64))).is_ok());
    }
}
