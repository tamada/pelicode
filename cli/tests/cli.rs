use std::path::Path;
use std::process::Command;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/project");

fn pelicode(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_pelicode"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn db_arg(dir: &Path) -> String {
    dir.join("t.db").to_string_lossy().into_owned()
}

#[test]
fn indexes_every_supported_language() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    let summary = pelicode(&["index", FIXTURE, "--db", &db, "--format", "json"]);
    let summary: serde_json::Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(summary["package"]["name"], "project");
    assert_eq!(summary["summary"]["files"], 5);

    let rows: serde_json::Value =
        serde_json::from_str(&pelicode(&["export", "--db", &db, "--format", "json"])).unwrap();
    let mut languages: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["language"].as_str().unwrap())
        .collect();
    languages.dedup();
    assert_eq!(languages, ["ruby", "python", "java", "kotlin", "go"]);
}

#[test]
fn extract_formats() {
    let csv = pelicode(&["extract", FIXTURE, "--format", "csv"]);
    let mut lines = csv.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("path,language,name,kind,")
    );
    assert_eq!(lines.count(), 16);

    let json: serde_json::Value =
        serde_json::from_str(&pelicode(&["extract", FIXTURE, "-f", "json", "--tokens"])).unwrap();
    let first = &json[0]["functions"][0];
    assert_eq!(first["name"], "initialize");
    assert!(first["tokens"]["N1"].is_array());
}
