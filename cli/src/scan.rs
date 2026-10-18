//! `pelicode scan`: reports stored functions similar to the checked ones.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use globset::GlobSet;
use pelicode_core::{Config, Language, extract_bytes, slash_path};
use pelicode_db::Database;
use pelicode_detect::dependencies::Dependencies;
use pelicode_detect::{Candidate, Detector, MatchKind, Options, Report, detect_all};
use serde::Serialize;

use crate::git::Changes;
use crate::settings::{self, Loaded};
use crate::{extract_paths, open_output};

#[derive(Args)]
pub struct ScanArgs {
    /// Files or directories to check; with --git, the repository (default: .).
    paths: Vec<PathBuf>,
    /// Database file, or a snapshot as oci://REGISTRY/REPOSITORY:TAG; repeat to check against
    /// several [default: `database` of the settings file, else pelicode.db].
    #[arg(long)]
    db: Vec<PathBuf>,
    /// Check only the functions changed in a git range (BASE, BASE..HEAD or BASE...HEAD).
    #[arg(long, value_name = "RANGE")]
    git: Option<String>,
    /// Minimum similarity (Jaccard of fingerprint sets) to report [default: 0.7].
    #[arg(long)]
    threshold: Option<f64>,
    /// Candidates reported per function [default: 5].
    #[arg(long)]
    max_candidates: Option<usize>,
    /// Functions with fewer tokens are not checked [default: 50].
    #[arg(long)]
    min_tokens: Option<usize>,
    /// Check only these languages (comma-separated) [default: every supported language].
    #[arg(long, value_delimiter = ',')]
    languages: Vec<Language>,
    /// Never report functions of this package (ECOSYSTEM:NAME); repeatable, added to the
    /// settings file's.
    #[arg(long, value_name = "ECOSYSTEM:NAME")]
    exclude: Vec<String>,
    /// Do not check files whose path matches this glob (`*` within a directory, `**` across);
    /// repeatable, added to the settings file's.
    #[arg(long, value_name = "GLOB")]
    exclude_path: Vec<String>,
    /// Output format; sarif is for GitHub code scanning and other SARIF readers.
    #[arg(short, long, value_enum, default_value_t = ScanFormat::Text)]
    format: ScanFormat,
    /// Output file (default: standard output).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Exit with status 1 when any candidate is found.
    #[arg(long)]
    fail_on_match: bool,
    /// Ignore hashes found in at least N packages, or in P% of the packages (0 disables)
    /// [default: 100].
    #[arg(long, value_name = "N|P%")]
    stop_packages: Option<StopThreshold>,
    /// Minimum number of shared fingerprints (common ones excluded) for a similar match
    /// [default: 5].
    #[arg(long, value_name = "N")]
    min_shared: Option<usize>,
    /// Weight fingerprints by inverse document frequency.
    #[arg(long)]
    idf: bool,
    /// Report candidates found only in the dependencies of the checked code too. By default,
    /// they are left out; the dependencies come from the lock files and manifests (Cargo.lock,
    /// go.mod, requirements*.txt, poetry.lock, uv.lock, Pipfile.lock, Gemfile.lock, pom.xml,
    /// gradle.lockfile) in the checked directories and above, up to the repository root.
    #[arg(long)]
    no_exclude_dependencies: bool,
}

/// The dependencies of what `args` checks: those around the repository with
/// `--git`, else around each path given.
fn dependencies_of(args: &ScanArgs) -> Dependencies {
    let roots: Vec<PathBuf> = if args.git.is_some() || args.paths.is_empty() {
        vec![
            args.paths
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from(".")),
        ]
    } else {
        args.paths.clone()
    };
    let mut dependencies = Dependencies::default();
    for root in roots {
        dependencies.extend(Dependencies::find_around(&root));
    }
    dependencies
}

/// Output formats of `scan`.
#[derive(Clone, Copy, ValueEnum)]
enum ScanFormat {
    Text,
    Json,
    Csv,
    Sarif,
}

/// The path of a report relative to where pelicode runs, normally the root of
/// the repository, as SARIF readers expect: the report paths are relative to
/// the directory given (or to the repository with `--git`).
fn repository_uri(paths: &[PathBuf], git: bool, report: &Path) -> String {
    let base: Option<&Path> = match paths {
        [] => None,
        [path] if git || path.is_dir() => Some(path),
        // A single file is reported with the path it was given.
        [_] => None,
        // Several paths: the report path starts with the name of its directory.
        _ => {
            let first = report.components().next().map(|c| c.as_os_str().to_owned());
            paths
                .iter()
                .filter(|p| p.is_dir())
                .find(|p| {
                    p.canonicalize()
                        .ok()
                        .and_then(|c| c.file_name().map(|n| n.to_owned()))
                        == first
                })
                .and_then(|p| p.parent())
        }
    };
    let joined = match base {
        Some(base) => base.join(report),
        None => report.to_path_buf(),
    };
    let relative = match std::env::current_dir() {
        Ok(cwd) if joined.is_absolute() => joined
            .strip_prefix(&cwd)
            .map(Path::to_path_buf)
            .unwrap_or(joined),
        _ => joined,
    };
    let text = slash_path(&relative).to_string_lossy().into_owned();
    let mut uri = text.as_str();
    while let Some(rest) = uri.strip_prefix("./") {
        uri = rest;
    }
    uri.to_string()
}

/// The options of a scan: the command line, else the settings file, else the defaults.
struct Resolved {
    options: Options,
    stop_packages: StopThreshold,
    fail_on_match: bool,
    exclude_dependencies: bool,
    languages: Option<Vec<Language>>,
    exclude_paths: GlobSet,
}

impl Resolved {
    fn new(args: &ScanArgs, loaded: Option<&Loaded>) -> Result<Resolved> {
        let defaults = Options::default();
        let file = loaded.map(|l| &l.settings.scan);
        let from_file = |get: fn(&settings::ScanSettings) -> Option<usize>| file.and_then(get);

        let mut packages = file.map(|f| f.exclude.packages.clone()).unwrap_or_default();
        packages.extend(args.exclude.iter().cloned());
        let mut paths = file.map(|f| f.exclude.paths.clone()).unwrap_or_default();
        paths.extend(args.exclude_path.iter().cloned());

        let threshold = args
            .threshold
            .or_else(|| file.and_then(|f| f.threshold))
            .unwrap_or(defaults.threshold);
        if !(0.0..=1.0).contains(&threshold) {
            bail!("--threshold must be between 0 and 1, got {threshold}");
        }
        let stop_packages = match args.stop_packages {
            Some(stop) => stop,
            None => match loaded {
                Some(l) => l.stop_packages()?,
                None => None,
            }
            .unwrap_or(StopThreshold::Count(defaults.stop_packages)),
        };
        Ok(Resolved {
            options: Options {
                min_tokens: args
                    .min_tokens
                    .or_else(|| from_file(|f| f.min_tokens))
                    .unwrap_or(defaults.min_tokens),
                threshold,
                max_candidates: args
                    .max_candidates
                    .or_else(|| from_file(|f| f.max_candidates))
                    .unwrap_or(defaults.max_candidates),
                min_shared: args
                    .min_shared
                    .or_else(|| from_file(|f| f.min_shared))
                    .unwrap_or(defaults.min_shared),
                idf: args.idf || file.and_then(|f| f.idf).unwrap_or(false),
                exclude_packages: packages
                    .iter()
                    .map(|p| settings::parse_package(p).context("--exclude"))
                    .collect::<Result<_>>()?,
                ..defaults
            },
            stop_packages,
            fail_on_match: args.fail_on_match
                || file.and_then(|f| f.fail_on_match).unwrap_or(false),
            exclude_dependencies: !args.no_exclude_dependencies
                && file.and_then(|f| f.exclude_dependencies).unwrap_or(true),
            languages: if args.languages.is_empty() {
                file.and_then(|f| f.languages.clone())
            } else {
                Some(args.languages.clone())
            },
            exclude_paths: settings::glob_set(&paths).context("--exclude-path")?,
        })
    }

    /// Whether a file is checked: its language is selected and its path not excluded.
    fn accepts(&self, path: &Path, language: Language) -> bool {
        self.languages
            .as_ref()
            .is_none_or(|languages| languages.contains(&language))
            && !self.exclude_paths.is_match(path)
    }
}

/// A number of packages, absolute or relative to the database.
#[derive(Clone, Copy, Debug)]
pub enum StopThreshold {
    Count(usize),
    Percent(f64),
}

impl StopThreshold {
    /// The absolute threshold for a database of `packages` packages; at least 2,
    /// since every stored hash is found in one package.
    fn resolve(self, packages: usize) -> usize {
        match self {
            StopThreshold::Count(n) => n,
            StopThreshold::Percent(p) if p <= 0.0 => 0,
            StopThreshold::Percent(p) => ((p / 100.0 * packages as f64).ceil() as usize).max(2),
        }
    }
}

impl std::str::FromStr for StopThreshold {
    type Err = String;

    /// Parses `N` as a count and `P%` as a percentage from 0 to 100.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("expected a count or a percentage, got {s}");
        match s.strip_suffix('%') {
            Some(p) => p
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|p| (0.0..=100.0).contains(p))
                .map(StopThreshold::Percent)
                .ok_or_else(invalid),
            None => s
                .trim()
                .parse()
                .map(StopThreshold::Count)
                .map_err(|_| invalid()),
        }
    }
}

/// Runs the scan; returns whether any candidate was found.
pub fn run(args: ScanArgs, settings: Option<&Loaded>) -> Result<bool> {
    let mut databases = Vec::new();
    for db in crate::databases(&args.db, settings) {
        let path = crate::oci::resolve(&db)?;
        if !path.exists() {
            bail!("{}: no such database", path.display());
        }
        let opened = Database::open_unchecked(&path)?;
        databases.push((db, opened));
    }
    // Featurize the checked code exactly like the stored code.
    let config = common_config(&databases)?;
    let resolved = Resolved::new(&args, settings)?;
    let dependencies = if resolved.exclude_dependencies {
        dependencies_of(&args)
    } else {
        Dependencies::default()
    };
    let dependency_files = dependencies.files.clone();
    let mut detectors = Vec::new();
    for (name, db) in &databases {
        let options = Options {
            stop_packages: resolved.stop_packages.resolve(db.package_count()?),
            dependencies: dependencies.clone(),
            ..resolved.options.clone()
        };
        if options.stop_packages > 0 || options.idf {
            let info = db.hash_stats_info()?;
            let prefix = if databases.len() > 1 {
                format!("{}: ", name.display())
            } else {
                String::new()
            };
            if info.updated_at.is_none() {
                eprintln!(
                    "warning: {prefix}hash statistics were never computed; common code is not \
                     filtered (run `pelicode stats --refresh`)"
                );
            } else if info.stale {
                eprintln!(
                    "warning: {prefix}hash statistics are out of date (run `pelicode stats --refresh`)"
                );
            }
        }
        detectors.push(Detector::new(db, options)?);
    }

    let reports = match &args.git {
        Some(range) => {
            let repo = match args.paths.as_slice() {
                [] => PathBuf::from("."),
                [repo] => repo.clone(),
                _ => bail!("--git takes a single repository path"),
            };
            let changes = Changes::of(&repo, range)?;
            let mut files = Vec::new();
            for path in changes.files.keys() {
                let Some(language) = Language::from_path(path) else {
                    continue;
                };
                if !resolved.accepts(path, language) {
                    continue;
                }
                let content = changes.content(path)?;
                files.push(
                    extract_bytes(path, language, &content, &config)
                        .with_context(|| format!("cannot extract {}", path.display()))?,
                );
            }
            detect_all(&detectors, &files, |file, f| {
                changes.touches(&file.path, f.start_line, f.end_line)
            })?
        }
        None => {
            if args.paths.is_empty() {
                bail!("give files or directories to check, or --git RANGE");
            }
            let mut files = extract_paths(&args.paths, &config)?;
            files.retain(|f| resolved.accepts(&f.path, f.language));
            detect_all(&detectors, &files, |_, _| true)?
        }
    };

    let excluded: usize = detectors
        .iter()
        .map(Detector::excluded_as_dependencies)
        .sum();
    if excluded > 0 {
        let mut files: Vec<String> = dependency_files
            .iter()
            .take(3)
            .map(|f| f.display().to_string())
            .collect();
        if dependency_files.len() > 3 {
            files.push(format!("and {} more", dependency_files.len() - 3));
        }
        eprintln!(
            "note: {excluded} candidate(s) found only in dependencies of the checked code were not \
             reported (dependencies read from {}); --no-exclude-dependencies shows them",
            files.join(", ")
        );
    }

    let mut out = open_output(args.output.as_deref())?;
    match args.format {
        ScanFormat::Text => text(&mut out, &reports)?,
        ScanFormat::Json => crate::output::json(&mut out, &reports)?,
        ScanFormat::Csv => csv(&mut out, &reports)?,
        ScanFormat::Sarif => crate::sarif::write(&mut out, &reports, |r| {
            repository_uri(&args.paths, args.git.is_some(), &r.path)
        })?,
    }
    out.flush()?;
    Ok(resolved.fail_on_match && !reports.is_empty())
}

/// The parameters to featurize the checked code with: those of the databases,
/// which must agree, with the fingerprint levels of all of them.
fn common_config(databases: &[(PathBuf, Database)]) -> Result<Config> {
    let base = Config {
        min_tokens: 0,
        ..Config::default()
    };
    let mut config: Option<(Config, &Path)> = None;
    for (name, db) in databases {
        let this = db.config_from_meta(&base)?;
        let Some((first, first_name)) = &mut config else {
            config = Some((this, name));
            continue;
        };
        let differs = [
            (
                "ident_mode",
                first.ident_mode.to_string(),
                this.ident_mode.to_string(),
            ),
            ("kgram", first.kgram.to_string(), this.kgram.to_string()),
            ("window", first.window.to_string(), this.window.to_string()),
            (
                "minhash_size",
                first.minhash_size.to_string(),
                this.minhash_size.to_string(),
            ),
            ("bands", first.bands.to_string(), this.bands.to_string()),
        ]
        .into_iter()
        .find(|(_, a, b)| a != b);
        if let Some((key, a, b)) = differs {
            bail!(
                "{} and {} were built with different parameters ({key} = {a} and {b}); \
                 they cannot be checked against at once",
                first_name.display(),
                name.display()
            );
        }
        for level in this.fingerprint_levels {
            if !first.fingerprint_levels.contains(&level) {
                first.fingerprint_levels.push(level);
            }
        }
        first.fingerprint_levels.sort();
    }
    Ok(config.expect("at least one database").0)
}

/// `exact(N1)`, `similar(N1)` and so on.
fn describe(kind: MatchKind) -> String {
    match kind {
        MatchKind::Exact(level) => format!("exact({level})"),
        MatchKind::Similar(level) => format!("similar({level})"),
    }
}

/// Line ranges as `1-3,5`.
fn ranges(lines: &[(u32, u32)]) -> String {
    lines
        .iter()
        .map(|&(s, e)| {
            if s == e {
                s.to_string()
            } else {
                format!("{s}-{e}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `ecosystem:package@version path:start-end` of the first occurrence of a candidate.
fn source(c: &Candidate) -> String {
    match c.occurrences.first() {
        Some(o) => format!(
            "{}:{}@{} {}:{}-{}",
            o.ecosystem, o.package, o.version, o.path, c.function.start_line, c.function.end_line
        ),
        None => "?".into(),
    }
}

/// Text output of `scan`: each function with candidates, followed by its candidates.
fn text(out: &mut dyn Write, reports: &[Report]) -> Result<()> {
    if reports.is_empty() {
        writeln!(out, "No similar code candidates found.")?;
        return Ok(());
    }
    writeln!(
        out,
        "{} function(s) have similar code candidates to review.\n",
        reports.len()
    )?;
    for r in reports {
        writeln!(
            out,
            "{}:{}-{} {} (tokens={})",
            r.path.display(),
            r.start_line,
            r.end_line,
            r.name,
            r.token_count
        )?;
        for c in &r.candidates {
            let license = c
                .occurrences
                .first()
                .and_then(|o| o.license.as_deref())
                .unwrap_or("license unknown");
            let more = match c.occurrence_count {
                0 | 1 => String::new(),
                n => format!(" (+{} more)", n - 1),
            };
            writeln!(
                out,
                "  {:<12} {:.2}  {} [{}]{}",
                describe(c.kind),
                c.similarity,
                source(c),
                license,
                more
            )?;
            if let MatchKind::Similar(_) = c.kind {
                writeln!(
                    out,
                    "               lines {} <-> {} (containment {:.2}, {} shared fingerprints)",
                    ranges(&c.matched_lines),
                    ranges(&c.candidate_lines),
                    c.containment,
                    c.shared_fingerprints
                )?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct Record<'a> {
    path: String,
    name: &'a str,
    start_line: usize,
    end_line: usize,
    token_count: usize,
    #[serde(rename = "match")]
    kind: String,
    similarity: f64,
    containment: f64,
    shared_fingerprints: usize,
    matched_lines: String,
    candidate_ecosystem: &'a str,
    candidate_package: &'a str,
    candidate_version: &'a str,
    candidate_path: &'a str,
    candidate_name: &'a str,
    candidate_start_line: usize,
    candidate_end_line: usize,
    candidate_lines: String,
    candidate_license: Option<&'a str>,
    occurrence_count: usize,
}

/// CSV output of `scan`: one row per candidate.
fn csv(out: &mut dyn Write, reports: &[Report]) -> Result<()> {
    let mut writer = csv::Writer::from_writer(out);
    for r in reports {
        for c in &r.candidates {
            let o = c.occurrences.first();
            writer.serialize(Record {
                path: r.path.to_string_lossy().into_owned(),
                name: &r.name,
                start_line: r.start_line,
                end_line: r.end_line,
                token_count: r.token_count,
                kind: describe(c.kind),
                similarity: c.similarity,
                containment: c.containment,
                shared_fingerprints: c.shared_fingerprints,
                matched_lines: ranges(&c.matched_lines),
                candidate_ecosystem: o.map_or("", |o| &o.ecosystem),
                candidate_package: o.map_or("", |o| &o.package),
                candidate_version: o.map_or("", |o| &o.version),
                candidate_path: o.map_or("", |o| &o.path),
                candidate_name: &c.function.name,
                candidate_start_line: c.function.start_line,
                candidate_end_line: c.function.end_line,
                candidate_lines: ranges(&c.candidate_lines),
                candidate_license: o.and_then(|o| o.license.as_deref()),
                occurrence_count: c.occurrence_count,
            })?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_thresholds() {
        let parse = |s: &str| s.parse::<StopThreshold>().unwrap();
        assert_eq!(parse("100").resolve(25), 100);
        assert_eq!(parse("10%").resolve(25), 3);
        assert_eq!(parse("1%").resolve(25), 2);
        assert_eq!(parse("0%").resolve(25), 0);
        assert!("abc".parse::<StopThreshold>().is_err());
        assert!("120%".parse::<StopThreshold>().is_err());
    }
}
