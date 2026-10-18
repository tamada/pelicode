//! `pelicode stats`: table sizes and the hashes shared by the most packages.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;
use pelicode_core::Level;
use pelicode_db::{CommonHash, Database};

use crate::SummaryFormat;
use crate::settings::Loaded;

#[derive(Args)]
pub struct StatsArgs {
    /// Database file [default: `database` of the settings file, else pelicode.db].
    #[arg(long)]
    db: Option<PathBuf>,
    /// Rebuild the per-hash package counts first.
    #[arg(long)]
    refresh: bool,
    /// With --refresh, recount every hash instead of only those of what was indexed since the
    /// last statistics.
    #[arg(long, requires = "refresh")]
    full: bool,
    /// Number of most common hashes to list.
    #[arg(long, default_value_t = 10)]
    top: usize,
    /// Normalization level of the listed hashes.
    #[arg(long, default_value = "N1")]
    level: Level,
    /// List exact hashes of functions with at least this many tokens only.
    #[arg(long, default_value_t = 50)]
    min_tokens: usize,
    /// Output format.
    #[arg(short, long, value_enum, default_value_t = SummaryFormat::Text)]
    format: SummaryFormat,
}

/// Runs `pelicode stats`, refreshing the hash statistics first with `--refresh`.
pub fn run(args: StatsArgs, settings: Option<&Loaded>) -> Result<()> {
    let db_path = crate::database(&args.db, settings);
    if !db_path.exists() {
        bail!("{}: no such database", db_path.display());
    }
    let mut db = Database::open_unchecked(&db_path)?;
    let mut refreshed = None;
    if args.refresh {
        let before = db.hash_stats_info()?;
        let incremental = before.incremental && !args.full;
        if incremental {
            db.refresh_hash_stats()?;
        } else {
            db.rebuild_hash_stats()?;
        }
        refreshed = Some((incremental, before.pending_hashes));
    }
    let tables = db.stats()?;
    let hashes = db.hash_stats_info()?;
    let exact = db.common_exact_hashes(args.level, args.min_tokens, args.top)?;
    let fingerprints = db.common_fingerprints(args.level, args.top)?;

    match args.format {
        SummaryFormat::Json => {
            let value = serde_json::json!({
                "tables": tables,
                "hash_stats": hashes,
                "refreshed": refreshed.map(|(incremental, recounted)| serde_json::json!({
                    "incremental": incremental,
                    "recounted_hashes": if incremental { Some(recounted) } else { None },
                })),
                "common_exact_hashes": exact,
                "common_fingerprints": fingerprints,
            });
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        SummaryFormat::Text => {
            println!(
                "{} ecosystems, {} packages, {} versions, {} files, {} functions ({} with fingerprints)",
                tables.ecosystems,
                tables.packages,
                tables.versions,
                tables.source_files,
                tables.functions,
                tables.indexed_functions
            );
            if let Some((incremental, pending)) = refreshed {
                if incremental {
                    println!(
                        "hash statistics refreshed incrementally ({pending} hashes recounted)"
                    );
                } else {
                    println!("hash statistics recounted from scratch");
                }
            }
            match &hashes.updated_at {
                Some(at) => println!(
                    "hash statistics: {at}{}; {} exact hashes and {} fingerprints in 2+ packages",
                    if hashes.stale {
                        " (stale: run with --refresh)"
                    } else {
                        ""
                    },
                    hashes.common_exact_hashes,
                    hashes.common_fingerprints
                ),
                None => println!("hash statistics: never computed (run with --refresh)"),
            }
            print_common("exact hashes", &exact);
            print_common("fingerprints", &fingerprints);
        }
    }
    Ok(())
}

/// Prints the most common hashes of one kind, with a sample function each.
fn print_common(title: &str, hashes: &[CommonHash]) {
    if hashes.is_empty() {
        return;
    }
    println!("\nmost common {title} ({}):", hashes[0].level);
    for h in hashes {
        println!(
            "  {:>6} packages {:>7} functions  {}  {}",
            h.package_count,
            h.function_count,
            &h.hash[..h.hash.len().min(16)],
            h.sample.as_deref().unwrap_or("")
        );
    }
}
