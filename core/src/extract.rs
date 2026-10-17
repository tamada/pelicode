//! Function extraction and feature computation for a single source file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};

use crate::Error;
use crate::config::{Config, Level};
use crate::hash::{self, Fingerprint, Hash128};
use crate::language::{Language, LanguageSpec};
use crate::normalize::normalize;

/// Features of one source file.
#[derive(Debug, Clone, Serialize)]
pub struct FileFeatures {
    pub path: PathBuf,
    pub language: Language,
    /// SHA-256 of the file content (hex).
    pub content_sha256: String,
    pub functions: Vec<FunctionFeatures>,
}

/// Features of one function unit.
#[derive(Debug, Clone, Serialize)]
pub struct FunctionFeatures {
    /// Function name for display; `<anonymous>` for lambdas and blocks. Never used for matching.
    pub name: String,
    /// Tree-sitter node kind of the function.
    pub kind: String,
    /// 1-based, inclusive.
    pub start_line: usize,
    pub end_line: usize,
    /// 1-based byte columns of the first and last character.
    pub start_column: usize,
    pub end_column: usize,
    /// Byte range in the file (0-based, end exclusive).
    pub start_byte: usize,
    pub end_byte: usize,
    /// Index of the enclosing function in the same file.
    pub parent: Option<usize>,
    /// Number of N0 tokens.
    pub token_count: usize,
    /// Number of named AST nodes (comments excluded).
    pub node_count: usize,
    pub depth: usize,
    pub exact_hashes: BTreeMap<Level, Hash128>,
    /// Present only for functions with at least `min_tokens` tokens.
    pub features: BTreeMap<Level, LevelFeatures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<BTreeMap<Level, Vec<String>>>,
}

impl FunctionFeatures {
    /// Whether this function is large enough to carry fingerprints.
    pub fn is_indexed(&self) -> bool {
        !self.features.is_empty()
    }
}

/// Partial-match features of one normalization level.
#[derive(Debug, Clone, Serialize)]
pub struct LevelFeatures {
    pub fingerprints: Vec<Fingerprint>,
    #[serde(serialize_with = "hash::serialize_u64_hex_vec")]
    pub minhash: Vec<u64>,
    #[serde(serialize_with = "hash::serialize_u64_hex_vec")]
    pub lsh_buckets: Vec<u64>,
}

pub fn sha256_hex(content: &[u8]) -> String {
    hash::to_hex(&Sha256::digest(content))
}

/// Reads and extracts a file; `display_path` is the path recorded in the result.
pub fn extract_file(
    path: &Path,
    display_path: &Path,
    language: Language,
    config: &Config,
) -> Result<FileFeatures, Error> {
    let content = std::fs::read(path).map_err(|e| Error::Io(path.to_path_buf(), e))?;
    extract_bytes(display_path, language, &content, config)
}

/// Extracts a file whose content is already in memory (e.g. read from a git revision).
pub fn extract_bytes(
    path: &Path,
    language: Language,
    content: &[u8],
    config: &Config,
) -> Result<FileFeatures, Error> {
    Ok(FileFeatures {
        path: path.to_path_buf(),
        language,
        content_sha256: sha256_hex(content),
        functions: extract_source(language, content, config)?,
    })
}

/// Extracts the features of every function in `source`.
pub fn extract_source(
    language: Language,
    source: &[u8],
    config: &Config,
) -> Result<Vec<FunctionFeatures>, Error> {
    config.validate()?;
    let mut parser = Parser::new();
    parser
        .set_language(&language.ts_language())
        .map_err(|e| Error::Parse(e.to_string()))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::Parse("parser returned no tree".into()))?;

    let spec = language.spec();
    let mut functions = Vec::new();
    collect(
        tree.root_node(),
        None,
        None,
        source,
        spec,
        config,
        &mut functions,
    );
    Ok(functions)
}

fn collect(
    node: Node<'_>,
    parent_kind: Option<&str>,
    parent_index: Option<usize>,
    source: &[u8],
    spec: &'static LanguageSpec,
    config: &Config,
    out: &mut Vec<FunctionFeatures>,
) {
    let kind = node.kind();
    let mut current = parent_index;
    // A function directly wrapping another (e.g. Ruby `-> { }` with its block) is one unit.
    let wrapped = parent_kind.is_some_and(|p| spec.functions.contains(&p));
    if spec.functions.contains(&kind) && !wrapped && !node.has_error() {
        out.push(features(node, parent_index, source, spec, config));
        current = Some(out.len() - 1);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, Some(kind), current, source, spec, config, out);
    }
}

fn features(
    node: Node<'_>,
    parent: Option<usize>,
    source: &[u8],
    spec: &'static LanguageSpec,
    config: &Config,
) -> FunctionFeatures {
    let normalized = normalize(node, source, spec, config.ident_mode);
    let token_count = normalized.tokens[&Level::N0].len();
    let exact_hashes = normalized
        .tokens
        .iter()
        .map(|(level, tokens)| (*level, Hash128::of_tokens(tokens)))
        .collect();

    let mut features = BTreeMap::new();
    if token_count >= config.min_tokens {
        for level in &config.fingerprint_levels {
            let tokens = &normalized.tokens[level];
            let rows = &normalized.rows[level];
            let k = config.kgram.min(tokens.len());
            let fingerprints: Vec<Fingerprint> =
                hash::winnow(&hash::kgram_hashes(tokens, config.kgram), config.window)
                    .into_iter()
                    .map(|(position, hash)| Fingerprint {
                        hash,
                        position: position as u32,
                        start_line: rows[position].0 as u32 + 1,
                        end_line: rows[position + k - 1].1 as u32 + 1,
                    })
                    .collect();
            let Some(minhash) =
                hash::minhash(fingerprints.iter().map(|f| f.hash), config.minhash_size)
            else {
                continue;
            };
            let lsh_buckets = hash::lsh_buckets(&minhash, config.bands);
            features.insert(
                *level,
                LevelFeatures {
                    fingerprints,
                    minhash,
                    lsh_buckets,
                },
            );
        }
    }

    FunctionFeatures {
        name: function_name(node, source),
        kind: node.kind().to_string(),
        start_line: node.start_position().row + 1,
        end_line: node.end_position().row + 1,
        start_column: node.start_position().column + 1,
        // The end position points just past the last character.
        end_column: node.end_position().column.max(1),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        parent,
        token_count,
        node_count: normalized.node_count,
        depth: normalized.depth,
        exact_hashes,
        features,
        tokens: config.keep_tokens.then_some(normalized.tokens),
    }
}

fn function_name(node: Node<'_>, source: &[u8]) -> String {
    if let Some(name) = node.child_by_field_name("name")
        && let Ok(text) = name.utf8_text(source)
    {
        return text.to_string();
    }
    match node.kind() {
        "getter" => "get".into(),
        "setter" => "set".into(),
        "secondary_constructor" => "constructor".into(),
        _ => "<anonymous>".into(),
    }
}
