//! Native code-quality metrics built directly on tree-sitter.
//!
//! Replaces the lizard subprocess with an in-process implementation
//! that walks the AST and emits per-function metrics:
//!
//! - **complexity** — cyclomatic complexity number (CCN). Counts
//!   decision-point nodes inside each function body.
//! - **function-length** — line count of the function body.
//! - **parameter-count** — number of declared parameters.
//!
//! Languages bundled: Rust, TypeScript, TSX, JavaScript, Python, Go,
//! Java, C, C++. Adding a new language is one entry in `Language`
//! plus a `LanguageSpec` with the relevant tree-sitter node names.
//!
//! Files in unsupported languages are silently skipped (the caller
//! sees no findings for them; behaviour matches lizard).

use std::path::Path;

use tree_sitter::{Node, Parser};

mod spec;

pub use spec::{Language, LanguageSpec, language_for_path};

/// One per-function metric record. The caller is responsible for
/// fanning this out into the three downstream finding kinds
/// (complexity / function-length / parameter-count).
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionMetrics {
    /// Repo-relative file path (caller-supplied; pass-through).
    pub path: String,
    /// Best-effort function name. May include receiver/class qualifier.
    pub name: String,
    /// Cyclomatic complexity (decision points + 1).
    pub complexity: u32,
    /// Body line count (end_line - start_line + 1).
    pub length: u32,
    /// Declared parameter count.
    pub parameter_count: u32,
    /// 1-based start line.
    pub start_line: u32,
    /// 1-based end line.
    pub end_line: u32,
}

/// Analyze a single file. Returns an empty Vec for unsupported
/// languages (or files that fail to parse).
pub fn analyze_file(path: &str, source: &str) -> Vec<FunctionMetrics> {
    let Some(lang) = language_for_path(path) else {
        return Vec::new();
    };
    analyze_with_language(path, source, lang)
}

/// Like `analyze_file` but with the language explicitly chosen
/// (useful when the path doesn't reveal the language, e.g. content
/// fetched from a git ref into a temp buffer).
pub fn analyze_with_language(
    path: &str,
    source: &str,
    language: Language,
) -> Vec<FunctionMetrics> {
    let spec = language.spec();
    let mut parser = Parser::new();
    if parser.set_language(&spec.tree_sitter_language()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk_functions(tree.root_node(), source.as_bytes(), spec, path, &mut out);
    out
}

/// Recursive function-finder. When we hit a node whose kind matches
/// one of `spec.function_kinds`, compute its metrics and DO NOT
/// recurse further — nested functions are reported as their own
/// records by the second pass below.
fn walk_functions(
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    path: &str,
    out: &mut Vec<FunctionMetrics>,
) {
    let kind = node.kind();
    if spec.function_kinds.contains(&kind) {
        if let Some(rec) = function_metrics(node, src, spec, path) {
            out.push(rec);
        }
        // Still descend so we capture nested closures / methods.
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_functions(child, src, spec, path, out);
    }
}

fn function_metrics(
    node: Node<'_>,
    src: &[u8],
    spec: &LanguageSpec,
    path: &str,
) -> Option<FunctionMetrics> {
    let start_line = node.start_position().row as u32 + 1;
    let end_line = node.end_position().row as u32 + 1;
    let length = end_line.saturating_sub(start_line) + 1;

    let name = function_name(node, src, spec).unwrap_or_else(|| "(anonymous)".into());
    let parameter_count = count_parameters(node, spec);
    let complexity = count_decision_points(node, spec) + 1;

    Some(FunctionMetrics {
        path: path.into(),
        name,
        complexity,
        length,
        parameter_count,
        start_line,
        end_line,
    })
}

fn function_name(node: Node<'_>, src: &[u8], spec: &LanguageSpec) -> Option<String> {
    for field in spec.name_fields {
        if let Some(name_node) = node.child_by_field_name(field) {
            if let Ok(text) = name_node.utf8_text(src) {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn count_parameters(node: Node<'_>, spec: &LanguageSpec) -> u32 {
    let Some(params_node) = spec
        .param_list_fields
        .iter()
        .find_map(|f| node.child_by_field_name(f))
    else {
        return 0;
    };
    let mut cursor = params_node.walk();
    let mut count = 0u32;
    for child in params_node.named_children(&mut cursor) {
        if spec.parameter_kinds.contains(&child.kind()) {
            count += 1;
        }
    }
    count
}

fn count_decision_points(node: Node<'_>, spec: &LanguageSpec) -> u32 {
    let mut count = 0u32;
    let mut cursor = node.walk();
    if spec.decision_kinds.contains(&node.kind()) {
        // Don't count the function node itself.
        if cursor.node().id() != node.id() {
            count += 1;
        }
    }
    for child in node.children(&mut cursor) {
        count += count_decision_subtree(child, spec, node.id());
    }
    count
}

fn count_decision_subtree(node: Node<'_>, spec: &LanguageSpec, root_id: usize) -> u32 {
    let mut count = 0u32;
    if node.id() != root_id && spec.decision_kinds.contains(&node.kind()) {
        count += 1;
    }
    // Stop descending into nested function bodies — their decisions
    // belong to that function's own metrics record.
    if spec.function_kinds.contains(&node.kind()) {
        return count;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_subtree(child, spec, root_id);
    }
    count
}

/// Entry point used by `analyze_functions_at_refs` — analyzes a
/// (path, content) batch on a single side and returns one record
/// per detected function. Files in unsupported languages produce
/// no records.
pub fn analyze_batch<I, S>(files: I) -> Vec<FunctionMetrics>
where
    I: IntoIterator<Item = (S, S)>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (path, source) in files {
        out.extend(analyze_file(path.as_ref(), source.as_ref()));
    }
    out
}

/// Convenience: peek at a path's extension to decide whether we
/// support it. Useful for callers that want to skip read-from-disk
/// when the language isn't supported anyway.
pub fn is_supported_path(path: impl AsRef<Path>) -> bool {
    let p = path.as_ref().to_string_lossy();
    language_for_path(&p).is_some()
}

#[cfg(test)]
mod tests;
