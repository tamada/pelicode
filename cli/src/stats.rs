//! `pelicode stats`: table sizes and the hashes shared by the most packages.

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Args;
use pelicode_core::Level;
use pelicode_db::{CommonHash, Database};

use crate::SummaryFormat;

#[derive(Args)]
pub struct StatsArgs {
    /// Database file.
    #[arg(long, default_value = "pelicode.db")]
    db: PathBuf,
    /// Rebuild the per-hash package counts first.
    #[arg(long)]
    refresh: bool,
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

pub fn run(args: StatsArgs) -> Result<()> {
    if !args.db.exists() {
        bail!("{}: no such database", args.db.display());
    }
    let mut db = Database::open_unchecked(&args.db)?;
    if args.refresh {
        db.refresh_hash_stats()?;
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
