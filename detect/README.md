# pelicode-detect

Search for functions similar to known open-source code, for [pelicode](https://github.com/tamada/pelicode).

Given the features of the code being checked (from [pelicode-core](https://crates.io/crates/pelicode-core)) and a database of published code (from [pelicode-db](https://crates.io/crates/pelicode-db)), pelicode-detect lists, for each function, the stored functions it resembles, with their sources and licenses.

> pelicode-detect does **not** decide whether code is copied or violates a license. It reports *similar code candidates to review*; the decision is left to a person.

## How candidates are searched

1. **Exact matches**: hashes of the whole function are looked up from the weakest normalization to the strongest (N0, then N1, then N2).
2. **LSH**: MinHash buckets narrow down the stored functions that may be similar.
3. **Fingerprint comparison**: the winnowing fingerprint sets of those candidates are compared by Jaccard similarity (optionally IDF-weighted), and the matching line ranges on both sides are reported.

Common code can be filtered out along the way:

- hashes found in many packages (`Options::stop_packages`) are ignored;
- a minimum number of shared fingerprints (`Options::min_shared`) is required;
- packages can be excluded by name (`Options::exclude_packages`);
- matches found only in the project's own **dependencies** are left out. The [`dependencies`](https://docs.rs/pelicode-detect/latest/pelicode_detect/dependencies/index.html) module reads them from lock files and manifests (`requirements*.txt`, `poetry.lock`, `uv.lock`, `Pipfile.lock`, `pom.xml`, `gradle.lockfile`, `go.mod`, `go.sum`, `Gemfile.lock` and `Cargo.lock`).

## Usage

```rust
use std::path::Path;
use pelicode_core::{discover, extract_all};
use pelicode_db::Database;
use pelicode_detect::{Detector, Options, dependencies::Dependencies};

// Use the parameters the database was built with.
let db = Database::open_unchecked("pelicode.db")?;
let config = db.config_from_meta(&Default::default())?;

let files = discover(Path::new("src"))?;
let features: Vec<_> = extract_all(&files, &config).into_iter().collect::<Result<_, _>>()?;

let options = Options {
    dependencies: Dependencies::find(Path::new(".")),
    ..Options::default()
};
let detector = Detector::new(&db, options)?;
for report in detector.detect(&features, |_, _| true)? {
    println!("{}:{}-{} {}", report.path.display(), report.start_line, report.end_line, report.name);
    for c in &report.candidates {
        let o = &c.occurrences[0];
        println!("  {:?} {:.2} {}:{}@{} {}", c.kind, c.similarity, o.ecosystem, o.package, o.version, o.path);
    }
}
```

The `select` closure of `Detector::detect` chooses which functions to check, for example only those touched by a git diff.
To search several databases at once and merge the results, use `detect_all`.

## Related crates

| Crate | Role |
| --- | --- |
| [pelicode](https://crates.io/crates/pelicode) | The `pelicode` command |
| [pelicode-core](https://crates.io/crates/pelicode-core) | Function extraction, normalization and fingerprinting |
| [pelicode-db](https://crates.io/crates/pelicode-db) | SQLite storage of fingerprints and provenance |
| **pelicode-detect** | Search for similar functions |
| [pelicode-crawl](https://crates.io/crates/pelicode-crawl) | Fetching package sources from public repositories |

## License

MIT
