//! Feature extraction shared by the indexer and the detector of
//! [pelicode](https://github.com/tamada/pelicode).
//!
//! Source code is parsed with Tree-sitter, split into function units,
//! normalized at several levels, and turned into exact hashes, winnowing
//! fingerprints, MinHash signatures and LSH buckets. Published packages and
//! the code being checked both go through this crate, so their features can
//! be compared.
//!
//! # Pipeline
//!
//! 1. **Parsing**: the source is parsed with the Tree-sitter grammar of its
//!    [`Language`], and every function, method, lambda and the like becomes
//!    a [`FunctionFeatures`]. Nested functions keep a link to their parent.
//! 2. **Normalization**: the tokens of each function are rewritten at each
//!    [`Level`], removing more superficial differences at each step:
//!
//!    | Level | What is removed |
//!    | --- | --- |
//!    | [`Level::N0`] | whitespace and comments |
//!    | [`Level::N1`] | identifiers (`$ID`, see [`IdentMode`]) and literals (`$STR`, `$NUM`, ...) |
//!    | [`Level::N2`] | type names too (`$TYPE`) |
//!    | [`Level::N3`] | everything but the kinds of the AST nodes |
//!
//! 3. **Hashing** ([`hash`]): a 128-bit exact hash of each normalized
//!    sequence finds identical functions. For the levels in
//!    [`Config::fingerprint_levels`], winnowing selects fingerprints from the
//!    k-gram hashes to find partial matches, and a MinHash signature with its
//!    LSH buckets narrows down the functions worth comparing.
//!
//! Only hashes, locations and counts come out; the source code itself is not
//! kept (unless [`Config::keep_tokens`] asks for the normalized tokens).
//!
//! # Examples
//!
//! Renaming identifiers does not change the hash at N1, but it does at N0:
//!
//! ```
//! use pelicode_core::{Config, Language, Level, extract_source};
//!
//! let config = Config::default();
//! let a = extract_source(Language::Python, b"def add(a, b):\n    return a + b\n", &config)?;
//! let b = extract_source(Language::Python, b"def plus(x, y):\n    return x + y\n", &config)?;
//!
//! assert_eq!(a[0].name, "add");
//! assert_ne!(a[0].exact_hashes[&Level::N0], b[0].exact_hashes[&Level::N0]);
//! assert_eq!(a[0].exact_hashes[&Level::N1], b[0].exact_hashes[&Level::N1]);
//! # Ok::<(), pelicode_core::Error>(())
//! ```
//!
//! For a directory, [`discover`] lists the source files and [`extract_all`]
//! extracts them in parallel.
//!
//! # Compatibility
//!
//! Features can be compared only when they were computed with the same
//! [`Config`] and the same [`NORMALIZER_VERSION`]. The database of pelicode
//! records both and refuses to mix features computed otherwise.

#![warn(missing_docs)]

pub mod config;
pub mod extract;
pub mod hash;
pub mod language;
mod normalize;
pub mod scan;

use std::path::PathBuf;

pub use config::{Config, IdentMode, Level, NORMALIZER_VERSION};
pub use extract::{
    FileFeatures, FunctionFeatures, LevelFeatures, extract_bytes, extract_file, extract_source,
};
pub use hash::{Fingerprint, Hash128};
pub use language::Language;
pub use scan::{SourceFile, discover, extract_all, slash_path};

/// Errors of feature extraction.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file could not be read; holds its path.
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    /// Tree-sitter could not set up the language or parse the source.
    #[error("parse error: {0}")]
    Parse(String),
    /// A directory could not be walked.
    #[error("directory walk error: {0}")]
    Walk(String),
    /// A [`Config`] or a value given for one is invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}
