//! Fetching snapshots from a registry, against a minimal registry on localhost.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use sha2::{Digest, Sha256};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/project");
const MEDIA_TYPE: &str = "application/vnd.pelicode.snapshot.v1.sqlite+zstd";
const TOKEN: &str = "anonymous-token";

fn run(cache: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pelicode"))
        .args(args)
        .arg("--no-config")
        .env("PELICODE_CACHE_DIR", cache)
        .output()
        .unwrap()
}

/// A registry serving one artifact, `test/db:latest`, behind an anonymous
/// token like quay.io. Counts the downloads of the blob.
struct Registry {
    address: String,
    downloads: Arc<AtomicUsize>,
}

impl Registry {
    fn start(layer: Vec<u8>, digest: String) -> Registry {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let downloads = Arc::new(AtomicUsize::new(0));
        let counter = downloads.clone();
        let realm = format!("http://{address}/v2/auth");
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                respond(stream, &layer, &digest, &realm, &counter);
            }
        });
        Registry { address, downloads }
    }

    fn reference(&self) -> String {
        format!("oci://{}/test/db:latest", self.address)
    }
}

fn respond(
    mut stream: TcpStream,
    layer: &[u8],
    digest: &str,
    realm: &str,
    downloads: &AtomicUsize,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut authorized = false;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
        if line.to_ascii_lowercase().starts_with("authorization:") {
            authorized = line.contains(TOKEN);
        }
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "artifactType": "application/vnd.pelicode.snapshot.v1",
        "layers": [{ "mediaType": MEDIA_TYPE, "digest": digest, "size": layer.len() }],
    })
    .to_string();
    let (status, extra, body): (&str, String, Vec<u8>) = if path.starts_with("/v2/auth") {
        (
            "200 OK",
            String::new(),
            format!(r#"{{"token":"{TOKEN}"}}"#).into_bytes(),
        )
    } else if !authorized {
        let challenge = format!("WWW-Authenticate: Bearer realm=\"{realm}\",service=\"test\"\r\n");
        ("401 Unauthorized", challenge, Vec::new())
    } else if path == "/v2/test/db/manifests/latest" {
        ("200 OK", String::new(), manifest.into_bytes())
    } else if path == format!("/v2/test/db/blobs/{digest}") {
        downloads.fetch_add(1, Ordering::SeqCst);
        ("200 OK", String::new(), layer.to_vec())
    } else {
        ("404 Not Found", String::new(), Vec::new())
    };
    let head = format!(
        "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

/// A snapshot of the fixture, compressed, with its digest.
fn snapshot(dir: &Path) -> (Vec<u8>, String) {
    let db = dir.join("ref.db").to_string_lossy().into_owned();
    let out = dir.join("snapshot.db").to_string_lossy().into_owned();
    for args in [
        vec!["index", FIXTURE, "--db", &db, "--min-tokens", "10"],
        vec!["snapshot", "--db", &db, "-o", &out],
    ] {
        let output = run(dir, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let layer = zstd::encode_all(std::fs::read(&out).unwrap().as_slice(), 3).unwrap();
    let hex: String = Sha256::digest(&layer)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let digest = format!("sha256:{hex}");
    (layer, digest)
}

#[test]
fn pulls_and_caches_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let (layer, digest) = snapshot(dir.path());
    let registry = Registry::start(layer, digest.clone());
    let reference = registry.reference();

    let first = run(&cache, &["pull", &reference]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let path = String::from_utf8(first.stdout).unwrap().trim().to_string();
    assert!(Path::new(&path).starts_with(&cache));
    assert!(String::from_utf8_lossy(&first.stderr).contains(&digest));

    // The tag still points to the same digest: nothing is downloaded again.
    let second = run(&cache, &["pull", &reference]);
    assert!(String::from_utf8_lossy(&second.stderr).contains("up to date"));
    assert_eq!(registry.downloads.load(Ordering::SeqCst), 1);

    // scan takes the reference directly.
    let copied = dir.path().join("copied.py");
    std::fs::copy(format!("{FIXTURE}/lib/stack.py"), &copied).unwrap();
    let scan = run(
        &cache,
        &[
            "scan",
            copied.to_str().unwrap(),
            "--db",
            &reference,
            "--min-tokens",
            "10",
            "-f",
            "json",
        ],
    );
    let reports: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    assert!(!reports.as_array().unwrap().is_empty());
    assert_eq!(registry.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn uses_the_cache_when_the_registry_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let (layer, digest) = snapshot(dir.path());
    let registry = Registry::start(layer, digest);
    assert!(
        run(&cache, &["pull", &registry.reference()])
            .status
            .success()
    );

    // Nothing listens on this port any more.
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = closed.local_addr().unwrap().to_string();
    drop(closed);
    let gone = run(
        &cache,
        &["pull", &format!("oci://{address}/test/db:latest")],
    );
    assert!(
        !gone.status.success(),
        "no cached snapshot for this registry"
    );

    // The same registry address as cached, but unreachable: copy the ref.
    let refs = cache.join("refs");
    let from = refs.join(registry.address.replace(':', "_"));
    let to = refs.join(address.replace(':', "_"));
    copy_dir(&from, &to);
    let offline = run(
        &cache,
        &["pull", &format!("oci://{address}/test/db:latest")],
    );
    assert!(
        offline.status.success(),
        "{}",
        String::from_utf8_lossy(&offline.stderr)
    );
    assert!(String::from_utf8_lossy(&offline.stderr).contains("using the cached snapshot"));
}

#[test]
fn rejects_corrupt_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let (mut layer, digest) = snapshot(dir.path());
    let last = layer.len() - 1;
    layer[last] ^= 0xff;
    let registry = Registry::start(layer, digest);
    let output = run(&cache, &["pull", &registry.reference()]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("digest mismatch") || stderr.contains("decompress"),
        "{stderr}"
    );
    assert!(!cache.join("snapshots").read_dir().is_ok_and(|mut d| {
        d.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "db")))
    }));
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
