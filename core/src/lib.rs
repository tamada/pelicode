//! Feature extraction shared by the indexer and the detector of pelicode.
//!
//! Source code is parsed with Tree-sitter, split into function units,
//! normalized at several levels, and turned into exact hashes, winnowing
//! fingerprints, MinHash signatures and LSH buckets.

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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}: {1}")]
    Io(PathBuf, #[source] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("directory walk error: {0}")]
    Walk(String),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}
