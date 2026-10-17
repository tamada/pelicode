//! Text, JSON and CSV renderings of extracted and stored functions.

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::Result;
use pelicode_core::{FileFeatures, Hash128, Level};
use pelicode_db::FunctionRow;
use serde::Serialize;

/// Writes `value` as pretty-printed JSON followed by a newline.
pub fn json<T: Serialize + ?Sized>(out: &mut dyn Write, value: &T) -> Result<()> {
    serde_json::to_writer_pretty(&mut *out, value)?;
    writeln!(out)?;
    Ok(())
}

/// The first 16 hexadecimal digits of a hash, for text output.
fn short(hash: &str) -> &str {
    &hash[..hash.len().min(16)]
}

/// Text output of `extract`: one line per function, and its tokens of `tokens` when given.
pub fn extract_text(
    out: &mut dyn Write,
    files: &[FileFeatures],
    tokens: Option<Level>,
) -> Result<()> {
    for file in files {
        for f in &file.functions {
            writeln!(
                out,
                "{}:{}-{}\t{}\t{}\t{}\ttokens={}\tN1={}{}",
                file.path.display(),
                f.start_line,
                f.end_line,
                file.language,
                f.name,
                f.kind,
                f.token_count,
                short(&f.exact_hashes[&Level::N1].to_hex()),
                if f.is_indexed() { "" } else { "\t(too small)" },
            )?;
            if let (Some(level), Some(all)) = (tokens, &f.tokens) {
                writeln!(out, "    {level}: {}", all[&level].join(" "))?;
            }
        }
    }
    Ok(())
}

/// Text output of `export`: one line per stored function occurrence.
pub fn export_text(out: &mut dyn Write, rows: &[FunctionRow]) -> Result<()> {
    for r in rows {
        writeln!(
            out,
            "{}:{}@{}\t{}:{}-{}\t{}\t{}\ttokens={}\tN1={}",
            r.ecosystem,
            r.package,
            r.version,
            r.path,
            r.start_line,
            r.end_line,
            r.language,
            r.name,
            r.token_count,
            r.exact_hashes.get(&Level::N1).map_or("", |h| short(h)),
        )?;
    }
    Ok(())
}

#[derive(Serialize)]
struct ExtractRecord<'a> {
    path: String,
    language: &'a str,
    name: &'a str,
    kind: &'a str,
    start_line: usize,
    end_line: usize,
    start_column: usize,
    end_column: usize,
    start_byte: usize,
    end_byte: usize,
    parent: Option<usize>,
    token_count: usize,
    node_count: usize,
    depth: usize,
    indexed: bool,
    n0: String,
    n1: String,
    n2: String,
    n3: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens: Option<String>,
}

/// CSV output of `extract`: one row per function, with the tokens of `tokens` when given.
pub fn extract_csv(
    out: &mut dyn Write,
    files: &[FileFeatures],
    tokens: Option<Level>,
) -> Result<()> {
    let mut writer = csv::Writer::from_writer(out);
    for file in files {
        let path = file.path.to_string_lossy().into_owned();
        for f in &file.functions {
            let hex = |level| {
                f.exact_hashes
                    .get(&level)
                    .map(Hash128::to_hex)
                    .unwrap_or_default()
            };
            writer.serialize(ExtractRecord {
                path: path.clone(),
                language: file.language.name(),
                name: &f.name,
                kind: &f.kind,
                start_line: f.start_line,
                end_line: f.end_line,
                start_column: f.start_column,
                end_column: f.end_column,
                start_byte: f.start_byte,
                end_byte: f.end_byte,
                parent: f.parent,
                token_count: f.token_count,
                node_count: f.node_count,
                depth: f.depth,
                indexed: f.is_indexed(),
                n0: hex(Level::N0),
                n1: hex(Level::N1),
                n2: hex(Level::N2),
                n3: hex(Level::N3),
                tokens: tokens.and_then(|level| Some(f.tokens.as_ref()?[&level].join(" "))),
            })?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[derive(Serialize)]
struct ExportRecord<'a> {
    id: i64,
    ecosystem: &'a str,
    package: &'a str,
    version: &'a str,
    license: Option<&'a str>,
    path: &'a str,
    language: &'a str,
    name: &'a str,
    kind: &'a str,
    start_line: usize,
    end_line: usize,
    start_column: usize,
    end_column: usize,
    start_byte: usize,
    end_byte: usize,
    parent_id: Option<i64>,
    token_count: usize,
    node_count: usize,
    depth: usize,
    indexed: bool,
    n0: &'a str,
    n1: &'a str,
    n2: &'a str,
    n3: &'a str,
}

/// CSV output of `export`: one row per stored function occurrence.
pub fn export_csv(out: &mut dyn Write, rows: &[FunctionRow]) -> Result<()> {
    let mut writer = csv::Writer::from_writer(out);
    for r in rows {
        let hex = |level| hash_of(&r.exact_hashes, level);
        writer.serialize(ExportRecord {
            id: r.id,
            ecosystem: &r.ecosystem,
            package: &r.package,
            version: &r.version,
            license: r.license.as_deref(),
            path: &r.path,
            language: &r.language,
            name: &r.name,
            kind: &r.kind,
            start_line: r.start_line,
            end_line: r.end_line,
            start_column: r.start_column,
            end_column: r.end_column,
            start_byte: r.start_byte,
            end_byte: r.end_byte,
            parent_id: r.parent_id,
            token_count: r.token_count,
            node_count: r.node_count,
            depth: r.depth,
            indexed: r.indexed,
            n0: hex(Level::N0),
            n1: hex(Level::N1),
            n2: hex(Level::N2),
            n3: hex(Level::N3),
        })?;
    }
    writer.flush()?;
    Ok(())
}

/// The hash of `level`, or an empty string.
fn hash_of(hashes: &BTreeMap<Level, String>, level: Level) -> &str {
    hashes.get(&level).map_or("", String::as_str)
}
