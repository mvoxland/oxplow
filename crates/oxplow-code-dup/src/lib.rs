//! Cross-file token-stream duplicate-block detector. The pipeline:
//!
//! 1. **Tokenize** each file by walking the tree-sitter AST and
//!    emitting a normalized token per leaf node (identifiers, numeric
//!    literals, and string literals are folded to placeholder kinds
//!    so that renames and constant changes don't suppress matches).
//! 2. **K-gram hash** over runs of `K` consecutive tokens. Each
//!    file's hashes are salted by its language tag so token streams
//!    from different grammars cannot collide.
//! 3. **Winnow** with window `W` to keep one fingerprint per `W`
//!    tokens — Schleimer/Aiken 2003.
//! 4. **Inverted index** fingerprint → occurrences.
//! 5. **Extend** each multi-occurrence fingerprint forward in the
//!    fingerprint sequence, tolerating up to `MAX_SKIP` non-matching
//!    fingerprints between matches (since winnowing samples
//!    stochastically and two real clones can have slightly divergent
//!    fingerprint subsets).
//! 6. **Filter** runs whose line span is shorter than `min_lines`.
//!
//! Tunables are exposed on `DupOptions` (K=20, W=4, min_lines=10 by
//! default — small enough that a real extracted helper still surfaces,
//! large enough that thiserror enum boilerplate / 5-line idioms don't).
//!
//! # Multi-way clones
//!
//! When fingerprint F appears in three or more files, only one
//! pairing is reported per fingerprint position — once a (doc, fp)
//! position is consumed by a match it's marked seen and won't seed
//! another pair. So a 3-way clone (A↔B↔C) typically surfaces as a
//! single (A, B) row in the panel, not three. This keeps the
//! Code Quality panel readable; if you need full multi-way analysis
//! the renderer would have to walk peer chains via `extra.peerPath`.

use std::collections::{BTreeSet, HashMap, HashSet};

use oxplow_code_metrics::{Language, language_for_path};
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
            min_lines: 10,
        }
    }
}

/// Maximum gap (in fingerprint indices) between matching fingerprints
/// that we'll still consider part of the same run. Two real clones
/// can have slightly divergent winnowing samples, especially at
/// boundaries; tolerating one or two skips keeps short matches from
/// fragmenting without exploding false positives.
const MAX_SKIP: usize = 2;

/// Scan a batch of (path, content) pairs for duplicate blocks. All
/// pairwise matches across distinct positions are surfaced.
///
/// **Note**: same-path matches (a region of a file matching another
/// region of the SAME file) are intentionally surfaced here — the
/// raw detector treats two ranges in one file as a valid in-file
/// clone. Callers that don't want that (the change-analysis flow,
/// for instance) should use [`detect_duplicates_scoped`] which
/// filters them out.
///
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
        let mut parser = Parser::new();
        if parser.set_language(&lang.tree_sitter_language()).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(source.as_ref(), None) else {
            continue;
        };
        let tokens = tokenize_source(tree.root_node(), lang);
        if tokens.len() < opts.k {
            continue;
        }
        let fps = winnow(&tokens, lang, opts.k, opts.w);
        if fps.is_empty() {
            continue;
        }
        docs.push(Doc { path, tokens, fps });
    }
    detect_across_docs(&docs, opts)
}

/// Same as [`detect_duplicates`] but applies "scope" semantics:
///
/// - The whole corpus participates in fingerprint matching (so a
///   changed file can clone-match an unchanged peer file).
/// - A finding is reported only if **at least one** side's path is
///   in `scope_paths`.
/// - Same-path matches (file vs itself) are dropped — two ranges
///   in one file that happen to fingerprint-match are almost always
///   shifted-by-one artifacts of winnowing on long token streams,
///   not real duplication worth surfacing.
/// - When only one side is in scope it is rotated to the A side
///   of the [`DuplicateBlock`] so the renderer's "side A is what
///   you're analyzing, side B is the peer" convention holds.
pub fn detect_duplicates_scoped<I, P, S>(
    files: I,
    scope_paths: &BTreeSet<String>,
    opts: DupOptions,
) -> Vec<DuplicateBlock>
where
    I: IntoIterator<Item = (P, S)>,
    P: Into<String>,
    S: AsRef<str>,
{
    let raw = detect_duplicates(files, opts);
    let mut out = Vec::with_capacity(raw.len());
    for block in raw {
        if block.a_path == block.b_path {
            continue;
        }
        let a_in = scope_paths.contains(&block.a_path);
        let b_in = scope_paths.contains(&block.b_path);
        if !a_in && !b_in {
            continue;
        }
        if !a_in && b_in {
            // Side B is the scope path; flip so side A is the
            // analyzed file the panel row actually anchors on.
            out.push(DuplicateBlock {
                a_path: block.b_path,
                a_start_line: block.b_start_line,
                a_end_line: block.b_end_line,
                b_path: block.a_path,
                b_start_line: block.a_start_line,
                b_end_line: block.a_end_line,
                line_count: block.line_count,
            });
        } else {
            out.push(block);
        }
    }
    out
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

/// Roll a 64-bit polynomial hash over tokens (salted by `language`),
/// then winnow per Schleimer/Aiken to keep one fingerprint per
/// minimum-of-window k-hash.
fn winnow(tokens: &[Token], language: Language, k: usize, w: usize) -> Vec<Fingerprint> {
    let n = tokens.len();
    if n < k {
        return Vec::new();
    }
    // Per-token hash: 64-bit FNV of the token kind, seeded by
    // language tag so cross-language streams cannot collide.
    let token_hashes: Vec<u64> = tokens
        .iter()
        .map(|t| fnv1a_seeded(language.tag(), t.kind.as_bytes()))
        .collect();

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
        acc = acc
            .wrapping_sub(drop)
            .wrapping_mul(base)
            .wrapping_add(token_hashes[i]);
        k_hashes.push(acc);
    }

    // Winnow: in each window of `w` k-hashes, pick the rightmost
    // minimum (Schleimer/Aiken's tie-break preserves locality).
    let mut prints = Vec::new();
    let mut last_picked: Option<usize> = None;
    if k_hashes.is_empty() {
        return prints;
    }
    let last_window_start = k_hashes.len().saturating_sub(w);
    for window_start in 0..=last_window_start {
        let window_end = (window_start + w).min(k_hashes.len());
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

#[inline]
fn fnv1a_seeded(seed: u8, bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ (seed as u64);
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

    let mut seen_starts: HashSet<(usize, usize)> = HashSet::new();
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
                if seen_starts.contains(&(di_b, fi_b)) {
                    continue;
                }
                let doc_b = &docs[di_b];
                let Some(run) = extend_run(doc_a, fi_a, doc_b, fi_b) else {
                    continue;
                };
                let RunSpan {
                    a_fp_end,
                    b_fp_end,
                    consumed_a,
                    consumed_b,
                } = run;
                let a_tok_start = doc_a.fps[fi_a].token_idx;
                let a_tok_end = doc_a.fps[a_fp_end].token_idx;
                let b_tok_start = doc_b.fps[fi_b].token_idx;
                let b_tok_end = doc_b.fps[b_fp_end].token_idx;
                let a_end_line =
                    doc_a.tokens[a_tok_end.min(doc_a.tokens.len() - 1)].end_line;
                let b_end_line =
                    doc_b.tokens[b_tok_end.min(doc_b.tokens.len() - 1)].end_line;
                let a_start_line = doc_a.tokens[a_tok_start].start_line;
                let b_start_line = doc_b.tokens[b_tok_start].start_line;
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
                for fp_idx in consumed_a {
                    seen_starts.insert((di_a, fp_idx));
                }
                for fp_idx in consumed_b {
                    seen_starts.insert((di_b, fp_idx));
                }
            }
        }
    }
    blocks.sort_by(|x, y| {
        x.a_path
            .cmp(&y.a_path)
            .then_with(|| x.a_start_line.cmp(&y.a_start_line))
            .then_with(|| x.b_path.cmp(&y.b_path))
    });
    blocks
}

struct RunSpan {
    /// Last matching fingerprint index in doc A.
    a_fp_end: usize,
    /// Last matching fingerprint index in doc B.
    b_fp_end: usize,
    /// Every fingerprint index in doc A consumed by this run.
    consumed_a: Vec<usize>,
    /// Every fingerprint index in doc B consumed by this run.
    consumed_b: Vec<usize>,
}

/// Extend a (a, b) match through both fingerprint streams. Tolerates
/// up to `MAX_SKIP` non-matching fingerprints on either side before
/// giving up (winnowing can sample slightly different positions for
/// the same underlying clone).
fn extend_run(doc_a: &Doc, fi_a: usize, doc_b: &Doc, fi_b: usize) -> Option<RunSpan> {
    let mut consumed_a = vec![fi_a];
    let mut consumed_b = vec![fi_b];
    let mut a = fi_a + 1;
    let mut b = fi_b + 1;
    loop {
        // Look for the next matching pair within the skip window.
        let mut matched = None;
        'search: for da in 0..=MAX_SKIP {
            for db in 0..=MAX_SKIP {
                let na = a + da;
                let nb = b + db;
                if na >= doc_a.fps.len() || nb >= doc_b.fps.len() {
                    continue;
                }
                if doc_a.fps[na].hash == doc_b.fps[nb].hash {
                    matched = Some((na, nb));
                    break 'search;
                }
            }
        }
        match matched {
            Some((na, nb)) => {
                consumed_a.push(na);
                consumed_b.push(nb);
                a = na + 1;
                b = nb + 1;
            }
            None => break,
        }
    }
    if consumed_a.len() < 2 {
        return None;
    }
    Some(RunSpan {
        a_fp_end: *consumed_a.last().unwrap(),
        b_fp_end: *consumed_b.last().unwrap(),
        consumed_a,
        consumed_b,
    })
}

#[cfg(test)]
mod tests;
