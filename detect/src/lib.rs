//! Detection of functions similar to the stored ones (specification §7).
//!
//! Candidates are searched in stages: exact hashes from the weakest to the
//! strongest normalization, then LSH buckets, then the fingerprint sets of
//! the LSH candidates are compared.

#![warn(missing_docs)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use pelicode_core::{FileFeatures, FunctionFeatures, Language, Level};
use pelicode_db::{Database, FunctionInfo, Occurrence};
use serde::Serialize;

pub mod dependencies;

pub use pelicode_db::Error;

/// Result of the detection, whose errors are those of the database.
pub type Result<T> = std::result::Result<T, Error>;

/// Levels searched for exact matches, in order of preference.
const EXACT_LEVELS: [Level; 3] = [Level::N0, Level::N1, Level::N2];

/// Parameters of the search.
///
/// # Examples
///
/// ```
/// use pelicode_detect::Options;
///
/// let options = Options { threshold: 0.5, stop_packages: 0, ..Options::default() };
/// assert_eq!(options.max_candidates, 5);
/// ```
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
    /// Hashes found in at least this many packages are ignored (0 disables).
    /// Needs the statistics of [`Database::refresh_hash_stats`].
    pub stop_packages: usize,
    /// Minimum number of shared (non-common) fingerprints for a similar match.
    /// Without it, a function whose fingerprints are almost all common could
    /// reach a high similarity through one or two remaining ones.
    pub min_shared: usize,
    /// Weight fingerprints by inverse document frequency, `ln(1 + N / df)`,
    /// where N is the number of packages and df those containing the fingerprint.
    pub idf: bool,
    /// Packages the checked code depends on (see [`dependencies`]): candidates
    /// found only in them are not reported, and their occurrences are left out
    /// of the other candidates.
    pub dependencies: dependencies::Dependencies,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            min_tokens: 50,
            threshold: 0.7,
            max_candidates: 5,
            max_occurrences: 5,
            exclude_packages: Vec::new(),
            stop_packages: 100,
            min_shared: 5,
            idf: false,
            dependencies: dependencies::Dependencies::default(),
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
    /// How the candidate matched.
    #[serde(rename = "match")]
    pub kind: MatchKind,
    /// 1.0 for exact matches; the (weighted) Jaccard similarity of fingerprint sets otherwise.
    pub similarity: f64,
    /// (Weighted) share of the checked function's fingerprints found in the candidate.
    pub containment: f64,
    /// For exact matches, the number of packages containing the same normalized function.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_count: Option<usize>,
    /// Number of fingerprints the two functions share, common ones excluded.
    pub shared_fingerprints: usize,
    /// Lines of the checked function covered by shared fingerprints.
    pub matched_lines: Vec<(u32, u32)>,
    /// Lines of the candidate covered by shared fingerprints.
    pub candidate_lines: Vec<(u32, u32)>,
    /// The stored function.
    pub function: FunctionInfo,
    /// Where the candidate appears, earliest publication first (at most `max_occurrences`).
    pub occurrences: Vec<Occurrence>,
    /// Number of places the stored function was found, excluded packages aside.
    pub occurrence_count: usize,
    /// Position of the database of the candidate in the detectors given to
    /// [`detect_all`]; `function.id` is an id in that database.
    #[serde(skip)]
    pub database: usize,
}

/// Detection result of one checked function.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// File of the checked function.
    pub path: PathBuf,
    /// Language of the checked function.
    pub language: Language,
    /// Name of the checked function.
    pub name: String,
    /// First line of the checked function (1-based).
    pub start_line: usize,
    /// Last line of the checked function (1-based, inclusive).
    pub end_line: usize,
    /// Number of tokens of the checked function.
    pub token_count: usize,
    /// Candidates, best first.
    pub candidates: Vec<Candidate>,
}

/// Searches a database for functions similar to checked ones.
///
/// # Examples
///
/// ```
/// # use pelicode_core::{Config, Language, extract_bytes};
/// # use pelicode_db::{Database, PackageInfo};
/// # use pelicode_detect::{Detector, MatchKind, Options};
/// # let dir = tempfile::tempdir().unwrap();
/// # let config = Config::default();
/// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
/// let original = br#"class Search {
///     static int find(int[] values, int key) {
///         int low = 0, high = values.length - 1;
///         while (low <= high) {
///             int mid = (low + high) >>> 1;
///             if (values[mid] < key) low = mid + 1;
///             else if (values[mid] > key) high = mid - 1;
///             else return mid;
///         }
///         return -(low + 1);
///     }
/// }"#;
/// let package = PackageInfo {
///     ecosystem: "maven".into(),
///     name: "org.example:search".into(),
///     version: "1.0".into(),
///     ..PackageInfo::default()
/// };
/// let stored = extract_bytes("Search.java".as_ref(), Language::Java, original, &config).unwrap();
/// db.index(&package, &[stored]).unwrap();
///
/// // The same code with other names.
/// let copy = String::from_utf8_lossy(original)
///     .replace("Search", "Finder").replace("values", "xs").replace("key", "target");
/// let checked = extract_bytes("Finder.java".as_ref(), Language::Java, copy.as_bytes(), &config).unwrap();
/// let detector = Detector::new(&db, Options { min_tokens: 20, ..Options::default() }).unwrap();
/// let reports = detector.detect(&[checked], |_, _| true).unwrap();
/// assert_eq!(reports[0].name, "find");
/// assert_eq!(reports[0].candidates[0].kind, MatchKind::Exact(pelicode_core::Level::N1));
/// assert_eq!(reports[0].candidates[0].occurrences[0].package, "org.example:search");
/// ```
pub struct Detector<'a> {
    db: &'a Database,
    options: Options,
    /// Packages in the database, the N of IDF.
    packages: usize,
    document_frequencies: RefCell<HashMap<(Level, u64), usize>>,
    /// Candidates left out because they were found only in dependencies.
    excluded_as_dependencies: Cell<usize>,
}

impl<'a> Detector<'a> {
    /// `db` must have been opened for features computed like the checked ones
    /// (see [`Database::config_from_meta`]).
    ///
    /// # Errors
    ///
    /// [`Error`] when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::Config;
    /// use pelicode_db::Database;
    /// use pelicode_detect::{Detector, Options};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let db = Database::open(dir.path().join("pelicode.db"), &Config::default()).unwrap();
    /// let detector = Detector::new(&db, Options::default()).unwrap();
    /// ```
    pub fn new(db: &'a Database, options: Options) -> Result<Detector<'a>> {
        Ok(Detector {
            db,
            options,
            packages: db.package_count()?,
            document_frequencies: RefCell::new(HashMap::new()),
            excluded_as_dependencies: Cell::new(0),
        })
    }

    /// Number of candidates left out so far because they were found only in
    /// [`Options::dependencies`].
    ///
    /// # Examples
    ///
    /// ```
    /// use pelicode_core::{Config, Language, extract_bytes};
    /// use pelicode_db::{Database, PackageInfo};
    /// use pelicode_detect::{Detector, Options, dependencies::Dependencies};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let config = Config { min_tokens: 0, ..Config::default() };
    /// let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
    /// let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
    /// let package = PackageInfo {
    ///     ecosystem: "pypi".into(), name: "calc".into(), version: "1.0".into(), ..PackageInfo::default()
    /// };
    /// db.index(&package, &[file.clone()]).unwrap();
    ///
    /// // The checked project depends on calc.
    /// let mut dependencies = Dependencies::default();
    /// dependencies.add_file("requirements.txt", "calc>=1\n");
    /// let options = Options { min_tokens: 0, dependencies, ..Options::default() };
    /// let detector = Detector::new(&db, options).unwrap();
    /// assert!(detector.detect(&[file], |_, _| true).unwrap().is_empty());
    /// assert_eq!(detector.excluded_as_dependencies(), 1);
    /// ```
    pub fn excluded_as_dependencies(&self) -> usize {
        self.excluded_as_dependencies.get()
    }

    fn is_common(&self, package_count: usize) -> bool {
        self.options.stop_packages > 0 && package_count >= self.options.stop_packages
    }

    /// Weight of a fingerprint; 0 for common ones.
    fn weight(&self, level: Level, hash: u64) -> Result<f64> {
        let cached = self
            .document_frequencies
            .borrow()
            .get(&(level, hash))
            .copied();
        let df = match cached {
            Some(df) => df,
            None => {
                let df = self.db.fingerprint_package_count(level, hash)?;
                self.document_frequencies
                    .borrow_mut()
                    .insert((level, hash), df);
                df
            }
        };
        Ok(if self.is_common(df) {
            0.0
        } else if self.options.idf {
            (1.0 + self.packages.max(1) as f64 / df.max(1) as f64).ln()
        } else {
            1.0
        })
    }

    fn weights(
        &self,
        level: Level,
        hashes: impl Iterator<Item = u64>,
    ) -> Result<HashMap<u64, f64>> {
        let mut weights = HashMap::new();
        for hash in hashes {
            if let std::collections::hash_map::Entry::Vacant(e) = weights.entry(hash) {
                e.insert(self.weight(level, hash)?);
            }
        }
        Ok(weights)
    }

    /// Checks the functions of `files` accepted by `select`; only functions
    /// with at least one candidate are reported.
    ///
    /// # Errors
    ///
    /// [`Error`] when the database cannot be read.
    ///
    /// # Examples
    ///
    /// Only the functions accepted by `select` are checked, here those starting
    /// after line 1:
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # use pelicode_detect::{Detector, MatchKind, Options};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config::default();
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// let original = br#"class Search {
    ///     static int find(int[] values, int key) {
    ///         int low = 0, high = values.length - 1;
    ///         while (low <= high) {
    ///             int mid = (low + high) >>> 1;
    ///             if (values[mid] < key) low = mid + 1;
    ///             else if (values[mid] > key) high = mid - 1;
    ///             else return mid;
    ///         }
    ///         return -(low + 1);
    ///     }
    /// }"#;
    /// let package = PackageInfo {
    ///     ecosystem: "maven".into(),
    ///     name: "org.example:search".into(),
    ///     version: "1.0".into(),
    ///     ..PackageInfo::default()
    /// };
    /// let stored = extract_bytes("Search.java".as_ref(), Language::Java, original, &config).unwrap();
    /// db.index(&package, &[stored]).unwrap();
    ///
    /// // The same code with other names.
    /// let copy = String::from_utf8_lossy(original)
    ///     .replace("Search", "Finder").replace("values", "xs").replace("key", "target");
    /// let checked = extract_bytes("Finder.java".as_ref(), Language::Java, copy.as_bytes(), &config).unwrap();
    /// let detector = Detector::new(&db, Options { min_tokens: 20, ..Options::default() }).unwrap();
    /// let reports = detector.detect(&[checked], |_, f| f.start_line > 1).unwrap();
    /// assert_eq!(reports.len(), 1);
    /// ```
    pub fn detect(
        &self,
        files: &[FileFeatures],
        select: impl Fn(&FileFeatures, &FunctionFeatures) -> bool,
    ) -> Result<Vec<Report>> {
        detect_all(std::slice::from_ref(self), files, select)
    }

    /// Candidates for one function, best first.
    ///
    /// Unlike [`Detector::detect`], the function is checked whatever its size.
    ///
    /// # Errors
    ///
    /// [`Error`] when the database cannot be read.
    ///
    /// # Examples
    ///
    /// ```
    /// # use pelicode_core::{Config, Language, extract_bytes};
    /// # use pelicode_db::{Database, PackageInfo};
    /// # use pelicode_detect::{Detector, MatchKind, Options};
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let config = Config::default();
    /// # let mut db = Database::open(dir.path().join("pelicode.db"), &config).unwrap();
    /// let original = br#"class Search {
    ///     static int find(int[] values, int key) {
    ///         int low = 0, high = values.length - 1;
    ///         while (low <= high) {
    ///             int mid = (low + high) >>> 1;
    ///             if (values[mid] < key) low = mid + 1;
    ///             else if (values[mid] > key) high = mid - 1;
    ///             else return mid;
    ///         }
    ///         return -(low + 1);
    ///     }
    /// }"#;
    /// let package = PackageInfo {
    ///     ecosystem: "maven".into(),
    ///     name: "org.example:search".into(),
    ///     version: "1.0".into(),
    ///     ..PackageInfo::default()
    /// };
    /// let stored = extract_bytes("Search.java".as_ref(), Language::Java, original, &config).unwrap();
    /// db.index(&package, &[stored]).unwrap();
    ///
    /// // The same code with other names.
    /// let copy = String::from_utf8_lossy(original)
    ///     .replace("Search", "Finder").replace("values", "xs").replace("key", "target");
    /// let checked = extract_bytes("Finder.java".as_ref(), Language::Java, copy.as_bytes(), &config).unwrap();
    /// let detector = Detector::new(&db, Options::default()).unwrap();
    /// let candidates = detector.candidates(&checked.functions[0]).unwrap();
    /// assert_eq!(candidates[0].similarity, 1.0);
    /// assert_eq!(candidates[0].function.name, "find");
    /// ```
    pub fn candidates(&self, function: &FunctionFeatures) -> Result<Vec<Candidate>> {
        let mut found: HashMap<i64, Match> = HashMap::new();

        for level in EXACT_LEVELS {
            let Some(hash) = function.exact_hashes.get(&level) else {
                continue;
            };
            let package_count = self.db.exact_package_count(level, hash)?;
            if self.is_common(package_count) {
                continue;
            }
            for id in self.db.functions_by_exact_hash(level, hash)? {
                found.entry(id).or_insert(Match {
                    kind: MatchKind::Exact(level),
                    similarity: 1.0,
                    containment: 1.0,
                    package_count: Some(package_count),
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
            let query = self.weights(*level, features.fingerprints.iter().map(|f| f.hash))?;
            let query_weight: f64 = query.values().sum();
            if query_weight == 0.0 {
                // Nothing but common code.
                continue;
            }
            for id in lsh_hits {
                if found
                    .get(&id)
                    .is_some_and(|m| matches!(m.kind, MatchKind::Exact(_)))
                {
                    continue;
                }
                let stored = self.db.fingerprints(id, *level)?;
                let other = self.weights(*level, stored.iter().map(|f| f.hash))?;
                let shared: HashSet<u64> = query
                    .iter()
                    .filter(|(h, w)| **w > 0.0 && other.contains_key(h))
                    .map(|(h, _)| *h)
                    .collect();
                let shared_weight: f64 = shared.iter().map(|h| query[h]).sum();
                let union = query_weight + other.values().sum::<f64>() - shared_weight;
                let similarity = if union > 0.0 {
                    shared_weight / union
                } else {
                    0.0
                };
                if similarity < self.options.threshold
                    || shared.is_empty()
                    || shared.len() < self.options.min_shared
                {
                    continue;
                }
                let candidate = Match {
                    kind: MatchKind::Similar(*level),
                    similarity,
                    containment: shared_weight / query_weight,
                    package_count: None,
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
            occurrences.retain(|o| !self.options.dependencies.contains(&o.ecosystem, &o.package));
            if occurrences.is_empty() {
                self.excluded_as_dependencies
                    .set(self.excluded_as_dependencies.get() + 1);
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
                package_count: m.package_count,
                shared_fingerprints: m.shared,
                matched_lines: m.matched_lines,
                candidate_lines,
                function: function_info,
                occurrences,
                occurrence_count,
                database: 0,
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
    package_count: Option<usize>,
    shared: usize,
    matched_lines: Vec<(u32, u32)>,
    candidate_lines: Vec<(u32, u32)>,
}

/// Merges overlapping or adjacent line ranges.
/// Checks the functions of `files` that `select` accepts against several
/// databases at once, one detector each, and merges their candidates, best
/// first. Each detector filters common code with the package counts of its
/// own database. The size limits of the first detector's options apply.
///
/// The databases must hold features computed with the same parameters, the
/// fingerprint levels aside (see [`Database::config_from_meta`]).
///
/// # Errors
///
/// [`Error`] when a database cannot be read.
///
/// # Examples
///
/// ```
/// use pelicode_core::{Config, Language, extract_bytes};
/// use pelicode_db::{Database, PackageInfo};
/// use pelicode_detect::{Detector, Options, detect_all};
///
/// let dir = tempfile::tempdir().unwrap();
/// let config = Config { min_tokens: 0, ..Config::default() };
/// let source = b"def total(xs):\n    s = 0\n    for x in xs:\n        s += x * 2\n    return s\n";
/// let file = extract_bytes("calc.py".as_ref(), Language::Python, source, &config).unwrap();
/// let mut databases = Vec::new();
/// for ecosystem in ["pypi", "crates"] {
///     let mut db = Database::open(dir.path().join(format!("{ecosystem}.db")), &config).unwrap();
///     let package = PackageInfo {
///         ecosystem: ecosystem.into(), name: "calc".into(), version: "1.0".into(), ..PackageInfo::default()
///     };
///     db.index(&package, &[file.clone()]).unwrap();
///     databases.push(db);
/// }
/// let options = Options { min_tokens: 0, ..Options::default() };
/// let detectors: Vec<Detector> = databases
///     .iter()
///     .map(|db| Detector::new(db, options.clone()).unwrap())
///     .collect();
/// let reports = detect_all(&detectors, &[file], |_, _| true).unwrap();
/// let found: Vec<_> = reports[0].candidates.iter().map(|c| c.occurrences[0].ecosystem.as_str()).collect();
/// assert_eq!(found, ["pypi", "crates"]);
/// ```
pub fn detect_all(
    detectors: &[Detector<'_>],
    files: &[FileFeatures],
    select: impl Fn(&FileFeatures, &FunctionFeatures) -> bool,
) -> Result<Vec<Report>> {
    let Some(first) = detectors.first() else {
        return Ok(Vec::new());
    };
    let mut reports = Vec::new();
    for file in files {
        for function in &file.functions {
            if function.token_count < first.options.min_tokens || !select(file, function) {
                continue;
            }
            let mut candidates = Vec::new();
            for (database, detector) in detectors.iter().enumerate() {
                for mut candidate in detector.candidates(function)? {
                    candidate.database = database;
                    candidates.push(candidate);
                }
            }
            if candidates.is_empty() {
                continue;
            }
            candidates.sort_by(|a, b| {
                b.similarity
                    .total_cmp(&a.similarity)
                    .then(a.kind.cmp(&b.kind))
                    .then(a.database.cmp(&b.database))
                    .then(a.function.id.cmp(&b.function.id))
            });
            candidates.truncate(first.options.max_candidates);
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
    fn weights_follow_document_frequency() {
        let dir = tempfile::tempdir().unwrap();
        let db =
            Database::open(dir.path().join("t.db"), &pelicode_core::Config::default()).unwrap();
        let mut detector = Detector::new(&db, Options::default()).unwrap();
        detector.packages = 3;
        detector
            .document_frequencies
            .borrow_mut()
            .insert((Level::N1, 1), 3);
        detector
            .document_frequencies
            .borrow_mut()
            .insert((Level::N1, 2), 1);
        assert_eq!(detector.weight(Level::N1, 1).unwrap(), 1.0);
        detector.options.idf = true;
        assert!((detector.weight(Level::N1, 1).unwrap() - 2f64.ln()).abs() < 1e-9);
        assert!((detector.weight(Level::N1, 2).unwrap() - 4f64.ln()).abs() < 1e-9);
        detector.options.stop_packages = 3;
        assert_eq!(detector.weight(Level::N1, 1).unwrap(), 0.0);
    }

    #[test]
    fn merges_line_ranges() {
        let ranges = [(5, 6), (1, 2), (3, 4), (10, 12), (11, 11), (8, 8)];
        assert_eq!(merge(ranges.into_iter()), [(1, 6), (8, 8), (10, 12)]);
    }
}
