---
title: "😀 About"
description: "Why pelicode exists, the principles behind it, and its logo."
date: 2026-10-17
draft: false
---

Learn why **pelicode** was built, what it deliberately does not do, and where it is heading.

---

## 🎯 Why pelicode

Software is no longer written only by its developers. AI assistants generate code, and pull requests arrive from contributors the maintainers have never met. Any of that code may have been taken from somewhere else, often without the person submitting it knowing, and no reviewer can read every line and recognize where it came from.

Package managers already keep an eye on dependencies for us: dependabot compares the versions we use with those on the package repositories and warns about the ones that need attention. **pelicode** applies the same idea to the code itself. It keeps a database of the functions of published packages, and checks the functions of a project, or only those a change touches, against it.

pelicode is the prototype of a research project on infrastructure that detects software theft without developers having to look for it. The research questions are about the detection method, not about the tool:

- Can structural features extracted per function find code that was renamed, reformatted or partly edited better than text or token matching does?
- Under which conditions are functions a good unit of comparison, and where do they fall short (very short functions, functions split or merged, code moved around)?
- How do accuracy, speed and database size change as the database grows to millions of functions?

---

## 🧭 Principles

- **Candidates, not verdicts.** Similar code is not proof of copying, and copying is not always a problem. pelicode reports code to review, with its source and license; people decide.
- **No source code in the database.** The database holds hashes, locations, licenses and download URLs. Leaking it does not leak anyone's code, and the original can always be fetched again from its source.
- **Provenance first.** Every candidate says which package, version, file and lines it came from, and under which license.
- **Be gentle with public services.** The crawler sends one request at a time, waits between requests, verifies what it downloads, and never fetches a version twice.

---

## 🗺️ Status and Roadmap

pelicode is a research prototype (v0.1). pelicode checks its own code, too. What it can do today:

- Build a database from PyPI, Maven Central, the Go module proxy, RubyGems and crates.io, keep it current with the new versions of its packages (`crawl --updates`), and keep it small: files are stored once, the counts of common hashes are refreshed incrementally, and `pelicode clean` removes what no version refers to any more.
- Check Go, Java, Kotlin, Python, Ruby and Rust code against it, whole directories or only the lines a git range changed. It reports in text, JSON, CSV or SARIF for GitHub code scanning, and leaves out the dependencies of the checked code.
- Keep the options of a repository in a settings file (`.pelicode.yaml` or `.pelicode.json5`).
- Write snapshots of the database for checking only (`pelicode snapshot`), publish them as OCI artifacts in a registry, and check against them directly (`--db oci://...`), several at once if needed.
- Check pull requests with the GitHub Action [pelicode-action](https://github.com/tamada/pelicode-action).

Planned next:

- Snapshots published periodically, with their parameters readable before downloading.
- An evaluation framework to measure recall, precision, speed and database size on mutated code.

---

## 🎨 The Logo

{{< figure default=true src="pelicode.jpg" alt="pelicode logo" width="300" class="my-4" >}}

The mascot of **pelicode** is a pelican.

---

## 📃 How to Refer to the Tool

Use the following BibTeX entry.

```tex
@misc { pelicode,
    author  = {Haruaki Tamada},
    title   = {pelicode: Know where your code came from},
    note    = {\url{https://github.com/tamada/pelicode}},
    version = {0.1.0},
    year    = {2026},
}
```

The repository also has a [`CITATION.cff`](https://github.com/tamada/pelicode/blob/main/CITATION.cff), from which GitHub offers "Cite this repository".

---

## 📜 License

pelicode is released under the [MIT License](https://github.com/tamada/pelicode/blob/main/LICENSE).
