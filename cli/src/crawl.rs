//! `pelicode crawl`: fetches packages from a public repository and indexes them.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Args;
use pelicode_crawl::ecosystems::{self, Ranking};
use pelicode_crawl::{Client, Crawler, Ecosystem, Outcome, crates, pypi};
use pelicode_db::Database;

use crate::FeatureArgs;
use crate::settings::Loaded;

#[derive(Args)]
pub struct CrawlArgs {
    /// Repository to crawl: pypi, maven, go, rubygems or crates.
    ecosystem: Ecosystem,
    /// Packages: NAME[@VERSION] for pypi, rubygems and crates, GROUP:ARTIFACT[:VERSION] for
    /// maven, MODULE[@VERSION] for go.
    packages: Vec<String>,
    /// File listing packages, one per line (`#` starts a comment).
    #[arg(long)]
    list: Option<PathBuf>,
    /// Also crawl the N best-known packages: the most downloaded of pypi, crates and rubygems,
    /// and those most depended on of maven and go (from ecosyste.ms).
    #[arg(long, value_name = "N")]
    top: Option<usize>,
    /// Also check every package of the ecosystem already in the database for a newer version,
    /// and crawl the new ones.
    #[arg(long)]
    updates: bool,
    /// Database file [default: `database` of the settings file, else pelicode.db].
    #[arg(long)]
    db: Option<PathBuf>,
    /// Re-index versions that are already in the database.
    #[arg(long)]
    force: bool,
    /// Rebuild the per-hash package counts afterwards (see `pelicode stats`).
    #[arg(long)]
    refresh_stats: bool,
    /// Minimum interval between requests, in milliseconds [default: `crawl.delay_ms` of the
    /// settings file, else 1000].
    #[arg(long)]
    delay_ms: Option<u64>,
    /// Contact URL or e-mail address added to the User-Agent [default: `crawl.contact` of the
    /// settings file].
    #[arg(long)]
    contact: Option<String>,
    #[command(flatten)]
    features: FeatureArgs,
}

/// Returns whether every package was indexed or skipped.
pub fn run(args: CrawlArgs, settings: Option<&Loaded>) -> Result<bool> {
    let config = args.features.config(false)?;
    let crawl = settings.map(|s| &s.settings.crawl);
    let contact = args
        .contact
        .clone()
        .or_else(|| crawl.and_then(|c| c.contact.clone()));
    let delay_ms = args
        .delay_ms
        .or_else(|| crawl.and_then(|c| c.delay_ms))
        .unwrap_or(1000);
    let client = Client::new(contact.as_deref(), Duration::from_millis(delay_ms));

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
        let top = match args.ecosystem {
            Ecosystem::PyPI => pypi::top_packages(&client, n)?,
            Ecosystem::Crates => crates::top_packages(&client, n)?,
            Ecosystem::RubyGems => {
                ecosystems::top_packages(&client, "rubygems.org", Ranking::Downloads, n)?
            }
            Ecosystem::Maven => ecosystems::top_packages(
                &client,
                "repo1.maven.org",
                Ranking::DependentRepositories,
                n,
            )?,
            Ecosystem::Go => ecosystems::top_packages(
                &client,
                "proxy.golang.org",
                Ranking::DependentRepositories,
                n,
            )?,
        };
        packages.extend(top);
    }
    if args.ecosystem == Ecosystem::Crates && contact.is_none() {
        eprintln!(
            "warning: the crates.io data access policy strongly suggests a way to contact \
             the crawler's operator; please set --contact"
        );
    }
    let db_path = crate::database(&args.db, settings)?;
    let mut db = Database::open(&db_path, &config)
        .with_context(|| format!("cannot open {}", db_path.display()))?;
    let ecosystem = args.ecosystem.name();
    if args.updates {
        let tracked = db.package_names(ecosystem)?;
        let last = db
            .updates_checked_at(ecosystem)?
            .map(|t| format!("last checked {t}"))
            .unwrap_or_else(|| "never checked before".into());
        eprintln!(
            "checking {} {ecosystem} packages for new versions ({last})",
            tracked.len()
        );
        packages.extend(tracked);
    }
    // The same package may come from the arguments, --list, --top and --updates.
    let mut seen = std::collections::HashSet::new();
    packages.retain(|p| seen.insert(p.clone()));
    if packages.is_empty() {
        if args.updates {
            println!("no {ecosystem} packages in {}", db_path.display());
            return Ok(true);
        }
        bail!("give packages, --list FILE, --top N or --updates");
    }

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
    if args.updates {
        db.record_updates_check(ecosystem)?;
    }
    if args.refresh_stats {
        let info = db.refresh_hash_stats()?;
        println!(
            "hash statistics refreshed: {} exact hashes and {} fingerprints in 2+ of {} packages",
            info.common_exact_hashes, info.common_fingerprints, info.packages
        );
    }
    Ok(failed == 0)
}
