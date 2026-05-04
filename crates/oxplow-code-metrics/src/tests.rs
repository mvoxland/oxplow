use super::*;

#[test]
fn rust_simple_function() {
    let src = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    let f = &m[0];
    assert_eq!(f.name, "add");
    assert_eq!(f.parameter_count, 2);
    assert_eq!(f.complexity, 1);
    assert!(f.length >= 3);
}

#[test]
fn rust_branching_function_increments_complexity() {
    let src = r#"
fn classify(x: i32) -> &'static str {
    if x > 0 {
        "pos"
    } else if x < 0 {
        "neg"
    } else {
        "zero"
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    // base 1 + outer if + else_clause + inner if-as-expression
    assert!(m[0].complexity >= 3, "got {}", m[0].complexity);
}

#[test]
fn rust_match_increments_per_arm() {
    let src = r#"
fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1 => "one",
        _ => "other",
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    // 3 match arms = +3
    assert_eq!(m[0].complexity, 4);
}

#[test]
fn typescript_arrow_function_with_branches() {
    let src = r#"
const handler = (req: Request, res: Response) => {
    if (req.method === "GET") {
        return res.send("ok");
    }
    if (req.method === "POST") {
        return res.send("created");
    }
    return res.send("?");
};
"#;
    let m = analyze_file("src/x.ts", src);
    assert!(!m.is_empty());
    let f = m.iter().find(|f| f.parameter_count == 2).expect("arrow");
    assert!(f.complexity >= 3, "got {}", f.complexity);
}

#[test]
fn python_function_with_loops() {
    let src = "
def histogram(items):
    out = {}
    for it in items:
        if it in out:
            out[it] += 1
        else:
            out[it] = 1
    return out
";
    let m = analyze_file("src/x.py", src);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].name, "histogram");
    assert_eq!(m[0].parameter_count, 1);
    assert!(m[0].complexity >= 3, "got {}", m[0].complexity);
}

#[test]
fn go_method_counts_receiver() {
    let src = r#"
package main

func (s *Server) handle(w http.ResponseWriter, r *http.Request) {
    if r.URL.Path == "/" {
        w.WriteHeader(200)
        return
    }
    w.WriteHeader(404)
}
"#;
    let m = analyze_file("main.go", src);
    let f = m.iter().find(|f| f.name == "handle").expect("method");
    assert_eq!(f.parameter_count, 2);
    assert!(f.complexity >= 2);
}

#[test]
fn java_method() {
    let src = r#"
class Foo {
    public int classify(int x) {
        if (x > 0) return 1;
        if (x < 0) return -1;
        return 0;
    }
}
"#;
    let m = analyze_file("Foo.java", src);
    let f = m.iter().find(|f| f.name == "classify").expect("method");
    assert_eq!(f.parameter_count, 1);
    assert!(f.complexity >= 3);
}

#[test]
fn unsupported_language_returns_empty() {
    let m = analyze_file("README.md", "# heading");
    assert!(m.is_empty());
}

#[test]
fn extensionless_path_is_not_classified() {
    // Regression: previously a path like "rs" (no extension) was
    // misclassified as Rust because `rsplit('.').next()` returned
    // the whole string.
    let m = analyze_file("rs", "fn x() {}");
    assert!(m.is_empty());
    let m = analyze_file("Makefile", "all: foo\n");
    assert!(m.is_empty());
}

#[test]
fn c_function_name_is_just_the_identifier() {
    let src = r#"
int classify(int x) {
    if (x > 0) return 1;
    if (x < 0) return -1;
    return 0;
}
"#;
    let m = analyze_file("src/x.c", src);
    let f = m.iter().find(|f| f.name == "classify").unwrap_or_else(|| {
        panic!("expected name 'classify', got {:?}", m.iter().map(|f| &f.name).collect::<Vec<_>>())
    });
    assert_eq!(f.parameter_count, 1);
    assert!(f.complexity >= 3);
}

#[test]
fn cpp_function_name_strips_declarator_decoration() {
    let src = r#"
int Foo::bar(int x, int y) {
    if (x > y) return x;
    return y;
}
"#;
    let m = analyze_file("src/x.cpp", src);
    let names: Vec<_> = m.iter().map(|f| f.name.as_str()).collect();
    // The qualified id parses as `Foo::bar`; the inner identifier is
    // `bar`. Either is acceptable as long as we don't return the
    // whole declarator including parameters.
    assert!(
        names.iter().any(|n| *n == "bar" || *n == "Foo::bar"),
        "got {:?}",
        names
    );
    assert!(!names.iter().any(|n| n.contains('(')), "name leaked declarator parens: {:?}", names);
}

#[test]
fn rust_else_chain_does_not_double_count() {
    // Regression: previously `if/else if/else` was scored as
    // `if + else + if + else` = 4. McCabe says +2 (the two ifs).
    let src = r#"
fn classify(x: i32) -> &'static str {
    if x > 0 {
        "pos"
    } else if x < 0 {
        "neg"
    } else {
        "zero"
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].complexity, 3, "got {}", m[0].complexity);
}

#[test]
fn nested_functions_each_get_their_own_record() {
    let src = r#"
fn outer() {
    fn inner(x: i32) -> i32 {
        if x > 0 { x } else { -x }
    }
    let _ = inner(1);
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 2, "got {:?}", m.iter().map(|f| &f.name).collect::<Vec<_>>());
    let inner = m.iter().find(|f| f.name == "inner").unwrap();
    let outer = m.iter().find(|f| f.name == "outer").unwrap();
    // Inner function's `if` should count toward inner, not outer.
    assert!(inner.complexity >= 2);
    assert_eq!(outer.complexity, 1);
}
