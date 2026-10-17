mod crawl;
mod git;
mod output;
mod scan;
mod stats;

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pelicode_core::{
    Config, FileFeatures, IdentMode, Language, Level, discover, extract_all, slash_path,
};
use pelicode_db::{Database, PackageInfo};

/// Detects code similar to known open-source code at the function level.
#[derive(Parser)]
#[command(name = "pelicode", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Extracts the functions of every supported source file under the given
    /// directories and stores their features in the database.
    Index(IndexArgs),
    /// Extracts the functions of the given files or directories and prints their features.
    Extract(ExtractArgs),
    /// Prints the functions stored in the database.
    Export(ExportArgs),
    /// Reports stored functions similar to the functions of the given code.
    Scan(scan::ScanArgs),
    /// Fetches packages from a public repository (pypi, maven, go, rubygems, crates) and indexes them.
    Crawl(crawl::CrawlArgs),
    /// Shows database statistics and the hashes shared by the most packages.
    Stats(stats::StatsArgs),
    /// Lists the supported languages and their file extensions.
    Languages,
}

#[derive(Args)]
struct FeatureArgs {
    /// Functions with fewer tokens get exact hashes only (no fingerprints).
    #[arg(long, default_value_t = Config::default().min_tokens)]
    min_tokens: usize,
    /// How identifiers are abstracted.
    #[arg(long, default_value = "uniform")]
    ident_mode: IdentMode,
    /// Normalization levels for which fingerprints are computed.
    #[arg(long, value_delimiter = ',', default_value = "N1")]
    levels: Vec<Level>,
    /// k of the k-grams.
    #[arg(long, default_value_t = Config::default().kgram)]
    kgram: usize,
    /// Window size of winnowing.
    #[arg(long, default_value_t = Config::default().window)]
    window: usize,
    /// Length of MinHash signatures.
    #[arg(long, default_value_t = Config::default().minhash_size)]
    minhash_size: usize,
    /// Number of LSH bands.
    #[arg(long, default_value_t = Config::default().bands)]
    bands: usize,
}

impl FeatureArgs {
    fn config(&self, keep_tokens: bool) -> Result<Config> {
        let config = Config {
            min_tokens: self.min_tokens,
            ident_mode: self.ident_mode,
            kgram: self.kgram,
            window: self.window,
            minhash_size: self.minhash_size,
            bands: self.bands,
            fingerprint_levels: self.levels.clone(),
            keep_tokens,
        };
        config.validate()?;
        Ok(config)
    }
}

#[derive(Args)]
struct IndexArgs {
    /// Project directories (or files) to index.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Database file.
    #[arg(long, default_value = "pelicode.db")]
    db: PathBuf,
    /// Ecosystem the project belongs to.
    #[arg(long, default_value = "local")]
    ecosystem: String,
    /// Package name (default: the name of the first directory).
    #[arg(long)]
    name: Option<String>,
    /// Package version.
    #[arg(long, default_value = "unversioned")]
    version: String,
    /// License of the project (SPDX identifier).
    #[arg(long)]
    license: Option<String>,
    /// Repository URL of the project.
    #[arg(long)]
    repository: Option<String>,
    /// Format of the summary.
    #[arg(short, long, value_enum, default_value_t = SummaryFormat::Text)]
    format: SummaryFormat,
    /// Rebuild the per-hash package counts afterwards (see `pelicode stats`).
    #[arg(long)]
    refresh_stats: bool,
    #[command(flatten)]
    features: FeatureArgs,
}

#[derive(Args)]
struct ExtractArgs {
    /// Files or directories to extract.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Output file (default: standard output).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Include the normalized tokens (of LEVEL in text and CSV; of every level in JSON).
    #[arg(long, value_name = "LEVEL", num_args = 0..=1, default_missing_value = "N1")]
    tokens: Option<Level>,
    #[command(flatten)]
    features: FeatureArgs,
}

#[derive(Args)]
struct ExportArgs {
    /// Database file.
    #[arg(long, default_value = "pelicode.db")]
    db: PathBuf,
    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Output file (default: standard output).
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Csv,
}

#[derive(Clone, Copy, ValueEnum)]
enum SummaryFormat {
    Text,
    Json,
}

fn main() -> Result<()> {
    match run(Cli::parse()) {
        // Output piped into `head` and the like.
        Err(e) if is_broken_pipe(&e) => Ok(()),
        other => other,
    }
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::BrokenPipe)
            || cause
                .downcast_ref::<csv::Error>()
                .is_some_and(|c| matches!(c.kind(), csv::ErrorKind::Io(io) if io.kind() == io::ErrorKind::BrokenPipe))
            || cause
                .downcast_ref::<serde_json::Error>()
                .is_some_and(|j| j.io_error_kind() == Some(io::ErrorKind::BrokenPipe))
    })
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Index(args) => index(args),
        Command::Extract(args) => extract(args),
        Command::Export(args) => export(args),
        Command::Stats(args) => stats::run(args),
        Command::Crawl(args) => {
            if !crawl::run(args)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Scan(args) => {
            if scan::run(args)? {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Languages => {
            for lang in Language::ALL {
                println!("{:<8} {}", lang.name(), lang.extensions().join(", "));
            }
            Ok(())
        }
    }
}

/// Extracts every path; files that fail are reported on stderr and skipped.
/// Paths of files under a directory are relative to it; with several
/// directories they are prefixed with the directory name so that, e.g.,
/// `json/__init__.py` and `http/__init__.py` stay distinct.
fn extract_paths(paths: &[PathBuf], config: &Config) -> Result<Vec<FileFeatures>> {
    let mut results = Vec::new();
    for path in paths {
        let files = discover(path).with_context(|| format!("cannot scan {}", path.display()))?;
        let prefix = (paths.len() > 1 && path.is_dir())
            .then(|| path.canonicalize().ok()?.file_name().map(PathBuf::from))
            .flatten();
        for result in extract_all(&files, config) {
            match result {
                Ok(mut file) => {
                    if let Some(prefix) = &prefix {
                        file.path = slash_path(&prefix.join(&file.path));
                    }
                    results.push(file);
                }
                Err(e) => eprintln!("warning: {e}"),
            }
        }
    }
    Ok(results)
}

fn index(args: IndexArgs) -> Result<()> {
    let config = args.features.config(false)?;
    let name = match &args.name {
        Some(name) => name.clone(),
        None => default_name(&args.paths[0])?,
    };
    let files = extract_paths(&args.paths, &config)?;
    let mut db = Database::open(&args.db, &config)
        .with_context(|| format!("cannot open {}", args.db.display()))?;
    let package = PackageInfo {
        ecosystem: args.ecosystem,
        name,
        version: args.version,
        license_spdx: args.license,
        repository_url: args.repository,
        ..PackageInfo::default()
    };
    let summary = db.index(&package, &files)?;
    if args.refresh_stats {
        db.refresh_hash_stats()?;
    }
    let stats = db.stats()?;
    match args.format {
        SummaryFormat::Text => {
            println!(
                "indexed {}:{}@{} into {}",
                package.ecosystem,
                package.name,
                package.version,
                args.db.display()
            );
            println!(
                "  files: {} ({} new), functions: {} new ({} with fingerprints)",
                summary.files, summary.new_files, summary.functions, summary.indexed_functions
            );
            println!(
                "  database: {} packages, {} versions, {} files, {} functions",
                stats.packages, stats.versions, stats.source_files, stats.functions
            );
        }
        SummaryFormat::Json => {
            let value = serde_json::json!({
                "package": {
                    "ecosystem": package.ecosystem,
                    "name": package.name,
                    "version": package.version,
                },
                "summary": summary,
                "database": stats,
            });
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
    }
    Ok(())
}

fn default_name(path: &Path) -> Result<String> {
    let path = path
        .canonicalize()
        .with_context(|| format!("cannot access {}", path.display()))?;
    let dir = if path.is_file() {
        path.parent().unwrap_or(&path)
    } else {
        &path
    };
    match dir.file_name() {
        Some(name) => Ok(name.to_string_lossy().into_owned()),
        None => bail!(
            "cannot infer a package name from {}; use --name",
            path.display()
        ),
    }
}

fn extract(args: ExtractArgs) -> Result<()> {
    let config = args.features.config(args.tokens.is_some())?;
    let files = extract_paths(&args.paths, &config)?;
    let mut out = open_output(args.output.as_deref())?;
    match args.format {
        Format::Text => output::extract_text(&mut out, &files, args.tokens)?,
        Format::Json => output::json(&mut out, &files)?,
        Format::Csv => output::extract_csv(&mut out, &files, args.tokens)?,
    }
    out.flush()?;
    Ok(())
}

fn export(args: ExportArgs) -> Result<()> {
    if !args.db.exists() {
        bail!("{}: no such database", args.db.display());
    }
    let db = Database::open_unchecked(&args.db)?;
    let rows = db.functions()?;
    let mut out = open_output(args.output.as_deref())?;
    match args.format {
        Format::Text => output::export_text(&mut out, &rows)?,
        Format::Json => output::json(&mut out, &rows)?,
        Format::Csv => output::export_csv(&mut out, &rows)?,
    }
    out.flush()?;
    Ok(())
}

fn open_output(path: Option<&Path>) -> Result<Box<dyn Write>> {
    Ok(match path {
        Some(path) => Box::new(BufWriter::new(
            File::create(path).with_context(|| format!("cannot create {}", path.display()))?,
        )),
        None => Box::new(BufWriter::new(io::stdout().lock())),
    })
}
