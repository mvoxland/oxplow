//! Cross-file token-stream duplicate-block detector. The pipeline:
//!
//! 1. **Tokenize** each file by walking the tree-sitter AST and
//!    emitting a normalized token per leaf node (identifiers, numeric
//!    literals, and string literals are folded to placeholder kinds
//!    so that renames and constant changes don't suppress matches).
//! 2. **K-gram hash** over runs of `K` consecutive tokens.
//! 3. **Winnow** with window `W` to keep one fingerprint per `W`
//!    tokens — Schleimer/Aiken 2003.
//! 4. **Inverted index** fingerprint → occurrences.
//! 5. **Extend** each multi-occurrence fingerprint into the longest
//!    contiguous run of matching fingerprints between the two files.
//! 6. **Filter** runs whose line span is shorter than `min_lines`.
//!
//! Tunables are exposed on `DupOptions` (K=20, W=4, min_lines=5 by
//! default).

use std::collections::HashMap;

use oxplow_code_metrics::language_for_path;
use tree_sitter::Parser;

mod tokenize;

use tokenize::{tokenize_source, Token};

/// One duplicate pair the runner will emit as two findings (one per side).
#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateBlock {
    pub a_path: String,
    pub a_start_line: u32,
    pub a_end_line: u32,
    pub b_path: String,
    pub b_start_line: u32,
    pub b_end_line: u32,
    /// Number of lines the two regions span (max of the two).
    pub line_count: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct DupOptions {
    /// Tokens per k-gram. Smaller = catches shorter clones, more noise.
    pub k: usize,
    /// Winnowing window. Larger = fewer fingerprints, faster, less precise.
    pub w: usize,
    /// Minimum line span to report (filters out trivial echoes).
    pub min_lines: u32,
}

impl Default for DupOptions {
    fn default() -> Self {
        Self {
            k: 20,
            w: 4,
            min_lines: 5,
        }
    }
}

/// Scan a batch of (path, content) pairs for duplicate blocks.
/// Files in unsupported languages are skipped silently.
pub fn detect_duplicates<I, P, S>(files: I, opts: DupOptions) -> Vec<DuplicateBlock>
where
    I: IntoIterator<Item = (P, S)>,
    P: Into<String>,
    S: AsRef<str>,
{
    let mut docs: Vec<Doc> = Vec::new();
    for (path, source) in files {
        let path = path.into();
        let Some(lang) = language_for_path(&path) else {
            continue;
        };
        let spec = lang.spec();
        let mut parser = Parser::new();
        if parser.set_language(&spec.tree_sitter_language()).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(source.as_ref(), None) else {
            continue;
        };
        let tokens = tokenize_source(tree.root_node());
        if tokens.len() < opts.k {
            continue;
        }
        let fps = winnow(&tokens, opts.k, opts.w);
        if fps.is_empty() {
            continue;
        }
        docs.push(Doc {
            path,
            tokens,
            fps,
        });
    }
    detect_across_docs(&docs, opts)
}

struct Doc {
    path: String,
    tokens: Vec<Token>,
    fps: Vec<Fingerprint>,
}

#[derive(Clone, Copy, Debug)]
struct Fingerprint {
    hash: u64,
    /// Index into `tokens` where this fingerprint's k-gram starts.
    token_idx: usize,
}

/// Roll a 64-bit polynomial hash over tokens; emit minimum-of-window
/// fingerprints per Schleimer/Aiken winnowing.
fn winnow(tokens: &[Token], k: usize, w: usize) -> Vec<Fingerprint> {
    let n = tokens.len();
    if n < k {
        return Vec::new();
    }
    // Per-token hash: 64-bit FNV of the token kind string.
    let token_hashes: Vec<u64> = tokens.iter().map(|t| fnv1a(t.kind.as_bytes())).collect();

    // K-gram hashes: rolling polynomial over token_hashes.
    let base: u64 = 1099511628211;
    let mut k_hashes: Vec<u64> = Vec::with_capacity(n - k + 1);
    let mut acc: u64 = 0;
    let mut base_pow: u64 = 1;
    for i in 0..k {
        acc = acc.wrapping_mul(base).wrapping_add(token_hashes[i]);
        if i < k - 1 {
            base_pow = base_pow.wrapping_mul(base);
        }
    }
    k_hashes.push(acc);
    for i in k..n {
        let drop = token_hashes[i - k].wrapping_mul(base_pow);
        acc = acc.wrapping_sub(drop).wrapping_mul(base).wrapping_add(token_hashes[i]);
        k_hashes.push(acc);
    }

    // Winnow: in each window of `w` k-hashes, pick the minimum's
    // index. If two share the minimum, prefer the rightmost (which
    // keeps the algorithm's locality property).
    let mut prints = Vec::new();
    let mut last_picked: Option<usize> = None;
    for window_start in 0..k_hashes.len().saturating_sub(w) + 1 {
        let window_end = (window_start + w).min(k_hashes.len());
        if window_start >= window_end {
            break;
        }
        let mut min_idx = window_start;
        for j in (window_start + 1)..window_end {
            if k_hashes[j] <= k_hashes[min_idx] {
                min_idx = j;
            }
        }
        if last_picked != Some(min_idx) {
            prints.push(Fingerprint {
                hash: k_hashes[min_idx],
                token_idx: min_idx,
            });
            last_picked = Some(min_idx);
        }
    }
    prints
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x00000100000001B3);
    }
    h
}

fn detect_across_docs(docs: &[Doc], opts: DupOptions) -> Vec<DuplicateBlock> {
    // Inverted index: fingerprint hash → Vec<(doc_idx, fp_idx)>.
    let mut index: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (di, doc) in docs.iter().enumerate() {
        for (fi, fp) in doc.fps.iter().enumerate() {
            index.entry(fp.hash).or_default().push((di, fi));
        }
    }

    // For each multi-occurrence fingerprint, build candidate pairs
    // and try to extend forward through the fingerprint sequence.
    // Dedupe by tracking which (doc, fp_idx) starts have already
    // been consumed by a longer match.
    let mut seen_starts: HashMap<(usize, usize), bool> = HashMap::new();
    let mut blocks: Vec<DuplicateBlock> = Vec::new();
    for (di_a, doc_a) in docs.iter().enumerate() {
        for (fi_a, fp_a) in doc_a.fps.iter().enumerate() {
            let occurrences = match index.get(&fp_a.hash) {
                Some(v) => v,
                None => continue,
            };
            if occurrences.len() < 2 {
                continue;
            }
            for &(di_b, fi_b) in occurrences {
                // Only consider each unordered pair once, and skip
                // self-against-self at the same position.
                if di_b < di_a || (di_b == di_a && fi_b <= fi_a) {
                    continue;
                }
                if seen_starts.contains_key(&(di_b, fi_b)) {
                    continue;
                }
                let doc_b = &docs[di_b];
                // Extend forward through the fingerprint sequence.
                let mut len = 1usize;
                while fi_a + len < doc_a.fps.len()
                    && fi_b + len < doc_b.fps.len()
                    && doc_a.fps[fi_a + len].hash == doc_b.fps[fi_b + len].hash
                {
                    len += 1;
                }
                if len < 2 {
                    // A single shared fingerprint isn't a clone.
                    continue;
                }
                // Map fp range → token range → line range.
                let a_tok_start = doc_a.fps[fi_a].token_idx;
                let a_tok_end = doc_a.fps[fi_a + len - 1].token_idx;
                let b_tok_start = doc_b.fps[fi_b].token_idx;
                let b_tok_end = doc_b.fps[fi_b + len - 1].token_idx;
                let a_start_line = doc_a.tokens[a_tok_start].start_line;
                let a_end_line = doc_a.tokens[a_tok_end.min(doc_a.tokens.len() - 1)].end_line;
                let b_start_line = doc_b.tokens[b_tok_start].start_line;
                let b_end_line = doc_b.tokens[b_tok_end.min(doc_b.tokens.len() - 1)].end_line;
                let span_a = a_end_line.saturating_sub(a_start_line) + 1;
                let span_b = b_end_line.saturating_sub(b_start_line) + 1;
                let line_count = span_a.max(span_b);
                if line_count < opts.min_lines {
                    continue;
                }
                blocks.push(DuplicateBlock {
                    a_path: doc_a.path.clone(),
                    a_start_line,
                    a_end_line,
                    b_path: doc_b.path.clone(),
                    b_start_line,
                    b_end_line,
                    line_count,
                });
                // Mark every consumed start so we don't re-emit
                // shorter sub-runs of the same match.
                for offset in 0..len {
                    seen_starts.insert((di_a, fi_a + offset), true);
                    seen_starts.insert((di_b, fi_b + offset), true);
                }
            }
        }
    }
    // Sort for determinism (helps tests + UI).
    blocks.sort_by(|x, y| {
        x.a_path
            .cmp(&y.a_path)
            .then_with(|| x.a_start_line.cmp(&y.a_start_line))
            .then_with(|| x.b_path.cmp(&y.b_path))
    });
    blocks
}

#[cfg(test)]
mod tests;
