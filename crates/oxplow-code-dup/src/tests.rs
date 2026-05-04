use super::*;

#[test]
fn detects_obvious_clone_across_two_files() {
    let body = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out
}
"#;
    let files = vec![
        ("src/a.rs".to_string(), body.to_string()),
        ("src/b.rs".to_string(), body.to_string()),
    ];
    let blocks = detect_duplicates(files, DupOptions::default());
    assert!(!blocks.is_empty(), "expected at least one duplicate");
    let b = &blocks[0];
    assert_eq!(b.a_path, "src/a.rs");
    assert_eq!(b.b_path, "src/b.rs");
    assert!(b.line_count >= 5);
}

#[test]
fn rename_resistant_clones() {
    let original = r#"
fn process(items: Vec<i32>) -> Vec<i32> {
    let mut result = Vec::new();
    for thing in items {
        if thing > 0 {
            result.push(thing * 2);
        } else if thing < 0 {
            result.push(thing * -1);
        } else {
            result.push(0);
        }
    }
    result
}
"#;
    let renamed = r#"
fn handle(values: Vec<i32>) -> Vec<i32> {
    let mut output = Vec::new();
    for v in values {
        if v > 0 {
            output.push(v * 2);
        } else if v < 0 {
            output.push(v * -1);
        } else {
            output.push(0);
        }
    }
    output
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), original.to_string()),
            ("src/b.rs".to_string(), renamed.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(!blocks.is_empty(), "renamed clones should still match");
}

#[test]
fn ignores_unique_files() {
    let a = "fn add(a: i32, b: i32) -> i32 { a + b }";
    let b = "fn unrelated() { println!(\"hello world\"); }";
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), a.to_string()),
            ("src/b.rs".to_string(), b.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty());
}

#[test]
fn ignores_short_clones_below_min_lines() {
    let body = "fn tiny() { println!(\"x\"); }";
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), body.to_string()),
            ("src/b.rs".to_string(), body.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty(), "single-line clone should be filtered");
}

#[test]
fn cross_language_files_dont_match() {
    // Even if textually similar, different languages produce
    // different token kinds and shouldn't cross-match.
    let rs = r#"fn add(a: i32, b: i32) -> i32 { if a > b { a } else { b } }"#;
    let py = r#"def add(a, b):
    if a > b:
        return a
    else:
        return b
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), rs.to_string()),
            ("src/b.py".to_string(), py.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty());
}

#[test]
fn language_salt_prevents_cross_language_collisions() {
    // Two grammars producing structurally similar token sequences
    // with overlapping node-kind names should never share fingerprints.
    // We construct C and Rust functions whose normalized token streams
    // overlap heavily ("if", "(", "ID", ")", "{", ... "}").
    let c_src = r#"
int a(int x) {
    if (x > 0) { return 1; }
    if (x > 1) { return 2; }
    if (x > 2) { return 3; }
    if (x > 3) { return 4; }
    return 0;
}
"#;
    let rust_src = r#"
fn a(x: i32) -> i32 {
    if x > 0 { return 1; }
    if x > 1 { return 2; }
    if x > 2 { return 3; }
    if x > 3 { return 4; }
    0
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/x.c".to_string(), c_src.to_string()),
            ("src/x.rs".to_string(), rust_src.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(
        blocks.is_empty(),
        "cross-language fingerprints should be salted apart, got {:?}",
        blocks
    );
}

#[test]
fn extend_tolerates_minor_winnowing_divergence() {
    // Two clones with a one-token difference in the middle should
    // still be detected as one block (skip-tolerance), not two
    // smaller fragments or none at all.
    let a = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out.push(99);
    out
}
"#;
    let b = r#"
fn helper(items: Vec<i32>) -> Vec<i32> {
    let mut out = Vec::new();
    for item in items {
        if item > 0 {
            out.push(item * 2);
        } else if item < 0 {
            out.push(item * -1);
        } else {
            out.push(0);
        }
    }
    out.push(100);
    out
}
"#;
    let blocks = detect_duplicates(
        vec![
            ("src/a.rs".to_string(), a.to_string()),
            ("src/b.rs".to_string(), b.to_string()),
        ],
        DupOptions::default(),
    );
    assert!(!blocks.is_empty(), "near-clone should still match");
}

#[test]
fn skips_unsupported_languages() {
    let blocks = detect_duplicates(
        vec![
            ("README.md".to_string(), "# heading\nlots of text".to_string()),
            (
                "OTHER.md".to_string(),
                "# heading\nlots of text".to_string(),
            ),
        ],
        DupOptions::default(),
    );
    assert!(blocks.is_empty());
}
