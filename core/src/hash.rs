//! Exact hashes, winnowing fingerprints, MinHash signatures and LSH buckets.

use std::fmt;

use serde::{Serialize, Serializer};
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

/// 128-bit hash of a whole normalized representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash128(pub [u8; 16]);

impl Hash128 {
    pub fn of_tokens(tokens: &[String]) -> Hash128 {
        let mut hasher = blake3::Hasher::new();
        for token in tokens {
            hasher.update(token.as_bytes());
            // A separator that never appears in source code tokens.
            hasher.update(&[0x1f]);
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        Hash128(bytes)
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }
}

impl fmt::Display for Hash128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Hash128 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A k-gram hash selected by winnowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Fingerprint {
    #[serde(serialize_with = "serialize_u64_hex")]
    pub hash: u64,
    /// Index of the first token of the k-gram.
    pub position: u32,
    /// Source lines covered by the k-gram (1-based, inclusive).
    pub start_line: u32,
    pub end_line: u32,
}

pub(crate) fn serialize_u64_hex<S: Serializer>(
    value: &u64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("{value:016x}"))
}

pub(crate) fn serialize_u64_hex_vec<S: Serializer>(
    values: &[u64],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(values.iter().map(|v| format!("{v:016x}")))
}

/// Hashes of all k-grams of `tokens`. A sequence shorter than `k` yields one k-gram.
pub fn kgram_hashes(tokens: &[String], k: usize) -> Vec<u64> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let token_hashes: Vec<u64> = tokens.iter().map(|t| xxh3_64(t.as_bytes())).collect();
    let k = k.min(token_hashes.len());
    token_hashes
        .windows(k)
        .map(|gram| {
            let bytes: Vec<u8> = gram.iter().flat_map(|h| h.to_le_bytes()).collect();
            xxh3_64(&bytes)
        })
        .collect()
}

/// Winnowing (Schleimer et al., 2003): the rightmost minimum of every window of `w` hashes,
/// as `(position, hash)` pairs.
pub fn winnow(hashes: &[u64], w: usize) -> Vec<(usize, u64)> {
    let mut selected: Vec<(usize, u64)> = Vec::new();
    if hashes.is_empty() {
        return selected;
    }
    let w = w.min(hashes.len());
    for start in 0..=hashes.len() - w {
        let mut min_pos = start;
        for pos in start..start + w {
            if hashes[pos] <= hashes[min_pos] {
                min_pos = pos;
            }
        }
        if selected.last().is_none_or(|(pos, _)| *pos != min_pos) {
            selected.push((min_pos, hashes[min_pos]));
        }
    }
    selected
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// MinHash signature of a set of hashes; `None` for an empty set.
pub fn minhash(hashes: impl Iterator<Item = u64>, size: usize) -> Option<Vec<u64>> {
    let mut signature = vec![u64::MAX; size];
    let mut any = false;
    for h in hashes {
        any = true;
        for (i, slot) in signature.iter_mut().enumerate() {
            let seed = splitmix64(i as u64);
            let v = splitmix64(h ^ seed);
            if v < *slot {
                *slot = v;
            }
        }
    }
    any.then_some(signature)
}

/// LSH bucket hashes: one per band of `signature`.
pub fn lsh_buckets(signature: &[u64], bands: usize) -> Vec<u64> {
    let rows = signature.len() / bands;
    signature
        .chunks(rows)
        .enumerate()
        .map(|(band, chunk)| {
            let bytes: Vec<u8> = chunk.iter().flat_map(|h| h.to_le_bytes()).collect();
            xxh3_64_with_seed(&bytes, band as u64)
        })
        .collect()
}

/// Estimated Jaccard similarity of two MinHash signatures.
pub fn minhash_similarity(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let same = a.iter().zip(b).filter(|(x, y)| x == y).count();
    same as f64 / a.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn winnowing_covers_every_window() {
        let hashes = kgram_hashes(&toks("a b c d e f g h i j k l m n"), 3);
        let fps = winnow(&hashes, 4);
        assert!(!fps.is_empty());
        for start in 0..=hashes.len() - 4 {
            assert!(fps.iter().any(|(pos, _)| (start..start + 4).contains(pos)));
        }
    }

    #[test]
    fn short_sequences_still_have_a_fingerprint() {
        let hashes = kgram_hashes(&toks("a b"), 5);
        assert_eq!(hashes.len(), 1);
        assert_eq!(winnow(&hashes, 4).len(), 1);
    }

    #[test]
    fn minhash_of_identical_sets_is_identical() {
        let a = minhash([1u64, 2, 3].into_iter(), 16).unwrap();
        let b = minhash([3u64, 2, 1].into_iter(), 16).unwrap();
        assert_eq!(minhash_similarity(&a, &b), 1.0);
        assert_eq!(lsh_buckets(&a, 4), lsh_buckets(&b, 4));
        assert!(minhash(std::iter::empty(), 16).is_none());
    }
}
