# pelicode

Know where your code came from: the `pelicode` command finds functions in your code that are similar to code published on PyPI, Maven Central, the Go module proxy, RubyGems and crates.io.

Just as dependabot checks dependency versions, pelicode checks the functions in your commits and pull requests against published packages. It helps you notice code that may have slipped in from elsewhere, for example through AI-generated code or contributions from outside the project.

> pelicode does **not** decide whether code is plagiarized or violates a license. It shows *similar code candidates to review*, with their sources and licenses; a person decides what they mean.

Supported languages: Go, Java, Kotlin, Python, Ruby and Rust.

## Installation

```bash
cargo install pelicode
```

Rust 1.90 or later is required. Homebrew (`brew install tamada/tap/pelicode`), a container image (`quay.io/tama5/pelicode`) and prebuilt archives are also available; see the [installation guide](https://tamada.github.io/pelicode/install/).

## Quick start

```bash
# 1. Fetch published packages and index their functions into pelicode.db
pelicode crawl pypi requests six
pelicode crawl crates uuid --contact https://example.org/about

# 2. Count how many packages contain each hash, to ignore common code
pelicode stats --refresh

# 3. Check your code
pelicode scan ./src
pelicode scan --git main..HEAD        # only the functions changed since main
```

```
1 function(s) have similar code candidates to review.

hex.go:10-44 parseID (tokens=275)
  similar(N1)  0.81  go:github.com/google/uuid@v1.6.0 uuid.go:68-117 [BSD-3-Clause]
               lines 12-44 <-> 70-108,110-117 (containment 0.89, 80 shared fingerprints)
```

Results can also be written as JSON, CSV, or SARIF for GitHub code scanning (`-f sarif`). `--fail-on-match` makes `scan` exit with status 1 when there are candidates, for CI.

## Commands

| Command | Description |
| --- | --- |
| `crawl` | Fetch packages from a public repository and index them |
| `index` | Index local directories as one package |
| `stats` | Show database statistics and the most common hashes |
| `clean` | Remove data no longer referenced, and shrink the database |
| `snapshot` | Write a read-only copy of the database to distribute |
| `pull` | Fetch a snapshot from an OCI registry into the cache |
| `scan` | Search for similar code candidates |
| `extract` | Show extracted functions and their features (no database) |
| `export` | List the functions stored in the database |
| `languages` | Show the supported languages and extensions |

Run `pelicode <COMMAND> --help` for the options. Options used every time can be kept in a `.pelicode.yaml` at the root of the repository.

## Learn more

- [Documentation](https://tamada.github.io/pelicode/)
- [Repository](https://github.com/tamada/pelicode), with the design and the specification
- [pelicode-action](https://github.com/tamada/pelicode-action), to check pull requests on GitHub

The command is built on the library crates [pelicode-core](https://crates.io/crates/pelicode-core), [pelicode-db](https://crates.io/crates/pelicode-db), [pelicode-detect](https://crates.io/crates/pelicode-detect) and [pelicode-crawl](https://crates.io/crates/pelicode-crawl).

## License

MIT
