//! SARIF 2.1.0 output of `pelicode scan`, for GitHub code scanning and other
//! tools that read static analysis results.
//!
//! Each checked function with candidates is one result, located at the
//! function, with the level `note`: a candidate is something to review, not a
//! finding of a defect.

use std::io::Write;

use anyhow::Result;
use pelicode_detect::{Candidate, MatchKind, Report};
use serde_json::{Value, json};

const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const INFORMATION_URI: &str = "https://tamada.github.io/pelicode";
const HELP_URI: &str = "https://tamada.github.io/pelicode/usage/#scan";
const EXACT: &str = "exact-match";
const SIMILAR: &str = "similar-match";
const ADVICE: &str = "Review whether the code was copied, and whether the license of its source \
    requires attribution or prevents the reuse.";

/// Writes `reports` as a SARIF log. `uri` turns the path of a report into the
/// URI of the file relative to the root of the repository.
pub fn write(
    out: &mut dyn Write,
    reports: &[Report],
    uri: impl Fn(&Report) -> String,
) -> Result<()> {
    let results: Vec<Value> = reports.iter().map(|r| result(r, &uri(r))).collect();
    let log = json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "pelicode",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": INFORMATION_URI,
                    "rules": [rule(EXACT), rule(SIMILAR)],
                }
            },
            "results": results,
        }]
    });
    serde_json::to_writer_pretty(&mut *out, &log)?;
    writeln!(out)?;
    Ok(())
}

fn rule(id: &str) -> Value {
    let (name, short, full) = match id {
        EXACT => (
            "ExactMatch",
            "Function identical to published code after normalization",
            "The function is identical to a function of a published package once comments, \
             layout and, depending on the level, the names of identifiers and types are set aside.",
        ),
        _ => (
            "SimilarMatch",
            "Function similar to published code",
            "The function shares many fingerprints (hashes of token sequences) with a function \
             of a published package.",
        ),
    };
    json!({
        "id": id,
        "name": name,
        "shortDescription": { "text": short },
        "fullDescription": { "text": full },
        "help": {
            "text": format!("{full} {ADVICE}"),
            "markdown": format!("{full}\n\n{ADVICE} pelicode does not decide whether code was \
                plagiarized; see [pelicode scan]({HELP_URI})."),
        },
        "helpUri": HELP_URI,
        "defaultConfiguration": { "level": "note" },
        "properties": { "tags": ["provenance", "license"] },
    })
}

fn result(report: &Report, uri: &str) -> Value {
    let best = &report.candidates[0];
    let rule_id = match best.kind {
        MatchKind::Exact(_) => EXACT,
        MatchKind::Similar(_) => SIMILAR,
    };
    let mut message = format!("`{}` {}. {ADVICE}", report.name, describe(best));
    if report.candidates.len() > 1 {
        let others: Vec<String> = report.candidates[1..].iter().map(source).collect();
        message.push_str(&format!(" Other candidates: {}.", others.join("; ")));
    }
    json!({
        "ruleId": rule_id,
        "level": "note",
        "message": { "text": message },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": { "uri": uri },
                "region": {
                    "startLine": report.start_line,
                    "endLine": report.end_line,
                },
            }
        }],
        "partialFingerprints": {
            "pelicodeCandidate/v1": fingerprint(uri, report, best),
        },
        "properties": {
            "function": report.name,
            "tokens": report.token_count,
            "candidates": report.candidates.iter().map(candidate).collect::<Vec<_>>(),
        },
    })
}

/// `is similar (0.72) to super_len in pypi:requests@2.34.2 src/requests/utils.py:160-228
/// (Apache-2.0); lines 9-23 match lines 164-189`
fn describe(c: &Candidate) -> String {
    let relation = match c.kind {
        MatchKind::Exact(level) => format!("is identical after {level} normalization to"),
        MatchKind::Similar(_) => format!("is similar ({:.2}) to", c.similarity),
    };
    let mut text = format!("{relation} {}", source(c));
    if let MatchKind::Similar(_) = c.kind {
        text.push_str(&format!(
            "; lines {} match lines {}",
            ranges(&c.matched_lines),
            ranges(&c.candidate_lines)
        ));
    }
    text
}

/// `super_len in pypi:requests@2.34.2 src/requests/utils.py:160-228 (Apache-2.0)`
fn source(c: &Candidate) -> String {
    let location = match c.occurrences.first() {
        Some(o) => format!(
            "{}:{}@{} {}:{}-{}",
            o.ecosystem, o.package, o.version, o.path, c.function.start_line, c.function.end_line
        ),
        None => "an unknown package".into(),
    };
    let license = c
        .occurrences
        .first()
        .and_then(|o| o.license.as_deref())
        .unwrap_or("license unknown");
    format!("`{}` in {location} ({license})", c.function.name)
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
        .join(", ")
}

fn candidate(c: &Candidate) -> Value {
    let o = c.occurrences.first();
    json!({
        "match": match c.kind { MatchKind::Exact(_) => "exact", MatchKind::Similar(_) => "similar" },
        "level": match c.kind { MatchKind::Exact(l) | MatchKind::Similar(l) => l.to_string() },
        "similarity": c.similarity,
        "containment": c.containment,
        "ecosystem": o.map(|o| o.ecosystem.as_str()),
        "package": o.map(|o| o.package.as_str()),
        "version": o.map(|o| o.version.as_str()),
        "path": o.map(|o| o.path.as_str()),
        "license": o.and_then(|o| o.license.as_deref()),
        "repository": o.and_then(|o| o.repository_url.as_deref()),
        "function": c.function.name,
        "startLine": c.function.start_line,
        "endLine": c.function.end_line,
        "occurrences": c.occurrence_count,
    })
}

/// A key that stays the same while the function and its best candidate do,
/// even when lines move, so that code scanning tracks one alert across runs.
fn fingerprint(uri: &str, report: &Report, best: &Candidate) -> String {
    let source = best
        .occurrences
        .first()
        .map(|o| format!("{}:{}:{}", o.ecosystem, o.package, o.path))
        .unwrap_or_default();
    let key = format!(
        "{uri}\u{1f}{}\u{1f}{source}\u{1f}{}",
        report.name, best.function.name
    );
    pelicode_core::extract::sha256_hex(key.as_bytes())[..32].to_string()
}
