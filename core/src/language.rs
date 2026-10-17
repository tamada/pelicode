//! Language adapters: the per-language rules for function extraction and normalization.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A programming language supported by pelicode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Go,
    Java,
    Kotlin,
    Python,
    Ruby,
    Rust,
}

impl Language {
    pub const ALL: [Language; 6] = [
        Language::Go,
        Language::Java,
        Language::Kotlin,
        Language::Python,
        Language::Ruby,
        Language::Rust,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Language::Go => "go",
            Language::Java => "java",
            Language::Kotlin => "kotlin",
            Language::Python => "python",
            Language::Ruby => "ruby",
            Language::Rust => "rust",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Language::Go => &["go"],
            Language::Java => &["java"],
            Language::Kotlin => &["kt", "kts"],
            Language::Python => &["py"],
            Language::Ruby => &["rb"],
            Language::Rust => &["rs"],
        }
    }

    pub fn from_extension(ext: &str) -> Option<Language> {
        Language::ALL
            .into_iter()
            .find(|lang| lang.extensions().contains(&ext))
    }

    pub fn from_path(path: &Path) -> Option<Language> {
        path.extension()
            .and_then(|ext| ext.to_str())
            .and_then(Language::from_extension)
    }

    pub fn ts_language(self) -> tree_sitter::Language {
        match self {
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }

    pub(crate) fn spec(self) -> &'static LanguageSpec {
        match self {
            Language::Go => &GO,
            Language::Java => &JAVA,
            Language::Kotlin => &KOTLIN,
            Language::Python => &PYTHON,
            Language::Ruby => &RUBY,
            Language::Rust => &RUST,
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Language {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Language::ALL
            .into_iter()
            .find(|lang| lang.name().eq_ignore_ascii_case(s))
            .ok_or_else(|| format!("unsupported language: {s}"))
    }
}

/// Node kinds that drive extraction and normalization for one language.
pub(crate) struct LanguageSpec {
    /// Nodes treated as a function unit.
    pub functions: &'static [&'static str],
    /// Comment nodes (dropped from every level).
    pub comments: &'static [&'static str],
    /// Variable-like identifiers, abstracted to `$ID` from N1.
    pub identifiers: &'static [&'static str],
    /// Type-like identifiers, kept at N1 and abstracted to `$TYPE` from N2.
    pub type_identifiers: &'static [&'static str],
    /// Ancestors that turn a plain identifier into a type identifier (e.g. Python's `type`).
    pub type_contexts: &'static [&'static str],
    /// String-like literals, emitted as one atomic token (`$STR` from N1).
    pub strings: &'static [&'static str],
    /// Numeric literals (`$NUM` from N1).
    pub numbers: &'static [&'static str],
    /// Whether the first string statement of a function body is a docstring to drop.
    pub docstrings: bool,
    /// Whether `;` is an optional statement terminator (layout, not structure) and is dropped.
    pub optional_semicolons: bool,
}

static GO: LanguageSpec = LanguageSpec {
    functions: &["function_declaration", "method_declaration", "func_literal"],
    comments: &["comment"],
    identifiers: &["identifier", "field_identifier", "label_name"],
    type_identifiers: &["type_identifier", "package_identifier"],
    type_contexts: &[],
    strings: &[
        "interpreted_string_literal",
        "raw_string_literal",
        "rune_literal",
    ],
    numbers: &["int_literal", "float_literal", "imaginary_literal"],
    docstrings: false,
    optional_semicolons: true,
};

static JAVA: LanguageSpec = LanguageSpec {
    functions: &[
        "method_declaration",
        "constructor_declaration",
        "compact_constructor_declaration",
        "lambda_expression",
    ],
    comments: &["line_comment", "block_comment"],
    identifiers: &["identifier"],
    type_identifiers: &["type_identifier"],
    type_contexts: &[],
    strings: &["string_literal", "character_literal"],
    numbers: &[
        "decimal_integer_literal",
        "hex_integer_literal",
        "octal_integer_literal",
        "binary_integer_literal",
        "decimal_floating_point_literal",
        "hex_floating_point_literal",
    ],
    docstrings: false,
    optional_semicolons: false,
};

static KOTLIN: LanguageSpec = LanguageSpec {
    functions: &[
        "function_declaration",
        "secondary_constructor",
        "anonymous_function",
        "lambda_literal",
        "getter",
        "setter",
    ],
    comments: &["line_comment", "block_comment"],
    identifiers: &["identifier"],
    type_identifiers: &[],
    type_contexts: &["user_type"],
    strings: &[
        "string_literal",
        "multiline_string_literal",
        "character_literal",
    ],
    numbers: &["number_literal", "float_literal"],
    docstrings: false,
    optional_semicolons: true,
};

static PYTHON: LanguageSpec = LanguageSpec {
    functions: &["function_definition", "lambda"],
    comments: &["comment"],
    identifiers: &["identifier"],
    type_identifiers: &[],
    type_contexts: &["type"],
    strings: &["string", "concatenated_string"],
    numbers: &["integer", "float"],
    docstrings: true,
    optional_semicolons: true,
};

static RUBY: LanguageSpec = LanguageSpec {
    functions: &["method", "singleton_method", "lambda", "block", "do_block"],
    comments: &["comment"],
    identifiers: &[
        "identifier",
        "instance_variable",
        "class_variable",
        "global_variable",
    ],
    type_identifiers: &["constant"],
    type_contexts: &[],
    strings: &[
        "string",
        "chained_string",
        "character",
        "simple_symbol",
        "delimited_symbol",
        "regex",
        "heredoc_body",
        "string_array",
        "symbol_array",
    ],
    numbers: &["integer", "float", "rational", "complex"],
    docstrings: false,
    optional_semicolons: true,
};

static RUST: LanguageSpec = LanguageSpec {
    // Trait methods without a body (`function_signature_item`) are not functions.
    functions: &["function_item", "closure_expression"],
    comments: &["line_comment", "block_comment"],
    identifiers: &[
        "identifier",
        "field_identifier",
        "shorthand_field_identifier",
    ],
    type_identifiers: &["type_identifier"],
    type_contexts: &[],
    strings: &["string_literal", "raw_string_literal", "char_literal"],
    numbers: &["integer_literal", "float_literal"],
    docstrings: false,
    // `;` turns an expression into a statement, so it is structure.
    optional_semicolons: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_language_from_extension() {
        assert_eq!(
            Language::from_path(Path::new("a/b/Main.kt")),
            Some(Language::Kotlin)
        );
        assert_eq!(
            Language::from_path(Path::new("x.py")),
            Some(Language::Python)
        );
        assert_eq!(Language::from_path(Path::new("x.rs")), Some(Language::Rust));
        assert_eq!(Language::from_path(Path::new("x.c")), None);
    }

    #[test]
    fn grammars_load() {
        for lang in Language::ALL {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&lang.ts_language()).unwrap();
        }
    }
}
