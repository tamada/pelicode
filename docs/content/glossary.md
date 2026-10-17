---
title: "📖 Glossary"
description: "The terms pelicode uses, each defined once."
date: 2026-10-17
draft: false
---

{{< katex >}}

The terms below are the ones the commands, their `--help` and this site use. The order follows the work: code is **collected**, its functions are **normalized** and **hashed**, hashes are **counted** across packages, and code is **scanned** against them.

---

## 📦 Collecting

### Ecosystem

A public package repository: `pypi`, `maven` (Maven Central), `go` (the Go module proxy) or `rubygems`. Projects registered with `pelicode index` belong to the ecosystem `local` unless another one is given.

### Package

A named unit of an ecosystem: a PyPI project, a Maven `groupId:artifactId`, a Go module path or a gem. Hash counts are per package, so the versions of one package count once.

### Version

One release of a package. `crawl` indexes one version per request, the latest unless a version is given.

### Occurrence

A place where a stored function was found: a file of a version of a package, with its license, publication date and download URL. The same file in several versions is stored once and has several occurrences.

### Reference database

The SQLite file, `pelicode.db` by default, holding the features of every indexed function together with its occurrences. It never holds source code.

---

## 🧩 Normalizing

### Function

The unit pelicode compares: a function, method, constructor, lambda or block, extracted with Tree-sitter. Nested functions are extracted as well as the function around them. Functions with syntax errors are skipped.

### Token

One lexical element of a function after comments and layout are removed: a keyword, an operator, an identifier or a literal. The size of a function is its number of tokens.

### Normalization level

How much a function is abstracted before it is hashed:

| Level | What is removed or abstracted | Survives |
| --- | --- | --- |
| N0 | Comments, whitespace, layout | Reformatting |
| N1 | N0, and identifiers become `$ID`, literals `$STR`, `$NUM`, `$BOOL` | Renaming |
| N2 | N1, and type names become `$TYPE` | Renaming of types |
| N3 | Everything but the kinds of the syntax tree nodes | Most token-level edits |

Docstrings of Python functions are dropped at every level.

### Clone types

The usual classification of similar code: **Type-1** differs only in layout and comments, **Type-2** also in identifiers and literals, **Type-3** also by added, removed or changed statements, and **Type-4** is equivalent in behavior with different syntax. pelicode targets types 1 to 3.

---

## #️⃣ Hashing

### Exact hash

A 128-bit hash of the whole token sequence of a function at one normalization level. Two functions with the same exact hash at N1 are identical except for names and literals.

### k-gram

\\(k\\) consecutive tokens, 10 by default (`--kgram`). Every k-gram of a function is hashed.

### Winnowing

The method that selects a subset of the k-gram hashes: in every window of \\(w\\) consecutive hashes (`--window`, 4 by default), the smallest is kept. Two functions that share a run of at least \\(w + k - 1\\) tokens are guaranteed to share a selected hash.

### Fingerprint

A k-gram hash selected by winnowing, together with the source lines it covers. The fingerprints of a function form its fingerprint set, which is what similar matches compare.

### MinHash

A fixed-length signature of a fingerprint set (`--minhash-size`, 128 by default) whose agreement with another signature estimates the Jaccard similarity of the two sets.

### LSH

Locality-sensitive hashing: the MinHash signature is cut into bands (`--bands`, 32 by default), and each band is hashed into a bucket. Functions sharing a bucket become candidates, so that only a few stored functions need a full comparison.

---

## 📊 Counting

### Package count

The number of packages containing a hash. Also called the *document frequency* (df) of the hash. Counted by `pelicode stats --refresh`.

### Common hash

A hash found in many packages: getters, `equals` methods, small idioms. It is no evidence of copying.

### Stop threshold

The package count from which a hash is ignored during a scan (`--stop-packages`): a number of packages, or a percentage of all packages.

### IDF

Inverse document frequency: with `--idf`, each fingerprint is weighted by \\(\ln(1 + N / \mathit{df})\\), where \\(N\\) is the number of packages, so that rare fingerprints count more than common ones.

### Stale statistics

Hash counts made before the last package was indexed. `scan` warns about them; `pelicode stats --refresh` brings them up to date.

---

## 🔍 Scanning

### Candidate

A stored function reported as similar to a checked function: something for a person to review, not a verdict.

### Exact match

A candidate whose exact hash equals that of the checked function at N0, N1 or N2. Reported as `exact(N1)` and so on.

### Similar match

A candidate whose fingerprint set is similar enough to that of the checked function. Reported as `similar(N1)` and so on.

### Similarity

For a similar match, the Jaccard similarity of the two fingerprint sets after common fingerprints are removed: the size of their intersection divided by the size of their union, from 0 to 1. With `--idf`, the sizes are sums of weights. Exact matches have similarity 1.

### Containment

The share of the checked function's fingerprints that are found in the candidate. A high containment with a lower similarity means the checked function is mostly inside a larger candidate.

### Shared fingerprints

The fingerprints the two functions have in common, excluding common ones. A similar match needs at least `--min-shared` of them, 5 by default, so that one or two coincidences cannot make a match.

### Matched lines

The line ranges covered by shared fingerprints, in the checked function and in the candidate (`lines 9-23 <-> 164-189`).
