//! Normalization of a function subtree into token sequences per level.

use std::collections::{BTreeMap, HashMap};

use tree_sitter::Node;

use crate::config::{IdentMode, Level};
use crate::language::LanguageSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Plain,
    Ident,
    Type,
    Str,
    Num,
    Bool,
}

struct Token<'a> {
    class: Class,
    text: &'a str,
    rows: (usize, usize),
}

/// Normalized representations of one function.
pub(crate) struct Normalized {
    pub tokens: BTreeMap<Level, Vec<String>>,
    /// 0-based first and last source rows of each token, aligned with `tokens`.
    pub rows: BTreeMap<Level, Vec<(usize, usize)>>,
    pub node_count: usize,
    pub depth: usize,
}

pub(crate) fn normalize(
    root: Node<'_>,
    source: &[u8],
    spec: &'static LanguageSpec,
    ident_mode: IdentMode,
) -> Normalized {
    let mut walker = Walker {
        source,
        spec,
        root,
        leaves: Vec::new(),
        kinds: Vec::new(),
        depth: 0,
    };
    walker.visit(root, 1, false);

    let mut tokens = BTreeMap::new();
    tokens.insert(
        Level::N0,
        walker.leaves.iter().map(|t| t.text.to_string()).collect(),
    );
    tokens.insert(Level::N1, render(&walker.leaves, ident_mode, false));
    tokens.insert(Level::N2, render(&walker.leaves, ident_mode, true));
    tokens.insert(
        Level::N3,
        walker.kinds.iter().map(|(k, _)| k.to_string()).collect(),
    );

    let leaf_rows: Vec<_> = walker.leaves.iter().map(|t| t.rows).collect();
    let mut rows = BTreeMap::new();
    rows.insert(Level::N0, leaf_rows.clone());
    rows.insert(Level::N1, leaf_rows.clone());
    rows.insert(Level::N2, leaf_rows);
    // A node kind stands at the row where the node starts.
    rows.insert(
        Level::N3,
        walker.kinds.iter().map(|(_, row)| (*row, *row)).collect(),
    );
    Normalized {
        tokens,
        rows,
        node_count: walker.kinds.len(),
        depth: walker.depth,
    }
}

fn render(leaves: &[Token<'_>], ident_mode: IdentMode, abstract_types: bool) -> Vec<String> {
    let mut ids: HashMap<&str, usize> = HashMap::new();
    leaves
        .iter()
        .map(|t| match t.class {
            Class::Plain => t.text.to_string(),
            Class::Type if !abstract_types => t.text.to_string(),
            Class::Type => "$TYPE".to_string(),
            Class::Str => "$STR".to_string(),
            Class::Num => "$NUM".to_string(),
            Class::Bool => "$BOOL".to_string(),
            Class::Ident => match ident_mode {
                IdentMode::Uniform => "$ID".to_string(),
                IdentMode::Indexed => {
                    let next = ids.len() + 1;
                    format!("$ID{}", ids.entry(t.text).or_insert(next))
                }
            },
        })
        .collect()
}

struct Walker<'a, 't> {
    source: &'a [u8],
    spec: &'static LanguageSpec,
    root: Node<'t>,
    leaves: Vec<Token<'a>>,
    kinds: Vec<(&'t str, usize)>,
    depth: usize,
}

impl<'a, 't> Walker<'a, 't> {
    fn visit(&mut self, node: Node<'t>, depth: usize, in_type: bool) {
        let kind = node.kind();
        if self.spec.comments.contains(&kind) || self.is_docstring(node) {
            return;
        }
        if node.is_named() {
            self.kinds.push((kind, node.start_position().row));
            self.depth = self.depth.max(depth);
        }
        let text = || node.utf8_text(self.source).unwrap_or("");
        let rows = (node.start_position().row, node.end_position().row);
        if self.spec.strings.contains(&kind) {
            self.leaves.push(Token {
                class: Class::Str,
                text: text(),
                rows,
            });
            return;
        }
        let in_type = in_type || self.spec.type_contexts.contains(&kind);
        if node.child_count() == 0 {
            let text = text();
            if text.trim().is_empty() || (self.spec.optional_semicolons && text == ";") {
                return;
            }
            let class = if self.spec.numbers.contains(&kind) {
                Class::Num
            } else if kind == "true" || kind == "false" {
                Class::Bool
            } else if self.spec.type_identifiers.contains(&kind) {
                Class::Type
            } else if self.spec.identifiers.contains(&kind) {
                if in_type { Class::Type } else { Class::Ident }
            } else {
                Class::Plain
            };
            self.leaves.push(Token { class, text, rows });
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child, depth + 1, in_type);
        }
    }

    /// The first string statement directly in the function body.
    fn is_docstring(&self, node: Node<'t>) -> bool {
        if !self.spec.docstrings || node.kind() != "expression_statement" {
            return false;
        }
        let Some(block) = node.parent() else {
            return false;
        };
        if block.parent() != Some(self.root) || block.named_child(0) != Some(node) {
            return false;
        }
        node.named_child_count() == 1
            && node
                .named_child(0)
                .is_some_and(|child| self.spec.strings.contains(&child.kind()))
    }
}
