---
title: "🏃 Usage"
description: "Every command of pelicode, its options, its output, and how to tune the search."
date: 2026-10-17
draft: false
---

[ [📋 Synopsis](#-synopsis) | [🧰 Commands](#-commands) | [📝 Settings File](#-settings-file) | [🤖 GitHub Action](#-github-action) | [🎛️ Feature Parameters](#-feature-parameters) | [📤 Output Formats](#-output-formats) | [🚦 Exit Status](#-exit-status) | [🔧 Tuning the Search](#-tuning-the-search) ]

---

## 📋 Synopsis

```
pelicode <COMMAND> [OPTIONS]

Commands:
  crawl      Fetches packages from a public repository (pypi, maven, go, rubygems, crates) and indexes them
  index      Extracts the functions of every supported source file under the given directories and stores their features in the database
  stats      Shows database statistics and the hashes shared by the most packages
  clean      Removes files and functions no package version refers to any more, and shrinks the database file
  snapshot   Writes a copy of the database for checking only, to be distributed
  scan       Reports stored functions similar to the functions of the given code
  extract    Extracts the functions of the given files or directories and prints their features
  export     Prints the functions stored in the database
  languages  Lists the supported languages and their file extensions
```

A typical session builds a database once and scans many times:

```sh
pelicode crawl pypi --top 100        # build the reference database
pelicode stats --refresh             # count common hashes
pelicode scan ./src                  # check code
```

Every command that uses a database takes `--db <FILE>`; the default is the `database` of the [settings file](#-settings-file), else `pelicode.db` in the current directory. `pelicode <COMMAND> --help` lists all options of a command.

---

## 🧰 Commands

### `crawl`

Fetches packages from a public repository and indexes them.

```
pelicode crawl [OPTIONS] <ECOSYSTEM> [PACKAGES]...
```

| Ecosystem | Package syntax | Example |
| --- | --- | --- |
| `pypi` | `NAME[@VERSION]` | `requests`, `numpy@2.1.0` |
| `maven` | `GROUP:ARTIFACT[:VERSION]` | `com.google.code.gson:gson:2.11.0` |
| `go` | `MODULE[@VERSION]` | `golang.org/x/mod@v0.20.0` |
| `rubygems` | `NAME[@VERSION]` | `thor@1.3.0` |
| `crates` | `NAME[@VERSION]` | `uuid`, `serde_json@1.0.140` |

Without a version, the latest release is fetched.

| Option | Default | Description |
| --- | --- | --- |
| `--list <FILE>` | | Read packages from a file, one per line. `#` starts a comment |
| `--top <N>` | | Also crawl the N most downloaded packages (`pypi` and `crates` only) |
| `--force` | | Re-index versions that are already in the database |
| `--refresh-stats` | | Recount common hashes afterwards (see [`stats`](#stats)) |
| `--delay-ms <MS>` | 1000 | Minimum interval between two requests |
| `--contact <URL or E-MAIL>` | | Contact added to the User-Agent |

It also takes the [feature parameters](#-feature-parameters).

```sh
pelicode crawl pypi requests numpy@2.1.0
pelicode crawl pypi --top 100 --contact https://example.org/about
pelicode crawl maven --list maven-packages.txt --refresh-stats
pelicode crawl go github.com/google/uuid
pelicode crawl rubygems rack thor@1.3.0
pelicode crawl crates --top 100 --contact https://example.org/about
```

What is fetched from each repository:

| Ecosystem | Archive | Checked against | License from |
| --- | --- | --- | --- |
| PyPI | sdist, or a wheel when there is none | SHA-256 published by PyPI | `License-Expression`, classifiers or `License` |
| Maven Central | `-sources.jar` (Java and Kotlin files) | `.sha1` file | `<licenses>` of the POM or of its parents |
| Go | Module zip from proxy.golang.org | `h1:` hash from sum.golang.org | Text of the `LICENSE` file |
| RubyGems | `.gem` file | SHA-256 published by RubyGems | `licenses` of the gem |
| crates.io | `.crate` file from static.crates.io | SHA-256 checksum published by crates.io | `license` of the version (SPDX) |

Good to know:

- Archives are read in memory and discarded. Nothing is unpacked to disk, and no source code is stored.
- Versions already indexed are skipped, so an interrupted crawl can simply be run again.
- A failed package does not stop the others. Failures are recorded in the database, and the exit status is 1 if any package failed.
- Requests are sent one at a time. Be gentle with public services: keep the delay, and set `--contact` when crawling many packages.
- For PyPI, the ranking used by `--top` comes from [hugovk/top-pypi-packages](https://github.com/hugovk/top-pypi-packages), not from PyPI itself; for crates.io, it is the all-time download count of crates.io. Maven Central, the Go module proxy and RubyGems have no such ranking; use `--list` for them.
- The [crates.io data access policy](https://crates.io/data-access) allows at most one API request per second and strongly suggests a way to contact you. Give `--contact` when crawling crates.io, and keep `--delay-ms` at 1000 or more.

### `index`

Indexes local directories (or files) as one package.

```
pelicode index [OPTIONS] <PATHS>...
```

| Option | Default | Description |
| --- | --- | --- |
| `--ecosystem <NAME>` | `local` | Ecosystem recorded for the package |
| `--name <NAME>` | name of the first directory | Package name |
| `--version <VERSION>` | `unversioned` | Package version |
| `--license <SPDX>` | | License of the package |
| `--repository <URL>` | | Repository URL of the package |
| `--refresh-stats` | | Recount common hashes afterwards |
| `-f, --format <FORMAT>` | `text` | Summary format: `text` or `json` |

It also takes the [feature parameters](#-feature-parameters).

```sh
pelicode index ./my-project --name my-project --version 1.0 --license MIT
```

Hidden files and, in Git repositories, files ignored by `.gitignore` are skipped (this applies to `scan` and `extract` too). Indexing the same name and version again replaces the files recorded for it. When several directories are given, each file path is prefixed with the name of its directory.

### `stats`

Shows the size of the database, recounts how many packages contain each hash, and lists the most common ones.

```
pelicode stats [OPTIONS]
```

| Option | Default | Description |
| --- | --- | --- |
| `--refresh` | | Recount common hashes before showing them |
| `--full` | | With `--refresh`, recount every hash rather than only those of what was indexed since |
| `--top <N>` | 10 | Number of common hashes to list |
| `--level <LEVEL>` | `N1` | Normalization level of the listed hashes |
| `--min-tokens <N>` | 50 | List exact hashes of functions with at least N tokens only |
| `-f, --format <FORMAT>` | `text` | `text` or `json` |

```
4 ecosystems, 5 packages, 5 versions, 373 files, 6550 functions (1654 with fingerprints)
hash statistics: 2026-10-17T08:27:18Z; 21 exact hashes and 354 fingerprints in 2+ packages

most common fingerprints (N1):
       4 packages      22 functions  7d679b3567a1201f  pypi:requests@2.34.2 tests/test_requests.py:1510-1510 test_iter_content_wraps_exceptions
```

The counts are not updated when packages are added. Run `pelicode stats --refresh`, or pass `--refresh-stats` to `crawl` or `index`; `scan` warns when the counts are out of date.

Once counted, the statistics are refreshed **incrementally**: only the hashes of the files added to or removed from a version since the last count are recounted, which gives the same result as recounting everything in a fraction of the time. `--refresh --full` recounts everything anyway. The first count, and the first after an upgrade from a database whose statistics were out of date, are always full.

### `clean`

Removes the files no package version refers to any more, with their functions and features, and shrinks the database file. Such files are left behind when a version is indexed again (`index` of the same name and version, or `crawl --force`).

```
pelicode clean [OPTIONS]
```

| Option | Default | Description |
| --- | --- | --- |
| `--dry-run` | | Only report what would be removed |
| `--no-vacuum` | | Do not shrink the file; shrinking (SQLite `VACUUM`) needs as much free disk space as the database takes |
| `-f, --format <FORMAT>` | `text` | `text` or `json` |

```
removed 3 files and 41 functions no longer referenced
database: 221.3 MB -> 213.5 MB
```

The hash statistics stay valid: they count only files that versions refer to.

### `snapshot`

Writes a copy of the database for checking only, to be distributed: building a reference database takes hours, so it is built in one place and others only `scan` against a snapshot of it.

```
pelicode snapshot -o <FILE> [OPTIONS]
```

| Option | Default | Description |
| --- | --- | --- |
| `-o, --out <FILE>` | | The snapshot file to write |
| `--ecosystem <ECOSYSTEM>` | all | Only these ecosystems (repeatable or comma-separated) |
| `--latest` | | Only the latest version of each package, by publication time |
| `--force` | | Replace the snapshot file if it exists |
| `-f, --format <FORMAT>` | `text` | `text` or `json` |

```
pypi.db: 26 packages, 26 versions, 77084 functions, 165.5 MB
```

A snapshot keeps everything `scan` needs, with the hash statistics recounted for what it holds, and drops the MinHash signatures, which only indexing and `export` use (`export` shows none for a snapshot). It is a single file; nothing can be indexed into it. The database it is made from is not changed.

Snapshots are published in quay.io as [OCI artifacts](https://oras.land/), compressed with zstd (to about a third), tagged `<NAME>-YYYYMMDD` and `<NAME>-latest`. With [ORAS](https://oras.land/docs/installation) and zstd installed:

```sh
pelicode snapshot -o pypi.db --ecosystem pypi --latest
oras login quay.io
just snapshot_push pypi.db pypi                 # quay.io/tama5/pelicode-db:pypi-YYYYMMDD and :pypi-latest
just snapshot_pull pypi-latest pypi.db          # elsewhere: fetch and decompress
pelicode scan ./src --db pypi.db
```

### `scan`

Searches the database for functions similar to those of the given code.

```
pelicode scan [OPTIONS] [PATHS]...
```

| Option | Default | Description |
| --- | --- | --- |
| `--git <RANGE>` | | Check only the functions changed in a git range. The path, if any, is the repository (default `.`) |
| `--threshold <X>` | 0.7 | Minimum [similarity](/glossary#similarity) to report |
| `--min-tokens <N>` | 50 | Functions with fewer tokens are not checked |
| `--max-candidates <N>` | 5 | Candidates reported per function |
| `--languages <LANGS>` | all | Check only these languages, comma-separated |
| `--exclude <ECOSYSTEM:NAME>` | | Never report this package; repeatable |
| `--exclude-path <GLOB>` | | Do not check files whose path matches; `*` stays within a directory, `**` crosses directories; repeatable |
| `--stop-packages <N or P%>` | 100 | Ignore hashes found in at least N packages, or in P% of them; 0 disables |
| `--min-shared <N>` | 5 | Shared fingerprints required for a similar match |
| `--idf` | | Weight fingerprints by inverse document frequency |
| `--fail-on-match` | | Exit with status 1 when any candidate is found |
| `--no-exclude-dependencies` | | Also report candidates found only in the dependencies of the checked code |
| `-f, --format <FORMAT>` | `text` | `text`, `json`, `csv` or `sarif` |
| `-o, --output <FILE>` | standard output | Write the result to a file |

```sh
pelicode scan ./src
pelicode scan --git main..HEAD                    # functions changed between main and HEAD
pelicode scan --git main...feature                # changed on feature since it left main
pelicode scan --git HEAD                          # uncommitted changes in the working tree
pelicode scan ./src --exclude local:my-project    # do not report the project itself
pelicode scan ./src -f json -o report.json
pelicode scan --git origin/main...HEAD -f sarif -o pelicode.sarif   # for GitHub code scanning
```

`scan` reads the feature parameters from the database, so your code is always hashed exactly like the stored code. The search runs in stages:

1. **Exact matches**, comparing the hash of the whole function after N0, N1 and N2 normalization, in that order.
2. **Candidates** from [LSH buckets](/glossary#lsh), which bring together functions with similar fingerprint sets.
3. **Similarity** of the fingerprint sets of each candidate, after removing common fingerprints; candidates below `--threshold` or sharing fewer than `--min-shared` fingerprints are dropped.

#### Dependencies are left out

Code that matches a library the checked project depends on is expected, so `scan` leaves it out. The dependencies are read from `Cargo.lock`, `go.mod`, `go.sum`, `requirements*.txt`, `poetry.lock`, `uv.lock`, `Pipfile.lock`, `Gemfile.lock`, `pom.xml` and `gradle.lockfile` found in the checked directories and below, and in the directories above them up to the root of the git repository (so `scan ./src` reads the lock files at the root of the project).

- A candidate found only in dependencies is not reported, and `scan` says how many were left out, on the standard error.
- A candidate found in a dependency and elsewhere is reported, without the occurrences in the dependency.
- `--no-exclude-dependencies`, or `exclude_dependencies: false` in the settings file, reports them all; for a license audit, it also shows code copied from dependencies into the repository.

### `extract`

Prints the functions found in files or directories and their features, without using a database. Useful to see what pelicode sees.

```
pelicode extract [OPTIONS] <PATHS>...
```

| Option | Default | Description |
| --- | --- | --- |
| `--tokens [<LEVEL>]` | | Include the normalized tokens (of LEVEL, `N1` by default, in text and CSV; of every level in JSON) |
| `-f, --format <FORMAT>` | `text` | `text`, `json` or `csv` |
| `-o, --output <FILE>` | standard output | Write the result to a file |

It also takes the [feature parameters](#-feature-parameters).

```sh
pelicode extract upload.py --tokens N1
```

```
upload.py:6-43	python	total_length	function_definition	tokens=202	N1=87ea5b9cd905f4c2
    N1: def $ID ( $ID ) : $ID = None $ID = $NUM if $ID ( $ID , $ID ) : $ID = $ID . $ID ( $STR ) ...
```

Functions marked `(too small)` have fewer tokens than `--min-tokens` and get no fingerprints.

### `export`

Prints every function in the database with its package, file, location, size and exact hashes.

```
pelicode export [OPTIONS]
```

| Option | Default | Description |
| --- | --- | --- |
| `-f, --format <FORMAT>` | `text` | `text`, `json` or `csv` |
| `-o, --output <FILE>` | standard output | Write the result to a file |

### `languages`

Lists the supported languages and their file extensions.

```
go       go
java     java
kotlin   kt, kts
python   py
ruby     rb
rust     rs
```

---

## 📝 Settings File

Options used every time, such as the database, the thresholds and what to leave out, can be written in a settings file at the root of the repository, in YAML or JSON5:

```yaml
# .pelicode.yaml
version: 1
database: data/pelicode.db        # relative to this file
scan:
  languages: [python, rust]
  threshold: 0.6
  stop_packages: 10%
  fail_on_match: true
  exclude:
    packages: ["local:my-project"]
    paths: ["vendor/**", "**/testdata/**"]
crawl:
  contact: https://example.org/about
```

```json5
// .pelicode.json5
{
  version: 1,
  database: 'data/pelicode.db',
  scan: {
    languages: ['python', 'rust'],
    threshold: 0.6,
    stop_packages: '10%',
    fail_on_match: true,
    exclude: { packages: ['local:my-project'], paths: ['vendor/**', '**/testdata/**'] },
  },
  crawl: { contact: 'https://example.org/about' },
}
```

**Where it is found**: `.pelicode.yaml`, `.pelicode.yml`, `.pelicode.json5` or `.pelicode.json` (read as JSON5), in the current directory or the nearest directory above it, up to the root of the git repository. Several in the same directory are an error. `--config <FILE>` names one explicitly, and `--no-config` ignores settings files; both work with every command.

**Precedence**: an option on the command line wins over the settings file, which wins over the built-in default. `--exclude` and `--exclude-path` are added to the lists of the settings file. Flags such as `--idf` and `--fail-on-match` can only switch a setting on; to switch off one set in the file, use `--no-config` or edit the file.

| Key | Option | Description |
| --- | --- | --- |
| `version` | | Version of the format; must be `1` (required) |
| `database` | `--db` | Database file, relative to the settings file |
| `scan.languages` | `--languages` | Languages to check |
| `scan.threshold` | `--threshold` | Minimum similarity, from 0 to 1 |
| `scan.min_tokens` | `--min-tokens` | Smallest function checked |
| `scan.max_candidates` | `--max-candidates` | Candidates per function |
| `scan.min_shared` | `--min-shared` | Shared fingerprints required |
| `scan.stop_packages` | `--stop-packages` | A number, or a percentage such as `"10%"` |
| `scan.idf` | `--idf` | `true` or `false` |
| `scan.fail_on_match` | `--fail-on-match` | `true` or `false` |
| `scan.exclude_dependencies` | `--no-exclude-dependencies` | `false` to report candidates found only in dependencies (default `true`) |
| `scan.exclude.packages` | `--exclude` | Packages never reported, as `ECOSYSTEM:NAME` |
| `scan.exclude.paths` | `--exclude-path` | Glob patterns of files not checked |
| `crawl.contact` | `--contact` | Contact added to the User-Agent |
| `crawl.delay_ms` | `--delay-ms` | Minimum interval between requests |

Unknown keys are errors, so that a misspelled key does not go unnoticed; the message names the file and, for YAML, the line.

The paths matched by `exclude.paths` are the paths `scan` reports: relative to the directory given, or to the repository with `--git`.

---

## 🤖 GitHub Action

The repository is also a GitHub Action that checks the functions changed by a pull request or a push, shows the candidates in the pull request through GitHub code scanning, and lists them in the summary of the job.

```yaml
# .github/workflows/pelicode.yaml
name: pelicode
on: pull_request

permissions:
  contents: read
  security-events: write   # to upload the results to code scanning

jobs:
  pelicode:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0       # pelicode compares the pull request with its base
      - uses: tamada/pelicode@v0.1.0
        with:
          database-url: https://example.org/pelicode/pelicode.db
```

What it does:

1. **Installs pelicode**: the release of the action's tag (`tamada/pelicode@v0.1.0` installs pelicode 0.1.0); used at a branch, it builds the action's own source with Cargo.
2. **Downloads the database** from `database-url`, if given.
3. **Scans** the changes of the pull request (`base...head`) or of the push (`before..after`); other events, and pushes creating a branch, check the whole repository. The settings file of the repository applies.
4. **Uploads** the SARIF log to code scanning, where each candidate is a note on the function, and writes the candidates to the summary of the job.
5. **Fails** the step after the upload, only with `fail-on-match: true`.

| Input | Default | Description |
| --- | --- | --- |
| `database` | settings file, else `pelicode.db` | Database file |
| `database-url` | | URL to download the database from |
| `config` | `.pelicode.yaml` and the like | Settings file |
| `range` | from the event | Git range to check |
| `paths` | | Files or directories to check instead of a git range |
| `working-directory` | `.` | Directory to run in |
| `args` | | Other options of `pelicode scan`, such as `--threshold 0.6` |
| `fail-on-match` | `false` | Fail when candidates are found |
| `upload-sarif` | `true` | Upload to code scanning |
| `sarif-file` | `pelicode.sarif` | Where to write the SARIF log |
| `pelicode-version` | the action's tag, else `source` | Release to install, or `source` |
| `pelicode-path` | | An installed binary to use |

The outputs are `candidates`, the number of functions with candidates, and `sarif-file`.

{{< alert icon="circle-info" >}}
Uploading to code scanning needs `security-events: write`, which pull requests from forks do not get; set `upload-sarif: false` for them, and the candidates are still listed in the summary. Code scanning is free for public repositories; private repositories need GitHub Advanced Security.
{{< /alert >}}

---

## 🎛️ Feature Parameters

`crawl`, `index` and `extract` take these options. They decide how functions are hashed, so a database records the values it was built with: adding to it with different values is refused, and `scan` uses the recorded values automatically.

| Option | Default | Description |
| --- | --- | --- |
| `--kgram <K>` | 10 | Length of the [k-grams](/glossary#k-gram) turned into fingerprints |
| `--window <W>` | 4 | Window size of [winnowing](/glossary#winnowing) |
| `--minhash-size <N>` | 128 | Length of [MinHash](/glossary#minhash) signatures |
| `--bands <B>` | 32 | Number of [LSH](/glossary#lsh) bands; must divide `--minhash-size` |
| `--levels <LEVELS>` | `N1` | [Normalization levels](/glossary#normalization-level) with fingerprints, such as `N1,N2` |
| `--ident-mode <MODE>` | `uniform` | `uniform` turns every identifier into `$ID`; `indexed` numbers them by first appearance (`$ID1`, `$ID2`, ...) |
| `--min-tokens <N>` | 50 | Functions with fewer tokens get exact hashes only |

---

## 📤 Output Formats

### Text

The text output of `scan` lists each function that has candidates, followed by its candidates:

```
upload.py:6-43 total_length (tokens=202)
  similar(N1)  0.72  pypi:requests@2.34.2 src/requests/utils.py:160-228 [Apache-2.0]
               lines 9-23,26-43 <-> 164-189,199-228 (containment 0.86, 62 shared fingerprints)
```

The kind of match is `exact(LEVEL)` when the whole function is identical after normalization at that level, or `similar(LEVEL)` when their fingerprint sets are similar. A candidate found in several packages or versions shows the earliest published one followed by `(+N more)`.

### JSON

`scan -f json` writes an array with one object per checked function that has candidates:

| Field | Description |
| --- | --- |
| `path`, `language`, `name`, `start_line`, `end_line`, `token_count` | The checked function |
| `candidates[].match` | `{"type": "exact" or "similar", "level": "N0" ... "N3"}` |
| `candidates[].similarity` | 1.0 for exact matches; the (weighted) Jaccard similarity of fingerprint sets otherwise |
| `candidates[].containment` | Share of the checked function's fingerprints found in the candidate |
| `candidates[].package_count` | For exact matches, the number of packages containing the same normalized function |
| `candidates[].shared_fingerprints` | Number of shared fingerprints |
| `candidates[].matched_lines`, `candidate_lines` | Matching line ranges, as `[start, end]` pairs, in the checked function and in the candidate |
| `candidates[].function` | The stored function: `name`, `kind`, `language`, `start_line`, `end_line`, `start_byte`, `end_byte`, `token_count` |
| `candidates[].occurrences` | Where it was found, earliest publication first: `ecosystem`, `package`, `version`, `path`, `license`, `published_at`, `repository_url`, `archive_url`, `content_sha256` |
| `candidates[].occurrence_count` | Number of places it was found |

`extract -f json` writes the full features of every function, including fingerprints, MinHash signatures and LSH buckets, which is useful for research.

### SARIF

`scan -f sarif` writes a [SARIF 2.1.0](https://docs.oasis-open.org/sarif/sarif/v2.1.0/sarif-v2.1.0.html) log, which GitHub code scanning (with `github/codeql-action/upload-sarif`) and other tools read:

- one result per checked function with candidates, located at the function, with the level `note`, since a candidate is something to review rather than a defect;
- the rule `exact-match` or `similar-match`, after the best candidate;
- a message naming the best candidate, its package, version, file, lines and license, the matched lines, and the other candidates;
- the details of every candidate in `properties.candidates`;
- `partialFingerprints`, so that code scanning keeps one alert for a function across runs even when its lines move.

File paths are written relative to the current directory, which should be the root of the repository: `pelicode scan ./src` reports `src/upload.py`, not `upload.py`.

### CSV

`scan -f csv` writes one row per candidate with the columns `path`, `name`, `start_line`, `end_line`, `token_count`, `match`, `similarity`, `containment`, `shared_fingerprints`, `matched_lines`, `candidate_ecosystem`, `candidate_package`, `candidate_version`, `candidate_path`, `candidate_name`, `candidate_start_line`, `candidate_end_line`, `candidate_lines`, `candidate_license` and `occurrence_count`.

`extract -f csv` and `export -f csv` write one row per function, with its location, size and the exact hashes `n0` to `n3`.

---

## 🚦 Exit Status

| Status | Meaning |
| --- | --- |
| 0 | Success |
| 1 | An error occurred; `crawl` failed for at least one package; or `scan --fail-on-match` found a candidate |

---

## 🔧 Tuning the Search

- **Fewer missed copies**: lower `--threshold` (for example to 0.5) to catch code that was edited more heavily.
- **Fewer false alarms from boilerplate**: keep the hash counts up to date with `stats --refresh`, and lower `--stop-packages`. The default of 100 packages suits a large database; with a few dozen packages, give a percentage such as `--stop-packages 40%`.
- **Short functions**: functions under 50 tokens are not checked, because short code matches by chance too often. Lowering `--min-tokens` finds more, at the cost of more noise.
- **Your own code**: exclude your own package with `--exclude local:NAME` when it is in the database.
- **Changing feature parameters** such as `--kgram` requires a new database. Keep one database per set of parameters.
