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
