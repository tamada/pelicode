//! Detection of functions similar to the stored ones (spec §7).
//!
//! Candidates are searched in stages: exact hashes from the weakest to the
//! strongest normalization, then LSH buckets, then the fingerprint sets of
//! the LSH candidates are compared.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use pelicode_core::{FileFeatures, FunctionFeatures, Language, Level};
use pelicode_db::{Database, FunctionInfo, Occurrence};
use serde::Serialize;

pub use pelicode_db::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Levels searched for exact matches, in order of preference.
const EXACT_LEVELS: [Level; 3] = [Level::N0, Level::N1, Level::N2];

#[derive(Debug, Clone)]
pub struct Options {
    /// Functions with fewer tokens are not checked.
    pub min_tokens: usize,
    /// Minimum Jaccard similarity of fingerprint sets to report a candidate.
    pub threshold: f64,
    /// Candidates reported per function.
    pub max_candidates: usize,
    /// Occurrences listed per candidate.
    pub max_occurrences: usize,
    /// Packages whose functions are never reported, as `(ecosystem, name)`.
    pub exclude_packages: Vec<(String, String)>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            min_tokens: 50,
            threshold: 0.7,
            max_candidates: 5,
            max_occurrences: 5,
            exclude_packages: Vec::new(),
        }
    }
}

/// How a candidate matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "type", content = "level", rename_all = "lowercase")]
pub enum MatchKind {
    /// Identical after normalization at the level.
    Exact(Level),
    /// Similar fingerprint sets at the level.
    Similar(Level),
}

/// A stored function similar to a checked function.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    #[serde(rename = "match")]
    pub kind: MatchKind,
    /// 1.0 for exact matches; the Jaccard similarity of fingerprint sets otherwise.
    pub similarity: f64,
    /// Share of the checked function's fingerprints found in the candidate.
    pub containment: f64,
    pub shared_fingerprints: usize,
    /// Lines of the checked function covered by shared fingerprints.
    pub matched_lines: Vec<(u32, u32)>,
    /// Lines of the candidate covered by shared fingerprints.
    pub candidate_lines: Vec<(u32, u32)>,
    pub function: FunctionInfo,
    /// Where the candidate appears, earliest publication first (at most `max_occurrences`).
    pub occurrences: Vec<Occurrence>,
    pub occurrence_count: usize,
}

/// Detection result of one checked function.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub path: PathBuf,
    pub language: Language,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub token_count: usize,
    pub candidates: Vec<Candidate>,
}

pub struct Detector<'a> {
    db: &'a Database,
    options: Options,
}

impl<'a> Detector<'a> {
    /// `db` must have been opened for features computed like the checked ones
    /// (see [`Database::config_from_meta`]).
    pub fn new(db: &'a Database, options: Options) -> Detector<'a> {
        Detector { db, options }
    }

    /// Checks the functions of `files` accepted by `select`; only functions
    /// with at least one candidate are reported.
    pub fn detect(
        &self,
        files: &[FileFeatures],
        select: impl Fn(&FileFeatures, &FunctionFeatures) -> bool,
    ) -> Result<Vec<Report>> {
        let mut reports = Vec::new();
        for file in files {
            for function in &file.functions {
                if function.token_count < self.options.min_tokens || !select(file, function) {
                    continue;
                }
                let candidates = self.candidates(function)?;
                if candidates.is_empty() {
                    continue;
                }
                reports.push(Report {
                    path: file.path.clone(),
                    language: file.language,
                    name: function.name.clone(),
                    start_line: function.start_line,
                    end_line: function.end_line,
                    token_count: function.token_count,
                    candidates,
                });
            }
        }
        Ok(reports)
    }

    /// Candidates for one function, best first.
    pub fn candidates(&self, function: &FunctionFeatures) -> Result<Vec<Candidate>> {
        let mut found: HashMap<i64, Match> = HashMap::new();

        for level in EXACT_LEVELS {
            let Some(hash) = function.exact_hashes.get(&level) else {
                continue;
            };
            for id in self.db.functions_by_exact_hash(level, hash)? {
                found.entry(id).or_insert(Match {
                    kind: MatchKind::Exact(level),
                    similarity: 1.0,
                    containment: 1.0,
                    shared: 0,
                    matched_lines: vec![(function.start_line as u32, function.end_line as u32)],
                    candidate_lines: Vec::new(),
                });
            }
        }

        for (level, features) in &function.features {
            let mut lsh_hits: HashSet<i64> = HashSet::new();
            for (band, bucket) in features.lsh_buckets.iter().enumerate() {
                lsh_hits.extend(self.db.functions_by_lsh_bucket(*level, band, *bucket)?);
            }
            let query: HashSet<u64> = features.fingerprints.iter().map(|f| f.hash).collect();
            for id in lsh_hits {
                if found
                    .get(&id)
                    .is_some_and(|m| matches!(m.kind, MatchKind::Exact(_)))
                {
                    continue;
                }
                let stored = self.db.fingerprints(id, *level)?;
                let other: HashSet<u64> = stored.iter().map(|f| f.hash).collect();
                let shared: HashSet<u64> = query.intersection(&other).copied().collect();
                let union = query.len() + other.len() - shared.len();
                let similarity = shared.len() as f64 / union.max(1) as f64;
                if similarity < self.options.threshold {
                    continue;
                }
                let candidate = Match {
                    kind: MatchKind::Similar(*level),
                    similarity,
                    containment: shared.len() as f64 / query.len().max(1) as f64,
                    shared: shared.len(),
                    matched_lines: merge(
                        features
                            .fingerprints
                            .iter()
                            .filter(|f| shared.contains(&f.hash))
                            .map(|f| (f.start_line, f.end_line)),
                    ),
                    candidate_lines: merge(
                        stored
                            .iter()
                            .filter(|f| shared.contains(&f.hash))
                            .map(|f| (f.start_line, f.end_line)),
                    ),
                };
                // Keep the most similar level when several levels hit.
                if found.get(&id).is_none_or(|m| m.similarity < similarity) {
                    found.insert(id, candidate);
                }
            }
        }

        let mut candidates = Vec::new();
        for (id, m) in found {
            let mut occurrences = self.db.occurrences(id)?;
            occurrences.retain(|o| !self.is_excluded(o));
            if occurrences.is_empty() {
                continue;
            }
            let function_info = self.db.function_info(id)?;
            let candidate_lines = if m.candidate_lines.is_empty() {
                vec![(
                    function_info.start_line as u32,
                    function_info.end_line as u32,
                )]
            } else {
                m.candidate_lines
            };
            let occurrence_count = occurrences.len();
            occurrences.truncate(self.options.max_occurrences);
            candidates.push(Candidate {
                kind: m.kind,
                similarity: m.similarity,
                containment: m.containment,
                shared_fingerprints: m.shared,
                matched_lines: m.matched_lines,
                candidate_lines,
                function: function_info,
                occurrences,
                occurrence_count,
            });
        }
        candidates.sort_by(|a, b| {
            b.similarity
                .total_cmp(&a.similarity)
                .then(a.kind.cmp(&b.kind))
                .then(a.function.id.cmp(&b.function.id))
        });
        candidates.truncate(self.options.max_candidates);
        Ok(candidates)
    }

    fn is_excluded(&self, o: &Occurrence) -> bool {
        self.options
            .exclude_packages
            .iter()
            .any(|(eco, name)| *eco == o.ecosystem && *name == o.package)
    }
}

struct Match {
    kind: MatchKind,
    similarity: f64,
    containment: f64,
    shared: usize,
    matched_lines: Vec<(u32, u32)>,
    candidate_lines: Vec<(u32, u32)>,
}

/// Merges overlapping or adjacent line ranges.
fn merge(ranges: impl Iterator<Item = (u32, u32)>) -> Vec<(u32, u32)> {
    let sorted: BTreeMap<u32, u32> = ranges.fold(BTreeMap::new(), |mut acc, (s, e)| {
        let end = acc.entry(s).or_insert(e);
        *end = (*end).max(e);
        acc
    });
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (start, end) in sorted {
        match merged.last_mut() {
            Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_line_ranges() {
        let ranges = [(5, 6), (1, 2), (3, 4), (10, 12), (11, 11), (8, 8)];
        assert_eq!(merge(ranges.into_iter()), [(1, 6), (8, 8), (10, 12)]);
    }
}
