//! `pelicode scan`: reports stored functions similar to the checked ones.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Args;
use pelicode_core::{Config, Language, extract_bytes};
use pelicode_db::Database;
use pelicode_detect::{Candidate, Detector, MatchKind, Options, Report};
use serde::Serialize;

use crate::git::Changes;
use crate::{Format, extract_paths, open_output};

#[derive(Args)]
pub struct ScanArgs {
    /// Files or directories to check; with --git, the repository (default: .).
    paths: Vec<PathBuf>,
    /// Database file.
    #[arg(long, default_value = "pelicode.db")]
    db: PathBuf,
    /// Check only the functions changed in a git range (BASE, BASE..HEAD or BASE...HEAD).
    #[arg(long, value_name = "RANGE")]
    git: Option<String>,
    /// Minimum similarity (Jaccard of fingerprint sets) to report.
    #[arg(long, default_value_t = Options::default().threshold)]
    threshold: f64,
    /// Candidates reported per function.
    #[arg(long, default_value_t = Options::default().max_candidates)]
    max_candidates: usize,
    /// Functions with fewer tokens are not checked.
    #[arg(long, default_value_t = Options::default().min_tokens)]
    min_tokens: usize,
    /// Never report functions of this package (ECOSYSTEM:NAME); repeatable.
    #[arg(long, value_name = "ECOSYSTEM:NAME")]
    exclude: Vec<String>,
    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Output file (default: standard output).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Exit with status 1 when any candidate is found.
    #[arg(long)]
    fail_on_match: bool,
    /// Ignore hashes found in at least N packages, or in P% of the packages (0 disables).
    #[arg(long, value_name = "N|P%", default_value = "100")]
    stop_packages: StopThreshold,
    /// Minimum number of shared fingerprints (common ones excluded) for a similar match.
    #[arg(long, value_name = "N", default_value_t = Options::default().min_shared)]
    min_shared: usize,
    /// Weight fingerprints by inverse document frequency.
    #[arg(long)]
    idf: bool,
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
pub fn run(args: ScanArgs) -> Result<bool> {
    if !args.db.exists() {
        bail!("{}: no such database", args.db.display());
    }
    let db = Database::open_unchecked(&args.db)?;
    // Featurize the checked code exactly like the stored code.
    let config = db.config_from_meta(&Config {
        min_tokens: 0,
        ..Config::default()
    })?;
    let options = Options {
        min_tokens: args.min_tokens,
        threshold: args.threshold,
        max_candidates: args.max_candidates,
        exclude_packages: args
            .exclude
            .iter()
            .map(|p| match p.split_once(':') {
                Some((eco, name)) => Ok((eco.to_string(), name.to_string())),
                None => bail!("--exclude expects ECOSYSTEM:NAME, got {p}"),
            })
            .collect::<Result<_>>()?,
        stop_packages: args.stop_packages.resolve(db.package_count()?),
        min_shared: args.min_shared,
        idf: args.idf,
        ..Options::default()
    };
    if options.stop_packages > 0 || args.idf {
        let info = db.hash_stats_info()?;
        if info.updated_at.is_none() {
            eprintln!(
                "warning: hash statistics were never computed; common code is not filtered \
                 (run `pelicode stats --refresh`)"
            );
        } else if info.stale {
            eprintln!("warning: hash statistics are out of date (run `pelicode stats --refresh`)");
        }
    }
    let detector = Detector::new(&db, options)?;

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
                let content = changes.content(path)?;
                files.push(
                    extract_bytes(path, language, &content, &config)
                        .with_context(|| format!("cannot extract {}", path.display()))?,
                );
            }
            detector.detect(&files, |file, f| {
                changes.touches(&file.path, f.start_line, f.end_line)
            })?
        }
        None => {
            if args.paths.is_empty() {
                bail!("give files or directories to check, or --git RANGE");
            }
            let files = extract_paths(&args.paths, &config)?;
            detector.detect(&files, |_, _| true)?
        }
    };

    let mut out = open_output(args.output.as_deref())?;
    match args.format {
        Format::Text => text(&mut out, &reports)?,
        Format::Json => crate::output::json(&mut out, &reports)?,
        Format::Csv => csv(&mut out, &reports)?,
    }
    out.flush()?;
    Ok(args.fail_on_match && !reports.is_empty())
}

fn describe(kind: MatchKind) -> String {
    match kind {
        MatchKind::Exact(level) => format!("exact({level})"),
        MatchKind::Similar(level) => format!("similar({level})"),
    }
}

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

fn source(c: &Candidate) -> String {
    match c.occurrences.first() {
        Some(o) => format!(
            "{}:{}@{} {}:{}-{}",
            o.ecosystem, o.package, o.version, o.path, c.function.start_line, c.function.end_line
        ),
        None => "?".into(),
    }
}

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
