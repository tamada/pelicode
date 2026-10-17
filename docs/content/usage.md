---
title: "🏃 Usage"
description: "Every command of pelicode, its options, its output, and how to tune the search."
date: 2026-10-17
draft: false
---

[ [📋 Synopsis](#-synopsis) | [🧰 Commands](#-commands) | [🎛️ Feature Parameters](#-feature-parameters) | [📤 Output Formats](#-output-formats) | [🚦 Exit Status](#-exit-status) | [🔧 Tuning the Search](#-tuning-the-search) ]

---

## 📋 Synopsis

```
pelicode <COMMAND> [OPTIONS]

Commands:
  crawl      Fetches packages from a public repository (pypi, maven, go, rubygems) and indexes them
  index      Extracts the functions of every supported source file under the given directories and stores their features in the database
  stats      Shows database statistics and the hashes shared by the most packages
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

Every command that uses a database takes `--db <FILE>`; the default is `pelicode.db` in the current directory. `pelicode <COMMAND> --help` lists all options of a command.

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

Without a version, the latest release is fetched.

| Option | Default | Description |
| --- | --- | --- |
| `--list <FILE>` | | Read packages from a file, one per line. `#` starts a comment |
| `--top <N>` | | Also crawl the N most downloaded packages (`pypi` only) |
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
```

What is fetched from each repository:

| Ecosystem | Archive | Checked against | License from |
| --- | --- | --- | --- |
| PyPI | sdist, or a wheel when there is none | SHA-256 published by PyPI | `License-Expression`, classifiers or `License` |
| Maven Central | `-sources.jar` (Java and Kotlin files) | `.sha1` file | `<licenses>` of the POM or of its parents |
| Go | Module zip from proxy.golang.org | `h1:` hash from sum.golang.org | Text of the `LICENSE` file |
| RubyGems | `.gem` file | SHA-256 published by RubyGems | `licenses` of the gem |

Good to know:

- Archives are read in memory and discarded. Nothing is unpacked to disk, and no source code is stored.
- Versions already indexed are skipped, so an interrupted crawl can simply be run again.
- A failed package does not stop the others. Failures are recorded in the database, and the exit status is 1 if any package failed.
- Requests are sent one at a time. Be gentle with public services: keep the delay, and set `--contact` when crawling many packages.
- The ranking used by `--top` comes from [hugovk/top-pypi-packages](https://github.com/hugovk/top-pypi-packages), not from PyPI itself. Maven Central, the Go module proxy and RubyGems have no such ranking; use `--list` for them.

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
| `--exclude <ECOSYSTEM:NAME>` | | Never report this package; repeatable |
| `--stop-packages <N or P%>` | 100 | Ignore hashes found in at least N packages, or in P% of them; 0 disables |
| `--min-shared <N>` | 5 | Shared fingerprints required for a similar match |
| `--idf` | | Weight fingerprints by inverse document frequency |
| `--fail-on-match` | | Exit with status 1 when any candidate is found |
| `-f, --format <FORMAT>` | `text` | `text`, `json` or `csv` |
| `-o, --output <FILE>` | standard output | Write the result to a file |

```sh
pelicode scan ./src
pelicode scan --git main..HEAD                    # functions changed between main and HEAD
pelicode scan --git main...feature                # changed on feature since it left main
pelicode scan --git HEAD                          # uncommitted changes in the working tree
pelicode scan ./src --exclude local:my-project    # do not report the project itself
pelicode scan ./src -f json -o report.json
```

`scan` reads the feature parameters from the database, so your code is always hashed exactly like the stored code. The search runs in stages:

1. **Exact matches**, comparing the hash of the whole function after N0, N1 and N2 normalization, in that order.
2. **Candidates** from [LSH buckets](/glossary#lsh), which bring together functions with similar fingerprint sets.
3. **Similarity** of the fingerprint sets of each candidate, after removing common fingerprints; candidates below `--threshold` or sharing fewer than `--min-shared` fingerprints are dropped.

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
```

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
