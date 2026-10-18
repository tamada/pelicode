# Contributing to pelicode

English | [日本語](.github/docs/CONTRIBUTING.ja.md)

Thank you for your interest in pelicode. This guide explains how the project is organized, how to build and test it, and what to keep in mind when adding a language or a package repository. For what pelicode is and why, read the [README](README.md), the [usage scenarios](.github/docs/scenario.md) and the [specification](.github/docs/spec.md).

pelicode reports **candidates to review**, never verdicts of plagiarism or license infringement. Please keep that stance in code, messages and documentation.

## Development Environment

- [Rust](https://www.rust-lang.org/tools/install) 1.90 or later, and a C compiler (the Tree-sitter grammars and the bundled SQLite are compiled from C).
- [Git](https://git-scm.com/), used by `pelicode scan --git` and by the tests.
- [just](https://just.systems/) for the tasks below.
- Optional: [Hugo](https://gohugo.io/) extended 0.163 to 0.166 for the documentation site, Docker for the container image, [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) for coverage, [actionlint](https://github.com/rhysd/actionlint) for workflows.

| Task | What it does |
| --- | --- |
| `just test` | Format, clippy, build, then the tests with coverage (`target/coverage.lcov`) |
| `just test_without_coverage` | The same without coverage |
| `just clippy` | `cargo clippy --workspace --all-targets -- -D warnings` |
| `just verify_format` | `cargo fmt --all --check`, as CI does |
| `just doc` | The API documentation, failing on warnings such as broken links |
| `just publish_check` | `cargo publish --workspace --dry-run` (needs a clean working tree) |
| `just docker` | Build the container image |
| `just site` / `just start` | Build or serve the documentation site |

The tests never access the network. Commands that do (`pelicode crawl`) are tried by hand; please be gentle with the public services (see [Adding a Package Repository](#adding-a-package-repository)).

## Layout

The repository is a Cargo workspace; each crate is a directory at the root:

```
core  ←  db  ←  detect  ←  cli
          ↑                  ↑
          └──── crawl ───────┘
```

| Crate | Role |
| --- | --- |
| `core` (pelicode-core) | Parsing with Tree-sitter, function extraction, normalization, hashes, fingerprints, MinHash, LSH, file discovery |
| `db` (pelicode-db) | SQLite storage, lookups, compatibility checks, hash statistics, cleanup |
| `detect` (pelicode-detect) | Candidate search, similarity, dependencies of the checked code |
| `crawl` (pelicode-crawl) | Fetching packages from public repositories |
| `cli` (pelicode-cli) | The `pelicode` command, settings files, git diffs, output formats, SARIF |

Other places: `.github/workflows/` (CI and releases), `.github/docs/` (specification and scenarios), `docs/` (the Hugo site), `Containerfile`, `Justfile`. The GitHub Action is in its own repository, [tamada/pelicode-action](https://github.com/tamada/pelicode-action).

## Making a Change

1. Open or pick an [issue](https://github.com/tamada/pelicode/issues), so that the direction can be discussed first for anything larger than a fix.
2. Work on a branch; `main` only changes through pull requests.
3. Keep commits focused, with a subject in the imperative ("Add ...", "Fix ...") and a body that says why, not only what. `Closes #N` in a commit or the pull request closes the issue when merged.
4. Before opening a pull request, run `just test` and `just doc`. CI runs format, clippy, build and tests on Linux, macOS and Windows (x64 and arm64).
5. Update the documentation that the change affects (see [Documentation](#documentation)).

### Code Style

- `cargo fmt` formatting and no clippy warnings (`-D warnings`).
- Code, comments, messages and documentation are in English; READMEs, the specification and the scenarios also have Japanese versions (`*.ja.md` in `.github/docs`) that should change together.
- Comments explain why, sparingly; names explain what.
- Paths that are recorded or reported use `/` on every platform (`pelicode_core::slash_path`).
- Errors name what failed and where (the file, the URL, the key of a settings file).

## Adding a Language

A language is a Tree-sitter grammar plus a `LanguageSpec` that names its node kinds.

1. Add the grammar crate to `core/Cargo.toml` (prefer the official `tree-sitter-*` grammars; check that it works with the `tree-sitter` version in use).
2. In `core/src/language.rs`:
   - add a variant to `Language`, and to `Language::ALL`, `name()`, `extensions()`, `ts_language()` and `spec()`;
   - add a `static` `LanguageSpec`:

| Field | Meaning |
| --- | --- |
| `functions` | Node kinds that are function units (functions, methods, constructors, lambdas, blocks, closures). Declarations without a body are not functions |
| `comments` | Node kinds of comments, dropped at every level |
| `identifiers` | Variable-like identifiers, which become `$ID` from N1 |
| `type_identifiers` | Type names, kept at N1 and turned into `$TYPE` at N2 |
| `type_contexts` | Ancestors that make a plain identifier a type name (`type` in Python) |
| `strings`, `numbers` | Literals, which become `$STR` and `$NUM` from N1; a string is one token |
| `docstrings` | Whether the first string statement of a body is dropped |
| `optional_semicolons` | Whether `;` is layout (Go, Kotlin, Python, Ruby) or structure (Java, Rust) |

   Inspect the node kinds in the grammar's `src/node-types.json`, or by parsing samples.
3. Add tests to `core/tests/extract.rs`: functions are found with the right names and lines, renaming identifiers changes the N0 hash but not the N1 hash, comments are ignored, nested functions have their parent, and declarations without a body are left out.
4. Add a file of the language to `cli/tests/fixtures/project/` if it helps the end-to-end tests (and update their expectations).
5. If the language has a package repository, see the next section; if it has lock files, teach `detect/src/dependencies.rs` to read them.
6. Update the language tables of the READMEs, the site (`docs/content/`) and the specification (§4.1).

Adding a language changes no existing hash, so `NORMALIZER_VERSION` stays.

## Adding a Package Repository

A repository adapter is a module of `crawl/src/` (see `pypi.rs`, `crates.rs`) with:

- `ECOSYSTEM`: the name recorded in the database (`pypi`, `crates`, ...);
- `resolve(client, name, version)`: fetches the metadata of a release, or of the latest one, and returns a `Release`: the `PackageInfo` (name as the repository spells it, version, license as SPDX where possible, publication time, repository URL, archive URL), the `ArchiveKind`, how many leading path components to strip, and the `Digest` published by the repository;
- `parse_release(json)` (or the like): the parsing, separate from the network, so that it can be tested offline;
- `top_packages(client, n)` if the repository can rank packages by downloads.

Then:

1. Add the variant to `Ecosystem` in `crawl/src/lib.rs` (`name()`, `FromStr`) and to `Crawler::resolve`, and mention it in the help of `cli/src/crawl.rs` (and `--top` if supported).
2. Verify downloads: compare with the digest the repository publishes (`Digest`); add a kind if needed (`verify` in `crawl/src/lib.rs`).
3. Archives are read in memory by `crawl/src/archive.rs`; add a format there if needed. Never write archive contents to disk, and never store source code.
4. If the metadata declares no license, `license::identify_all` reads the license files of the archive.
5. Read the usage policy of the service, and respect it: one request at a time through `Client` (which paces requests and retries), a User-Agent with contact information (`--contact`), and dumps or indexes rather than the API where the service asks for that.
6. Tests: parsing of sample responses (`parse_release`), URL construction, and names; no network. Examples of functions that use the network are `no_run`.
7. Document it: the READMEs, Usage on the site, and the table of §6.2 of the specification.

## Versions of the Hashes and the Database

Two versions protect users from mixing incompatible data. Bump them in the same pull request as the change that needs it.

- **`NORMALIZER_VERSION`** (`core/src/config.rs`): bump when the hashes of existing code would change: normalization rules, extraction of function units, token or hash computation. Databases built before are then refused, and must be rebuilt.
- **`SCHEMA_VERSION`** (`db/src/lib.rs`): bump when the tables change. Add a migration from the previous version to `check_schema` when possible (see the migration from 4 to 5), so that existing databases keep working.
- Feature parameters (`kgram`, `window`, `minhash_size`, `bands`, `ident_mode`) are recorded per database already; changing their defaults needs no version bump, but say so in the documentation.

## Documentation

- **API**: the library crates have `#![warn(missing_docs)]`. Every public item has a doc comment, and every public function an example (`no_run` when it needs the network); `just doc` must pass.
- **Command line**: the doc comments of clap arguments are the text of `--help`; keep them short and say the default.
- **Users**: the READMEs (`README.md`, `.github/docs/README.ja.md`) and the site (`docs/content/`: Usage, Install, Getting Started, Glossary).
- **Design**: the specification (`.github/docs/spec.md`, `spec.ja.md`) describes the implementation; move features from §12.2 (Not Implemented Yet) to §12.1 (Status) as they land. The scenarios (`.github/docs/scenario*.md`) say what works for whom.

## Releases

Releases are made by the maintainer from `release/vX.Y.Z` branches; the workflows build the binaries, the container image and the crates. The procedure is being written (#10).

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
