//! `pelicode crawl`: fetches packages from a public repository and indexes them.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use pelicode_crawl::{Client, Crawler, Ecosystem, Outcome, pypi};
use pelicode_db::Database;

use crate::FeatureArgs;

#[derive(Args)]
pub struct CrawlArgs {
    /// Repository to crawl: pypi or maven.
    ecosystem: Ecosystem,
    /// Packages: NAME[@VERSION] for pypi, GROUP:ARTIFACT[:VERSION] for maven.
    packages: Vec<String>,
    /// File listing packages, one per line (`#` starts a comment).
    #[arg(long)]
    list: Option<PathBuf>,
    /// Also crawl the N most downloaded packages (pypi only).
    #[arg(long, value_name = "N")]
    top: Option<usize>,
    /// Database file.
    #[arg(long, default_value = "pelicode.db")]
    db: PathBuf,
    /// Re-index versions that are already in the database.
    #[arg(long)]
    force: bool,
    /// Minimum interval between requests, in milliseconds.
    #[arg(long, default_value_t = 1000)]
    delay_ms: u64,
    /// Contact URL or e-mail address added to the User-Agent.
    #[arg(long)]
    contact: Option<String>,
    #[command(flatten)]
    features: FeatureArgs,
}

/// Returns whether every package was indexed or skipped.
pub fn run(args: CrawlArgs) -> Result<bool> {
    let config = args.features.config(false)?;
    let client = Client::new(
        args.contact.as_deref(),
        Duration::from_millis(args.delay_ms),
    );

    let mut packages = args.packages.clone();
    if let Some(list) = &args.list {
        let text = std::fs::read_to_string(list)
            .with_context(|| format!("cannot read {}", list.display()))?;
        packages.extend(
            text.lines()
                .map(|l| l.split('#').next().unwrap_or("").trim())
                .filter(|l| !l.is_empty())
                .map(String::from),
        );
    }
    if let Some(n) = args.top {
        if args.ecosystem != Ecosystem::PyPI {
            bail!("--top is available for pypi only");
        }
        packages.extend(pypi::top_packages(&client, n)?);
    }
    if packages.is_empty() {
        bail!("give packages, --list FILE or --top N");
    }

    let mut db = Database::open(&args.db, &config)
        .with_context(|| format!("cannot open {}", args.db.display()))?;
    let crawler = Crawler::new(client, config, args.force);
    let (mut indexed, mut skipped, mut failed) = (0, 0, 0);
    for (i, spec) in packages.iter().enumerate() {
        let progress = format!("[{}/{}]", i + 1, packages.len());
        match crawler.crawl(&mut db, args.ecosystem, spec) {
            Ok(Outcome::Indexed {
                package,
                summary,
                failed_files,
            }) => {
                indexed += 1;
                println!(
                    "{progress} indexed {}:{}@{} ({} files, {} new functions{}{})",
                    package.ecosystem,
                    package.name,
                    package.version,
                    summary.files,
                    summary.functions,
                    package
                        .license_spdx
                        .as_deref()
                        .map(|l| format!(", {l}"))
                        .unwrap_or_default(),
                    if failed_files > 0 {
                        format!(", {failed_files} files failed")
                    } else {
                        String::new()
                    },
                );
            }
            Ok(Outcome::Skipped { package }) => {
                skipped += 1;
                println!(
                    "{progress} skipped {}:{}@{} (already indexed)",
                    package.ecosystem, package.name, package.version
                );
            }
            Err(e) => {
                failed += 1;
                eprintln!("{progress} failed {spec}: {e}");
            }
        }
    }
    println!("{indexed} indexed, {skipped} skipped, {failed} failed");
    Ok(failed == 0)
}
