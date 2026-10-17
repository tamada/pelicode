---
title: "pelicode"
description: "Know where your code came from."
date: 2026-10-17
draft: false
outputs:
  - html
---

{{< keywordList markdownify="true" >}}
[![Version](https://shields.io/badge/Version-0.1.0-blue)](https://github.com/tamada/pelicode)
[![License-MIT](https://shields.io/badge/License-MIT-blue)](https://github.com/tamada/pelicode/blob/main/LICENSE)
[![Rust](https://shields.io/badge/Rust-1.90%2B-orange?logo=rust)](https://www.rust-lang.org/)
{{< /keywordList >}}

[ [🗣️ Overview](#-overview) | [⚙️ How It Works](#-how-it-works) | [🧭 Navigation](#-navigation) ]

{{< figure default=true src="pelicode.jpg" alt="pelicode logo" width="200" class="my-4 flex justify-center" >}}

**pelicode** extracts structural features of code at the function level and compares them with the code of published packages to find **similar code that should be reviewed**, together with its source and license.

---

## 🗣️ Overview

Developers no longer write every line themselves. Code comes from AI assistants and from pull requests by people outside the team, and reading all of it to make sure none was copied from somewhere else is not possible.

[dependabot](https://docs.github.com/en/code-security/dependabot) checks the versions of your dependencies against package repositories and tells you when to upgrade. **pelicode** works in a similar spirit for the code itself: it checks the functions in your repository, or only those changed by a commit or a pull request, against the functions of packages published on PyPI, Maven Central, the Go module proxy and RubyGems.

Matching is done on normalized syntax trees, not on text, so renamed variables, reformatting, edited comments and a few added or removed statements do not hide a match.

{{< alert icon="circle-info" >}}
**pelicode does not decide whether code is plagiarized or infringes a license.** It shows candidates to review, each with the package, version, file, lines and license it was found in. Whether code was reused, and whether that matters, is for a person to judge.
{{< /alert >}}

---

## ⚙️ How It Works

{{< mermaid >}}
flowchart LR
    R[("PyPI / Maven Central<br/>Go proxy / RubyGems")] -->|pelicode crawl| I
    P["Local projects"] -->|pelicode index| I
    I["Extract, normalize<br/>and hash functions"] --> D[("SQLite database")]
    D -->|pelicode stats --refresh| D
    T["Code to check<br/>(directory or git range)"] -->|pelicode scan| S["Search for<br/>similar functions"]
    D --> S
    S --> O["Candidates to review<br/>(text / JSON / CSV)"]
{{< /mermaid >}}

1. **Collect** the code of published packages with `pelicode crawl` (or your own projects with `pelicode index`).
2. **Extract** every function with [Tree-sitter](https://tree-sitter.github.io/), and **normalize** it: comments and layout are dropped, and identifiers, literals and type names are replaced with placeholders.
3. **Hash** each function as a whole, for exact matches, and as a set of [fingerprints](/glossary#fingerprint), for partial matches. Only these hashes, the locations and the provenance are stored; source code is not.
4. **Count** how many packages contain each hash with `pelicode stats --refresh`, so that boilerplate found everywhere can be ignored.
5. **Scan** your code with `pelicode scan`, and review the candidates it reports.

Supported languages are **Go, Java, Kotlin, Python and Ruby**.

---

## 🧭 Navigation

- To install `pelicode`, see [⚓️ Install](/install),
- To try it end to end in a few minutes, see [🚀 Getting Started](/getting-started),
- To learn every command and option, see [🏃 Usage](/usage),
- To look up a term such as *fingerprint* or *stop threshold*, see [📖 Glossary](/glossary), and
- To read about the goals and the design principles, see [😀 About](/about).
