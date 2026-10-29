# pelicode-core

Function-level feature extraction for [pelicode](https://github.com/tamada/pelicode).

pelicode-core parses source code with [Tree-sitter](https://tree-sitter.github.io/), splits it into function units (functions, methods, lambdas and the like), normalizes each unit at several levels, and turns it into hashes for similarity search.
The pelicode indexer and detector share this crate, so code stored in the database and code being checked always go through the same pipeline.

## What it computes

For each function, at each normalization level:

| Level | What is removed |
| --- | --- |
| N0 | whitespace and comments |
| N1 | identifiers (`$ID`) and literals (`$STR`, `$NUM`, ...) |
| N2 | type names too (`$TYPE`) |
| N3 | everything but the kinds of the AST nodes |

- an **exact hash** (128 bits) of the whole normalized token sequence, for exact matches;
- **winnowing fingerprints** selected from the k-grams of the token sequence, for partial matches;
- a **MinHash signature** and **LSH buckets** built from the fingerprints, to narrow down candidates.

Locations (lines, columns and byte ranges), token and node counts, and the nesting of functions are recorded too.
Source code itself is not kept.

## Supported languages

Go, Java, Kotlin, Python, Ruby and Rust (see [`Language`](https://docs.rs/pelicode-core/latest/pelicode_core/language/enum.Language.html)).

## Usage

```rust
use pelicode_core::{Config, Language, Level, extract_source};

let source = br#"
def add(a, b):
    return a + b
"#;
let functions = extract_source(Language::Python, source, &Config::default())?;
for f in &functions {
    println!(
        "{} lines {}-{} tokens={} N1={:?}",
        f.name, f.start_line, f.end_line, f.token_count, f.exact_hashes[&Level::N1]
    );
}
```

To process a directory, [`discover`](https://docs.rs/pelicode-core/latest/pelicode_core/scan/fn.discover.html) finds the supported source files (respecting `.gitignore`) and [`extract_all`](https://docs.rs/pelicode-core/latest/pelicode_core/scan/fn.extract_all.html) extracts them in parallel:

```rust
use std::path::Path;
use pelicode_core::{Config, discover, extract_all};

let files = discover(Path::new("src"))?;
for result in extract_all(&files, &Config::default()) {
    let file = result?;
    println!("{}: {} function(s)", file.path.display(), file.functions.len());
}
```

[`Config`](https://docs.rs/pelicode-core/latest/pelicode_core/config/struct.Config.html) holds the parameters (minimum function size, k-gram size, winnowing window, MinHash and LSH sizes, and the levels that get fingerprints).
Features are comparable only when they were computed with the same parameters and the same `NORMALIZER_VERSION`.

## Related crates

| Crate | Role |
| --- | --- |
| [pelicode](https://crates.io/crates/pelicode) | The `pelicode` command |
| **pelicode-core** | Function extraction, normalization and fingerprinting |
| [pelicode-db](https://crates.io/crates/pelicode-db) | SQLite storage of fingerprints and provenance |
| [pelicode-detect](https://crates.io/crates/pelicode-detect) | Search for similar functions |
| [pelicode-crawl](https://crates.io/crates/pelicode-crawl) | Fetching package sources from public repositories |

## License

MIT
