# pelicode-crawl

Fetches package sources from public repositories and indexes their functions, for [pelicode](https://github.com/tamada/pelicode).

pelicode-crawl resolves a package version on a public repository, downloads its source archive, verifies its digest, extracts the functions with [pelicode-core](https://crates.io/crates/pelicode-core) and stores them with their provenance and license in a [pelicode-db](https://crates.io/crates/pelicode-db) database.
The archive is processed in memory, and its source code is not kept.

## Supported repositories

| Ecosystem | Package specification | What is fetched | Integrity check |
| --- | --- | --- | --- |
| PyPI | `NAME[@VERSION]` | sdist (or a wheel when there is none) | SHA-256 |
| Maven Central | `GROUP:ARTIFACT[:VERSION]` | `-sources.jar` (Java and Kotlin) | SHA-1 |
| Go module proxy | `MODULE[@VERSION]` | module zip | `h1:` hash from sum.golang.org |
| RubyGems | `NAME[@VERSION]` | `.gem` | SHA-256 |
| crates.io | `NAME[@VERSION]` | `.crate` | SHA-256 |

Without a version, the latest release is fetched.
Licenses come from the repository metadata; for Go modules, which have none, the license file is identified roughly by its text.

## Being polite to the repositories

The [`Client`](https://docs.rs/pelicode-crawl/latest/pelicode_crawl/http/struct.Client.html) waits between requests and identifies itself in the `User-Agent`.
Pass a contact URL or e-mail address when crawling many packages; crates.io in particular asks crawlers for one.
Versions already in the database are skipped unless re-indexing is forced, and archives larger than `MAX_ARCHIVE_SIZE` are not downloaded.

## Usage

```rust
use std::time::Duration;
use pelicode_core::Config;
use pelicode_crawl::{Client, Crawler, Ecosystem, Outcome};
use pelicode_db::Database;

let config = Config::default();
let mut db = Database::open("pelicode.db", &config)?;
let client = Client::new(Some("https://example.org/about"), Duration::from_secs(1));
let crawler = Crawler::new(client, config, false);

for spec in ["six@1.17.0", "requests"] {
    match crawler.crawl(&mut db, Ecosystem::PyPI, spec)? {
        Outcome::Indexed { package, summary, .. } => {
            println!("{}@{}: {} function(s)", package.name, package.version, summary.functions)
        }
        Outcome::Skipped { package } => println!("{}@{}: already indexed", package.name, package.version),
    }
}
```

The repository adapters (`pypi`, `maven`, `go`, `rubygems`, `crates`) can also be used on their own, for example to resolve versions.
`pypi::top_packages` and `ecosystems::top_packages` list the best-known packages of a repository, as used by `pelicode crawl --top`.

## Related crates

| Crate | Role |
| --- | --- |
| [pelicode](https://crates.io/crates/pelicode) | The `pelicode` command |
| [pelicode-core](https://crates.io/crates/pelicode-core) | Function extraction, normalization and fingerprinting |
| [pelicode-db](https://crates.io/crates/pelicode-db) | SQLite storage of fingerprints and provenance |
| [pelicode-detect](https://crates.io/crates/pelicode-detect) | Search for similar functions |
| **pelicode-crawl** | Fetching package sources from public repositories |

## License

MIT
