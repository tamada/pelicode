---
title: "⚓️ Install"
description: "How to install pelicode with Homebrew, with Cargo, or from source."
date: 2026-10-17
draft: false
---

Install **pelicode** with [Homebrew](#-with-homebrew), with Rust's package manager [Cargo](#-with-cargo), or [build it from source](#-from-source). Every method installs a single command, `pelicode`. To run it without installing anything but Docker, use the [container image](#-with-docker).

---

## 🛠️ Prerequisites

- **For Homebrew**: [Homebrew](https://brew.sh/) on macOS or Linux. Nothing else is needed.
- **For Cargo and building from source**:
  - The [Rust toolchain](https://www.rust-lang.org/tools/install), version **1.90 or later**.
  - A C compiler (`cc`, Xcode Command Line Tools on macOS, `build-essential` on Debian and Ubuntu). The Tree-sitter grammars and the bundled SQLite are written in C and compiled during the build.
- **For `pelicode scan --git`**: [Git](https://git-scm.com/) on your `PATH`.

`pelicode crawl` downloads packages over HTTPS, so it needs network access. The other commands work offline.

---

## 🍺 With Homebrew

```sh
brew install tamada/tap/pelicode
```

This taps [tamada/homebrew-tap](https://github.com/tamada/homebrew-tap) and installs the `pelicode` command.

To upgrade or remove it later:

```sh
brew upgrade pelicode
brew uninstall pelicode
```

---

## 📦 With Cargo

```sh
cargo install pelicode
```

This builds the [`pelicode`](https://crates.io/crates/pelicode) crate from crates.io and installs the `pelicode` command into Cargo's binary directory (usually `~/.cargo/bin`, which should be in your `PATH`).

To install the latest commit of the repository instead:

```sh
cargo install --git https://github.com/tamada/pelicode pelicode
```

To remove it later:

```sh
cargo uninstall pelicode
```

---

## 🐳 With Docker

```sh
docker run --rm -v "$PWD":/workdir quay.io/tama5/pelicode languages
```

The image [quay.io/tama5/pelicode](https://quay.io/repository/tama5/pelicode) is built for `linux/amd64` and `linux/arm64`, and carries Git for `scan --git`. Its working directory is `/workdir`, so mount the directory that holds your code and the database there:

```sh
docker run --rm -v "$PWD":/workdir quay.io/tama5/pelicode crawl pypi requests
docker run --rm -v "$PWD":/workdir quay.io/tama5/pelicode scan --git main..HEAD
```

The command runs as an unprivileged user (UID 1000). On Linux, if that user cannot write the database into the mounted directory, run it as yourself with `--user "$(id -u):$(id -g)"`.

---

## 🏗️ From Source

```sh
git clone https://github.com/tamada/pelicode.git
cd pelicode
cargo build --release
```

The command is built as `target/release/pelicode`. Copy it to a directory in your `PATH`, or let Cargo install it:

```sh
cargo install --path cli
```

To run the tests (they do not access the network):

```sh
cargo test
```

The repository also has a [Justfile](https://github.com/tamada/pelicode/blob/main/Justfile) with the tasks CI runs, for [just](https://just.systems/): `just test` (formatting, clippy, build and tests with coverage), `just publish_check`, `just docker` and more; `just --list` shows them all.

The repository is a Cargo workspace of five crates:

| Directory | Crate | Contents |
| --- | --- | --- |
| `core` | pelicode-core | Parsing, function extraction, normalization, hashes, fingerprints, MinHash |
| `db` | pelicode-db | SQLite storage, lookups and hash counts |
| `detect` | pelicode-detect | Search for similar code candidates |
| `crawl` | pelicode-crawl | Fetching from PyPI, Maven Central, the Go module proxy, RubyGems and crates.io |
| `cli` | pelicode | The `pelicode` command |

---

## ✅ Check the Installation

```sh
pelicode --version
pelicode languages
```

The second command lists the supported languages and their file extensions:

```
go       go
java     java
kotlin   kt, kts
python   py
ruby     rb
rust     rs
```

Next, follow [🚀 Getting Started](/getting-started) to build a small database and check some code against it.
