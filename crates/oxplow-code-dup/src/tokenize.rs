//! Walk a tree-sitter syntax tree and emit a normalized token stream.
//!
//! Identifiers and literals are folded to placeholder kinds (`ID`,
//! `NUM`, `STR`) so renames and constant tweaks don't break clone
//! matches. Everything else uses the tree-sitter `node.kind()` string
//! as the token name. Comments are skipped entirely.

use tree_sitter::Node;

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
}

pub fn tokenize_source(root: Node<'_>) -> Vec<Token> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    walk(&mut cursor, &mut out);
    out
}

fn walk(cursor: &mut tree_sitter::TreeCursor<'_>, out: &mut Vec<Token>) {
    let node = cursor.node();
    let kind = node.kind();
    // Skip comment nodes whatever their grammar names them.
    if kind.contains("comment") {
        return;
    }
    if node.child_count() == 0 {
        // Leaf node — emit one normalized token.
        let normalized = normalize_kind(kind).to_string();
        out.push(Token {
            kind: normalized,
            start_line: node.start_position().row as u32 + 1,
            end_line: node.end_position().row as u32 + 1,
        });
        return;
    }
    if cursor.goto_first_child() {
        loop {
            walk(cursor, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn normalize_kind(kind: &str) -> &str {
    // Coarse buckets — enough to trip on renames + literal swaps but
    // keep structural punctuation distinct.
    if kind == "identifier"
        || kind == "type_identifier"
        || kind == "field_identifier"
        || kind == "property_identifier"
        || kind == "shorthand_property_identifier"
        || kind == "shorthand_property_identifier_pattern"
        || kind == "primitive_type"
    {
        return "ID";
    }
    if kind.contains("integer")
        || kind.contains("float")
        || kind == "number"
        || kind == "decimal_integer_literal"
        || kind == "hex_integer_literal"
    {
        return "NUM";
    }
    if kind.contains("string") || kind == "raw_string_literal" || kind == "char_literal" {
        return "STR";
    }
    kind
}
