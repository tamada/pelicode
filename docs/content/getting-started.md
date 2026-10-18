---
title: "🚀 Getting Started"
description: "Build a small reference database, check some code against it, and read the results."
date: 2026-10-17
draft: false
---

This tutorial takes about five minutes. You will build a small reference database from five published packages, write a function that was quietly borrowed from one of them, and let **pelicode** find it.

You need `pelicode` on your `PATH` (see [⚓️ Install](/install)), Git, and network access for the first step.

---

## 1️⃣ Build a Reference Database

Make a working directory and fetch a few packages from each supported repository:

```sh
mkdir pelicode-tutorial && cd pelicode-tutorial

pelicode crawl pypi requests six
pelicode crawl maven org.apache.commons:commons-lang3
pelicode crawl go github.com/google/uuid
pelicode crawl rubygems rack
```

Each package is downloaded, checked against the digest its repository publishes, read in memory, and indexed into `pelicode.db` in the current directory:

```
[1/2] indexed pypi:requests@2.34.2 (35 files, 709 new functions, Apache-2.0)
[2/2] indexed pypi:six@1.17.0 (4 files, 191 new functions, MIT)
2 indexed, 0 skipped, 0 failed
```

The versions you get are the latest ones at the time you run the commands, so your numbers will differ slightly. The source code itself is not kept: the database holds hashes, locations, licenses and where each file came from.

---

## 2️⃣ Count Common Hashes

Some code appears in many packages without anyone copying it: getters, `equals` methods, small idioms. Count how many packages contain each hash so that such code can be told apart:

```sh
pelicode stats --refresh
```

```
4 ecosystems, 5 packages, 5 versions, 373 files, 6550 functions (1654 with fingerprints)
hash statistics: 2026-10-17T08:27:18Z; 21 exact hashes and 354 fingerprints in 2+ packages
```

Run this again whenever you add packages; `pelicode scan` warns when the counts are out of date.

---

## 3️⃣ Write Some Borrowed Code

Create a small project with a function adapted from `super_len` in [requests](https://github.com/psf/requests) (Apache-2.0). The function and the variables are renamed, and the code is reformatted, as often happens when code is pasted in or rewritten by an AI assistant:

```sh
mkdir myapp && cd myapp
git init
git commit --allow-empty -m "Initial commit"
```

Save the following as `upload.py`:

```python
import io
import os
import warnings


def total_length(stream):
    size = None
    pos = 0
    if isinstance(stream, str):
        stream = stream.encode("utf-8")
    if hasattr(stream, "__len__"):
        size = len(stream)
    elif hasattr(stream, "len"):
        size = stream.len
    elif hasattr(stream, "fileno"):
        try:
            fd = stream.fileno()
        except (io.UnsupportedOperation, AttributeError):
            pass
        else:
            size = os.fstat(fd).st_size
            if "b" not in stream.mode:
                warnings.warn(
                    "Requests has determined the content-length for this request using the binary size of the file",
                    FileModeWarning,
                )
    if hasattr(stream, "tell"):
        try:
            pos = stream.tell()
        except OSError:
            if size is not None:
                pos = size
        else:
            if hasattr(stream, "seek") and size is None:
                try:
                    stream.seek(0, 2)
                    size = stream.tell()
                    stream.seek(pos or 0)
                except OSError:
                    size = 0
    if size is None:
        size = 0
    return max(0, size - pos)
```

---

## 4️⃣ Scan

Check the project against the database in the parent directory:

```sh
pelicode scan . --db ../pelicode.db
```

```
1 function(s) have similar code candidates to review.

upload.py:6-43 total_length (tokens=202)
  similar(N1)  0.72  pypi:requests@2.34.2 src/requests/utils.py:160-228 [Apache-2.0]
               lines 9-23,26-43 <-> 164-189,199-228 (containment 0.86, 62 shared fingerprints)
```

How to read it:

| Part | Meaning |
| --- | --- |
| `upload.py:6-43 total_length (tokens=202)` | The checked function: its file, lines, name and size |
| `similar(N1)` | A partial match after [N1 normalization](/glossary#normalization-level) (identifiers and literals abstracted). `exact(N1)` would mean the whole function is identical after normalization |
| `0.72` | The [similarity](/glossary#similarity) of the two functions, from 0 to 1 |
| `pypi:requests@2.34.2 src/requests/utils.py:160-228` | Where the similar function was found: ecosystem, package, version, file and lines |
| `[Apache-2.0]` | The license of that package |
| `lines 9-23,26-43 <-> 164-189,199-228` | Which lines of your function match which lines of the other one |
| `containment 0.86` | 86% of your function's [fingerprints](/glossary#fingerprint) are in the other function |

This is where a person takes over: open `src/requests/utils.py` of requests 2.34.2, compare the two functions, and decide what to do, for example add the attribution the Apache License requires, or rewrite the function.

---

## 5️⃣ Check Only What Changed

On a real project you usually want to check only the code a commit or a pull request touches. Commit the file and scan the last commit:

```sh
git add upload.py
git commit -m "Add upload helper"
pelicode scan --git HEAD~1..HEAD --db ../pelicode.db
```

`--git` takes a range such as `main..HEAD` or `main...feature`, and checks only the functions whose lines changed in it. With a single revision, as in `--git HEAD`, it checks the uncommitted changes in the working tree.

---

## 6️⃣ Use the Results Elsewhere

For scripts and spreadsheets, write JSON or CSV:

```sh
pelicode scan . --db ../pelicode.db -f json -o report.json
pelicode scan . --db ../pelicode.db -f csv -o report.csv
```

To fail a CI job when there is something to review, add `--fail-on-match`; the exit status is then 1 when any candidate is found:

```sh
pelicode scan --git origin/main...HEAD --db pelicode.db --fail-on-match
```

---

## 🧭 Next Steps

- **Grow the database.** `pelicode crawl pypi --top 100` fetches the 100 most downloaded PyPI packages, `pelicode crawl crates --top 100 --contact <URL>` does the same for crates.io, and `--list FILE` reads package names from a file. See [crawl](/usage#crawl).
- **Index your own projects** with `pelicode index`, so that copies between your projects are found too, and exclude a project from its own scan with `--exclude local:NAME`. See [index](/usage#index).
- **Tune the search.** `--threshold`, `--stop-packages` and `--min-shared` trade missed copies against false alarms. See [scan](/usage#scan).
- **Look inside.** `pelicode extract upload.py --tokens N1` shows how a function looks after normalization. See [extract](/usage#extract).
