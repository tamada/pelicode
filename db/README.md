# pelicode-db

SQLite storage of function fingerprints and their provenance for [pelicode](https://github.com/tamada/pelicode).

pelicode-db stores the features computed by [pelicode-core](https://crates.io/crates/pelicode-core) together with where each function came from: ecosystem, package, version, file path, license, repository and download URL.
**Source code is never stored**, only hashes, locations (lines and byte ranges) and metadata.

## Features

- **Indexing**: [`Database::index`](https://docs.rs/pelicode-db/latest/pelicode_db/struct.Database.html#method.index) stores the functions of one package version. Files are deduplicated by content, so a file shared by many versions is stored once and linked from each.
- **Compatibility checks**: the parameters of the features (k-gram size, window, MinHash size, normalizer version) and the table layout are recorded in the database. Opening it with other parameters fails with `Error::Incompatible` instead of mixing features that cannot be compared.
- **Lookup** of exact hashes, LSH buckets and fingerprint sets, used by [pelicode-detect](https://crates.io/crates/pelicode-detect).
- **Hash statistics**: how many packages contain each hash, so that common code (getters, `equals`, boilerplate) can be ignored or down-weighted during the search.
- **Crawl bookkeeping**: indexed and failed versions, so crawling can resume and skip what is done.
- **Maintenance**: removing unreferenced data, and writing a read-only **snapshot** for distribution.

## Usage

```rust
use pelicode_core::{Config, Language, extract_bytes};
use pelicode_db::{Database, PackageInfo};

let config = Config::default();
let mut db = Database::open("pelicode.db", &config)?;

let source = b"def greet(name):\n    return 'Hello, ' + name\n";
let file = extract_bytes("greet.py".as_ref(), Language::Python, source, &config)?;
let package = PackageInfo {
    ecosystem: "pypi".into(),
    name: "greet".into(),
    version: "1.0".into(),
    license_spdx: Some("MIT".into()),
    ..PackageInfo::default()
};
let summary = db.index(&package, &[file])?;
println!("stored {} function(s)", summary.functions);

db.refresh_hash_stats()?;
println!("{:?}", db.stats()?);
```

To read a database whose parameters are not known in advance, open it with `Database::open_unchecked` and get its [`Config`](https://docs.rs/pelicode-core/latest/pelicode_core/config/struct.Config.html) from `Database::config_from_meta`.

## Related crates

| Crate | Role |
| --- | --- |
| [pelicode](https://crates.io/crates/pelicode) | The `pelicode` command |
| [pelicode-core](https://crates.io/crates/pelicode-core) | Function extraction, normalization and fingerprinting |
| **pelicode-db** | SQLite storage of fingerprints and provenance |
| [pelicode-detect](https://crates.io/crates/pelicode-detect) | Search for similar functions |
| [pelicode-crawl](https://crates.io/crates/pelicode-crawl) | Fetching package sources from public repositories |

## License

MIT
