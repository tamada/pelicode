use std::path::Path;
use std::process::Command;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/project");

/// Runs pelicode without settings files, so that a settings file in the
/// repository does not change the results.
fn pelicode(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_pelicode"))
        .args(args)
        .arg("--no-config")
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

/// Runs pelicode in `dir`, where it looks for settings files.
fn pelicode_in(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pelicode"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn reported_files(output: &std::process::Output) -> Vec<String> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut files: Vec<String> = reports
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_string())
        .collect();
    files.dedup();
    files
}

#[test]
fn settings_files_configure_scan() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("data")).unwrap();
    std::fs::create_dir(work.join(".git")).unwrap();
    let db = work.join("data/ref.db").to_string_lossy().into_owned();
    pelicode(&["index", FIXTURE, "--db", &db, "--min-tokens", "10"]);

    // The database comes from the settings file, relative to it.
    std::fs::write(
        work.join(".pelicode.yaml"),
        "version: 1\ndatabase: data/ref.db\nscan:\n  min_tokens: 10\n  languages: [python, go]\n",
    )
    .unwrap();
    let scan = |extra: &[&str]| {
        let mut args = vec!["scan", FIXTURE, "-f", "json"];
        args.extend_from_slice(extra);
        reported_files(&pelicode_in(&work, &args))
    };
    assert_eq!(scan(&[]), ["lib/stack.py", "src/stack.go"]);
    // The command line wins over the settings file.
    assert_eq!(scan(&["--languages", "ruby"]), ["app/stack.rb"]);

    // JSON5, found from a subdirectory; paths excluded by glob.
    std::fs::remove_file(work.join(".pelicode.yaml")).unwrap();
    std::fs::write(
        work.join(".pelicode.json5"),
        "{ version: 1, database: 'data/ref.db', scan: { min_tokens: 10, exclude: { paths: ['src/**'] } } }",
    )
    .unwrap();
    std::fs::create_dir(work.join("sub")).unwrap();
    let files = reported_files(&pelicode_in(
        &work.join("sub"),
        &["scan", FIXTURE, "-f", "json"],
    ));
    assert_eq!(files, ["app/stack.rb", "lib/stack.py"]);

    // --no-config ignores it, so the default database is missing.
    let output = pelicode_in(&work, &["scan", FIXTURE, "--no-config"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("pelicode.db: no such database"));

    // Mistakes are reported with the file.
    std::fs::write(
        work.join(".pelicode.json5"),
        "{ version: 1, scan: { treshold: 0.5 } }",
    )
    .unwrap();
    let output = pelicode_in(&work, &["scan", FIXTURE]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains(".pelicode.json5") && stderr.contains("treshold"),
        "{stderr}"
    );
}

#[test]
fn sarif_for_code_scanning() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    pelicode(&["index", FIXTURE, "--db", &db, "--min-tokens", "10"]);
    let output = pelicode(&[
        "scan",
        FIXTURE,
        "--db",
        &db,
        "--min-tokens",
        "10",
        "-f",
        "sarif",
    ]);
    let log: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(log["version"], "2.1.0");
    let run = &log["runs"][0];
    assert_eq!(run["tool"]["driver"]["name"], "pelicode");
    let rules: Vec<_> = run["tool"]["driver"]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(rules, ["exact-match", "similar-match"]);

    let results = run["results"].as_array().unwrap();
    assert!(!results.is_empty());
    let push = results
        .iter()
        .find(|r| {
            r["properties"]["function"] == "push"
                && r["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
                    .as_str()
                    .unwrap()
                    .ends_with(".py")
        })
        .unwrap();
    assert_eq!(push["ruleId"], "exact-match");
    assert_eq!(push["level"], "note");
    // Relative to the working directory of the test (the crate), where the fixture lives.
    assert_eq!(
        push["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "tests/fixtures/project/lib/stack.py"
    );
    assert_eq!(
        push["locations"][0]["physicalLocation"]["region"]["startLine"],
        7
    );
    assert!(
        push["message"]["text"]
            .as_str()
            .unwrap()
            .contains("local:project@unversioned lib/stack.py")
    );
    assert_eq!(
        push["partialFingerprints"]["pelicodeCandidate/v1"]
            .as_str()
            .unwrap()
            .len(),
        32
    );
    assert_eq!(push["properties"]["candidates"][0]["match"], "exact");
}

#[test]
fn clean_removes_unreferenced_functions() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    let src = format!("{FIXTURE}/src");
    let lib = format!("{FIXTURE}/lib");
    pelicode(&[
        "index",
        &src,
        "--db",
        &db,
        "--name",
        "demo",
        "--version",
        "1.0",
    ]);
    // The same version again, with other files: those of src are left behind.
    pelicode(&[
        "index",
        &lib,
        "--db",
        &db,
        "--name",
        "demo",
        "--version",
        "1.0",
    ]);

    let dry: serde_json::Value = serde_json::from_str(&pelicode(&[
        "clean",
        "--db",
        &db,
        "--dry-run",
        "-f",
        "json",
    ]))
    .unwrap();
    assert_eq!(dry["removed"]["files"], 3);
    let done: serde_json::Value =
        serde_json::from_str(&pelicode(&["clean", "--db", &db, "-f", "json"])).unwrap();
    assert_eq!(done["removed"], dry["removed"]);
    assert!(done["size_after"].as_u64() <= done["size_before"].as_u64());
    let again: serde_json::Value =
        serde_json::from_str(&pelicode(&["clean", "--db", &db, "-f", "json"])).unwrap();
    assert_eq!(again["removed"]["files"], 0);

    let rows: serde_json::Value =
        serde_json::from_str(&pelicode(&["export", "--db", &db, "-f", "json"])).unwrap();
    assert!(
        rows.as_array()
            .unwrap()
            .iter()
            .all(|r| r["path"] == "stack.py")
    );
}

#[test]
fn dependencies_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    // The fixture stands for a published package the checked project depends on.
    pelicode(&[
        "index",
        FIXTURE,
        "--db",
        &db,
        "--ecosystem",
        "pypi",
        "--name",
        "requests",
        "--min-tokens",
        "10",
    ]);
    let work = dir.path().join("work");
    std::fs::create_dir_all(work.join("src")).unwrap();
    std::fs::create_dir(work.join(".git")).unwrap();
    std::fs::copy(
        Path::new(FIXTURE).join("lib/stack.py"),
        work.join("src/stack.py"),
    )
    .unwrap();
    // At the root of the project, above the directory checked.
    std::fs::write(work.join("requirements.txt"), "Requests==2.32.3\n").unwrap();

    let src = work.join("src").to_string_lossy().into_owned();
    let run = |extra: &[&str]| {
        let mut args = vec![
            "scan",
            src.as_str(),
            "--db",
            &db,
            "--min-tokens",
            "10",
            "-f",
            "json",
            "--no-config",
        ];
        args.extend_from_slice(extra);
        Command::new(env!("CARGO_BIN_EXE_pelicode"))
            .args(&args)
            .output()
            .unwrap()
    };
    let output = run(&[]);
    assert!(output.status.success());
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reports, serde_json::json!([]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("found only in dependencies") && stderr.contains("requirements.txt"),
        "{stderr}"
    );

    let output = run(&["--no-exclude-dependencies"]);
    let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!reports.as_array().unwrap().is_empty());
}

#[test]
fn snapshot_for_checking_only() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    let src = format!("{FIXTURE}/src");
    let lib = format!("{FIXTURE}/lib");
    for (path, ecosystem, version) in [
        (&src, "local", "1.0"),
        (&src, "local", "2.0"),
        (&lib, "pypi", "1.0"),
    ] {
        pelicode(&[
            "index",
            path,
            "--db",
            &db,
            "--ecosystem",
            ecosystem,
            "--name",
            "demo",
            "--version",
            version,
        ]);
    }

    let out = dir
        .path()
        .join("snapshot.db")
        .to_string_lossy()
        .into_owned();
    let args = [
        "snapshot",
        "--db",
        &db,
        "-o",
        &out,
        "--ecosystem",
        "local",
        "--latest",
        "-f",
        "json",
    ];
    let summary: serde_json::Value = serde_json::from_str(&pelicode(&args)).unwrap();
    assert_eq!(
        (summary["packages"].as_u64(), summary["versions"].as_u64()),
        (Some(1), Some(1))
    );
    assert!(summary["functions"].as_u64() > Some(0));

    // The file exists now.
    let again = pelicode_in(dir.path(), &args);
    assert!(!again.status.success());
    let mut forced = args.to_vec();
    forced.push("--force");
    pelicode(&forced);

    // Checking against the snapshot finds the indexed code ...
    let scan = pelicode_in(dir.path(), &["scan", &src, "--db", &out, "-f", "json"]);
    let reports: serde_json::Value = serde_json::from_slice(&scan.stdout).unwrap();
    let occurrences: Vec<_> = reports
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["candidates"].as_array().unwrap())
        .flat_map(|c| c["occurrences"].as_array().unwrap())
        .collect();
    assert!(!occurrences.is_empty());
    assert!(
        occurrences
            .iter()
            .all(|o| o["version"] == "2.0" && o["ecosystem"] == "local")
    );
    // ... but nothing can be indexed into it.
    let index = pelicode_in(
        dir.path(),
        &["index", &lib, "--db", &out, "--name", "x", "--version", "1"],
    );
    assert!(!index.status.success());
    assert!(String::from_utf8_lossy(&index.stderr).contains("snapshot"));
}

#[test]
fn several_databases_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    let (pypi, maven, other) = (path("pypi.db"), path("maven.db"), path("other.db"));
    let lib = format!("{FIXTURE}/lib");
    let src = format!("{FIXTURE}/src");
    for (dir, db, ecosystem) in [(&lib, &pypi, "pypi"), (&src, &maven, "maven")] {
        pelicode(&[
            "index",
            dir,
            "--db",
            db,
            "--ecosystem",
            ecosystem,
            "--name",
            "demo",
            "--min-tokens",
            "10",
        ]);
    }

    let ecosystems = |output: &std::process::Output| {
        let reports: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let mut found: Vec<String> = reports
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| r["candidates"].as_array().unwrap())
            .map(|c| {
                c["occurrences"][0]["ecosystem"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        found.sort();
        found.dedup();
        found
    };
    let both = pelicode_in(
        dir.path(),
        &[
            "scan",
            FIXTURE,
            "--db",
            &pypi,
            "--db",
            &maven,
            "--min-tokens",
            "10",
            "-f",
            "json",
        ],
    );
    assert_eq!(ecosystems(&both), ["maven", "pypi"]);

    // The same from a settings file listing both.
    std::fs::write(
        dir.path().join(".pelicode.yaml"),
        "version: 1\ndatabase: [pypi.db, maven.db]\n",
    )
    .unwrap();
    let configured = pelicode_in(
        dir.path(),
        &["scan", FIXTURE, "--min-tokens", "10", "-f", "json"],
    );
    assert_eq!(ecosystems(&configured), ["maven", "pypi"]);
    // Commands working on one database ask which.
    let stats = pelicode_in(dir.path(), &["stats"]);
    assert!(!stats.status.success());
    assert!(String::from_utf8_lossy(&stats.stderr).contains("--db"));

    // Databases built with other parameters cannot be combined.
    pelicode(&["index", &lib, "--db", &other, "--kgram", "7"]);
    let mixed = pelicode_in(
        dir.path(),
        &["scan", FIXTURE, "--db", &pypi, "--db", &other],
    );
    assert!(!mixed.status.success());
    assert!(String::from_utf8_lossy(&mixed.stderr).contains("kgram"));
}

#[test]
fn updates_of_an_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    // Nothing to check, so nothing is fetched.
    let out = pelicode(&["crawl", "pypi", "--updates", "--db", &db]);
    assert!(out.contains("no pypi packages"), "{out}");
}

#[test]
fn scans_staged_changes() {
    let dir = tempfile::tempdir().unwrap();
    let db = db_arg(dir.path());
    pelicode(&["index", FIXTURE, "--db", &db, "--min-tokens", "10"]);

    // A repository without commits yet: everything staged is checked.
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    let stack = std::fs::read(Path::new(FIXTURE).join("src/Stack.java")).unwrap();
    std::fs::write(repo.join("Staged.java"), &stack).unwrap();
    git(&repo, &["add", "Staged.java"]);
    // The staged content counts, not the working tree.
    std::fs::write(repo.join("Staged.java"), "class Staged {}\n").unwrap();
    // Not staged, so not checked.
    std::fs::write(repo.join("Unstaged.java"), &stack).unwrap();

    let repo_arg = repo.to_string_lossy().into_owned();
    let scan = || -> Vec<String> {
        let reports: serde_json::Value = serde_json::from_str(&pelicode(&[
            "scan",
            &repo_arg,
            "--db",
            &db,
            "--staged",
            "--min-tokens",
            "10",
            "-f",
            "json",
        ]))
        .unwrap();
        let mut paths: Vec<String> = reports
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_string())
            .collect();
        paths.dedup();
        paths
    };
    assert_eq!(scan(), ["Staged.java"]);

    // After a commit, nothing is staged.
    git(&repo, &["commit", "-q", "-m", "staged"]);
    assert!(scan().is_empty());

    // --staged and --git exclude each other.
    let both = pelicode_in(
        dir.path(),
        &["scan", &repo_arg, "--staged", "--git", "HEAD"],
    );
    assert!(!both.status.success());
}
