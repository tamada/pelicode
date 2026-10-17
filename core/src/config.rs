//! Parameters of feature extraction.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::Error;

/// Version of the normalization rules. Features computed with different
/// versions must never be compared with each other.
pub const NORMALIZER_VERSION: u32 = 1;

/// Normalization level (see spec §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    /// Tokens without whitespace and comments.
    N0,
    /// N0 with identifiers and literals abstracted.
    N1,
    /// N1 with type names abstracted.
    N2,
    /// Sequence of named AST node kinds.
    N3,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::N0, Level::N1, Level::N2, Level::N3];

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(value: u8) -> Option<Level> {
        Level::ALL.get(value as usize).copied()
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "N{}", self.as_u8())
    }
}

impl FromStr for Level {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "N0" | "0" => Ok(Level::N0),
            "N1" | "1" => Ok(Level::N1),
            "N2" | "2" => Ok(Level::N2),
            "N3" | "3" => Ok(Level::N3),
            _ => Err(Error::InvalidConfig(format!("unknown level: {s}"))),
        }
    }
}

/// How identifiers are abstracted from N1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentMode {
    /// Every identifier becomes `$ID`.
    #[default]
    Uniform,
    /// Identifiers are numbered by first appearance in the function (`$ID1`, `$ID2`, ...).
    Indexed,
}

impl fmt::Display for IdentMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IdentMode::Uniform => "uniform",
            IdentMode::Indexed => "indexed",
        })
    }
}

impl FromStr for IdentMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "uniform" => Ok(IdentMode::Uniform),
            "indexed" => Ok(IdentMode::Indexed),
            _ => Err(Error::InvalidConfig(format!("unknown ident mode: {s}"))),
        }
    }
}

/// Parameters of feature extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Functions with fewer N0 tokens get exact hashes only (no fingerprints / MinHash).
    pub min_tokens: usize,
    pub ident_mode: IdentMode,
    /// k of the k-grams used for fingerprints.
    pub kgram: usize,
    /// Window size of winnowing.
    pub window: usize,
    /// Length of MinHash signatures.
    pub minhash_size: usize,
    /// Number of LSH bands; `minhash_size` must be divisible by it.
    pub bands: usize,
    /// Levels for which fingerprints, MinHash and LSH buckets are computed.
    pub fingerprint_levels: Vec<Level>,
    /// Keep the normalized token sequences in the output (for debugging and research).
    pub keep_tokens: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            min_tokens: 50,
            ident_mode: IdentMode::Uniform,
            kgram: 5,
            window: 4,
            minhash_size: 128,
            bands: 32,
            fingerprint_levels: vec![Level::N1],
            keep_tokens: false,
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<(), Error> {
        if self.kgram == 0 || self.window == 0 {
            return Err(Error::InvalidConfig(
                "kgram and window must be positive".into(),
            ));
        }
        if self.bands == 0
            || self.minhash_size == 0
            || !self.minhash_size.is_multiple_of(self.bands)
        {
            return Err(Error::InvalidConfig(format!(
                "minhash_size ({}) must be a positive multiple of bands ({})",
                self.minhash_size, self.bands
            )));
        }
        Ok(())
    }

    /// Rows per LSH band.
    pub fn rows(&self) -> usize {
        self.minhash_size / self.bands
    }
}
