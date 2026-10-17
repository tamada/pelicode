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

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn scans_changed_functions_of_a_git_range() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    pelicode(&["index", FIXTURE, "--db", &db, "--min-tokens", "10"]);

    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "demo\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    std::fs::copy(
        Path::new(FIXTURE).join("src/Stack.java"),
        repo.join("Copied.java"),
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "copy"]);

    let repo_arg = repo.to_string_lossy().into_owned();
    let scan = |range: &str| -> serde_json::Value {
        serde_json::from_str(&pelicode(&[
            "scan",
            &repo_arg,
            "--db",
            &db,
            "--git",
            range,
            "--min-tokens",
            "10",
            "-f",
            "json",
        ]))
        .unwrap()
    };
    let reports = scan("HEAD~1..HEAD");
    let names: Vec<_> = reports
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["push", "pop"]);
    assert_eq!(reports[0]["candidates"][0]["match"]["type"], "exact");
    assert_eq!(
        reports[0]["candidates"][0]["occurrences"][0]["path"],
        "src/Stack.java"
    );

    // Nothing changed in the working tree.
    assert_eq!(scan("HEAD"), serde_json::json!([]));

    let status = Command::new(env!("CARGO_BIN_EXE_pelicode"))
        .args([
            "scan",
            &repo_arg,
            "--db",
            &db,
            "--min-tokens",
            "10",
            "--fail-on-match",
        ])
        .output()
        .unwrap()
        .status;
    assert_eq!(status.code(), Some(1));
}

#[test]
fn several_directories_keep_distinct_paths() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    let src = format!("{FIXTURE}/src");
    let lib = format!("{FIXTURE}/lib");
    pelicode(&["index", &src, &lib, "--db", &db, "--name", "demo"]);
    let csv = pelicode(&["export", "--db", &db, "-f", "csv"]);
    assert!(csv.contains(",src/Stack.java,"), "{csv}");
    assert!(csv.contains(",lib/stack.py,"), "{csv}");
}

#[test]
fn stats_after_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    pelicode(&[
        "index",
        FIXTURE,
        "--db",
        &db,
        "--name",
        "a",
        "--refresh-stats",
    ]);
    let stats: serde_json::Value =
        serde_json::from_str(&pelicode(&["stats", "--db", &db, "-f", "json"])).unwrap();
    assert_eq!(stats["hash_stats"]["stale"], false);
    assert_eq!(stats["hash_stats"]["packages"], 1);

    // The same code in a second package makes its hashes common.
    pelicode(&["index", FIXTURE, "--db", &db, "--name", "b"]);
    let stats: serde_json::Value = serde_json::from_str(&pelicode(&[
        "stats",
        "--db",
        &db,
        "--refresh",
        "--min-tokens",
        "0",
        "-f",
        "json",
    ]))
    .unwrap();
    assert_eq!(stats["hash_stats"]["packages"], 2);
    assert_eq!(stats["common_exact_hashes"][0]["package_count"], 2);
}
