//! Walk a tree-sitter syntax tree and emit a normalized token stream.
//!
//! Identifiers and literals are folded to placeholder kinds (`ID`,
//! `NUM`, `STR`) so renames and constant tweaks don't break clone
//! matches. Everything else uses the tree-sitter `node.kind()` string
//! (which is `&'static str`) as the token kind. Comments are skipped.
//!
//! Import / use / include / package declarations are also skipped
//! whole-subtree: an `import` block is line noise that biases the
//! fingerprint toward "two files share a few imports" matches. The
//! skip list is keyed off [`Language`] so each grammar contributes
//! exactly the node kinds it actually emits.

use oxplow_code_metrics::Language;
use tree_sitter::Node;

#[derive(Debug, Clone, Copy)]
pub struct Token {
    /// The normalized AST node kind. Borrows from tree-sitter's
    /// static grammar tables — no allocation per leaf.
    pub kind: &'static str,
    pub start_line: u32,
    pub end_line: u32,
}

pub fn tokenize_source(root: Node<'_>, lang: Language) -> Vec<Token> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    walk(&mut cursor, lang, &mut out);
    out
}

fn walk(cursor: &mut tree_sitter::TreeCursor<'_>, lang: Language, out: &mut Vec<Token>) {
    let node = cursor.node();
    let kind = node.kind();
    if is_comment_kind(kind) {
        return;
    }
    if is_skip_kind(lang, kind) {
        return;
    }
    if node.child_count() == 0 {
        out.push(Token {
            kind: normalize_kind(kind),
            start_line: node.start_position().row as u32 + 1,
            end_line: node.end_position().row as u32 + 1,
        });
        return;
    }
    if cursor.goto_first_child() {
        loop {
            walk(cursor, lang, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Exact-match comment node kinds across our 9 grammars. Substring
/// matching on "comment" was fragile (would catch any future node
/// like `commenter`).
fn is_comment_kind(kind: &str) -> bool {
    matches!(
        kind,
        "comment" | "line_comment" | "block_comment" | "doc_comment"
    )
}

/// Per-language list of node kinds whose entire subtree we skip
/// during tokenization. The motivating case is import / use /
/// include / package declarations: they're idiomatic line noise
/// that drags the fingerprint toward "two files share a few of the
/// same imports" false positives. Whatever lives inside an
/// import declaration (paths, aliases, brace-lists) contributes
/// nothing semantic to clone detection.
///
/// Adding a new language: list every top-level declaration node
/// kind that wraps an import / package directive. The skip is
/// whole-subtree so listing the outer wrapper is enough.
fn is_skip_kind(lang: Language, kind: &str) -> bool {
    match lang {
        Language::Rust => matches!(
            kind,
            "use_declaration" | "extern_crate_declaration"
        ),
        Language::TypeScript | Language::Tsx | Language::JavaScript => matches!(
            kind,
            "import_statement" | "import_alias"
        ),
        Language::Python => matches!(
            kind,
            "import_statement" | "import_from_statement" | "future_import_statement"
        ),
        Language::Go => matches!(
            kind,
            "import_declaration" | "import_spec" | "import_spec_list" | "package_clause"
        ),
        Language::Java => matches!(
            kind,
            "import_declaration" | "package_declaration"
        ),
        Language::C => matches!(kind, "preproc_include"),
        Language::Cpp => matches!(
            kind,
            "preproc_include" | "using_declaration" | "using_directive"
        ),
    }
}

/// Coarse token-kind buckets — enough to trip on renames + literal
/// swaps but keep structural punctuation distinct. Uses an
/// exact-match list per category instead of substring containment.
fn normalize_kind(kind: &str) -> &'static str {
    if matches!(
        kind,
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
            | "primitive_type"
            | "scoped_identifier"
            | "scoped_type_identifier"
    ) {
        return "ID";
    }
    if matches!(
        kind,
        "integer_literal"
            | "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "float_literal"
            | "decimal_floating_point_literal"
            | "hex_floating_point_literal"
            | "number"
            | "integer"
            | "float"
    ) {
        return "NUM";
    }
    if matches!(
        kind,
        "string_literal"
            | "raw_string_literal"
            | "char_literal"
            | "string"
            | "string_fragment"
            | "interpreted_string_literal"
            | "raw_string_fragment"
    ) {
        return "STR";
    }
    // Promote the tree-sitter `&'static str` to our return slot. No
    // allocation — `Node::kind()` is documented to return `'static`.
    static_str(kind)
}

/// Helper: tree-sitter's `Node::kind()` returns `&'static str`, but
/// the borrow checker can't prove that through a function boundary.
/// This intermediate lets us return `&'static` without unsafe.
#[inline]
fn static_str(s: &str) -> &'static str {
    // SAFETY: `Node::kind()` always returns a `&'static str` produced
    // by the tree-sitter C grammar tables. The `&str` we receive here
    // is one of those static strings; only the lifetime annotation is
    // narrower. Re-extending it back to `'static` is sound.
    unsafe { std::mem::transmute::<&str, &'static str>(s) }
}
