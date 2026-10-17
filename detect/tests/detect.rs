use pelicode_core::{Config, FileFeatures, Language, Level, extract_bytes};
use pelicode_db::{Database, PackageInfo};
use pelicode_detect::{Detector, MatchKind, Options};

const ORIGINAL: &str = r#"
public class Search {
    public static int binarySearch(int[] values, int key) {
        int low = 0;
        int high = values.length - 1;
        while (low <= high) {
            int mid = (low + high) >>> 1;
            int v = values[mid];
            if (v < key) {
                low = mid + 1;
            } else if (v > key) {
                high = mid - 1;
            } else {
                return mid;
            }
        }
        return -(low + 1);
    }
}
"#;

/// Renamed identifiers, reformatted, one statement added.
const DISGUISED: &str = r#"
class Finder {
    static int find(int[] xs, int target) {
        int lo = 0;
        int hi = xs.length - 1;
        int steps = 0;
        while (lo <= hi) {
            int m = (lo + hi) >>> 1; int x = xs[m];
            if (x < target) { lo = m + 1; }
            else if (x > target) { hi = m - 1; }
            else { return m; }
        }
        return -(lo + 1);
    }
}
"#;

const UNRELATED: &str = r#"
class Other {
    static String greet(String name, int times) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < times; i++) {
            sb.append("Hello, ").append(name).append('!');
            if (i % 2 == 0) { sb.append('\n'); }
        }
        return sb.toString().trim();
    }
}
"#;

fn config() -> Config {
    Config {
        min_tokens: 20,
        ..Config::default()
    }
}

fn file(path: &str, src: &str) -> FileFeatures {
    extract_bytes(path.as_ref(), Language::Java, src.as_bytes(), &config()).unwrap()
}

fn database(dir: &tempfile::TempDir) -> Database {
    let mut db = Database::open(dir.path().join("t.db"), &config()).unwrap();
    let pkg = PackageInfo {
        ecosystem: "maven".into(),
        name: "org.example:search".into(),
        version: "1.0".into(),
        license_spdx: Some("Apache-2.0".into()),
        ..PackageInfo::default()
    };
    db.index(&pkg, &[file("Search.java", ORIGINAL)]).unwrap();
    db
}

fn options() -> Options {
    Options {
        min_tokens: 20,
        threshold: 0.5,
        ..Options::default()
    }
}

#[test]
fn finds_exact_copies() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(&dir);
    let reports = Detector::new(&db, options())
        .detect(&[file("Copy.java", ORIGINAL)], |_, _| true)
        .unwrap();
    assert_eq!(reports.len(), 1);
    let c = &reports[0].candidates[0];
    assert_eq!(c.kind, MatchKind::Exact(Level::N0));
    assert_eq!(c.similarity, 1.0);
    assert_eq!(c.occurrences[0].package, "org.example:search");
    assert_eq!(c.occurrences[0].license.as_deref(), Some("Apache-2.0"));
}

#[test]
fn finds_disguised_copies() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(&dir);
    let reports = Detector::new(&db, options())
        .detect(&[file("Finder.java", DISGUISED)], |_, _| true)
        .unwrap();
    assert_eq!(reports.len(), 1, "{reports:#?}");
    let c = &reports[0].candidates[0];
    assert_eq!(c.kind, MatchKind::Similar(Level::N1));
    assert!(c.similarity >= 0.5, "{}", c.similarity);
    assert!(!c.matched_lines.is_empty());
    assert_eq!(c.function.name, "binarySearch");
}

#[test]
fn ignores_unrelated_and_excluded_code() {
    let dir = tempfile::tempdir().unwrap();
    let db = database(&dir);
    let reports = Detector::new(&db, options())
        .detect(&[file("Other.java", UNRELATED)], |_, _| true)
        .unwrap();
    assert!(reports.is_empty(), "{reports:#?}");

    let excluding = Options {
        exclude_packages: vec![("maven".into(), "org.example:search".into())],
        ..options()
    };
    let reports = Detector::new(&db, excluding)
        .detect(&[file("Copy.java", ORIGINAL)], |_, _| true)
        .unwrap();
    assert!(reports.is_empty());
}

#[test]
fn config_follows_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    let stored = Config {
        kgram: 7,
        ..Config::default()
    };
    Database::open(&path, &stored).unwrap();
    let db = Database::open_unchecked(&path).unwrap();
    assert_eq!(db.config_from_meta(&Config::default()).unwrap().kgram, 7);
}
