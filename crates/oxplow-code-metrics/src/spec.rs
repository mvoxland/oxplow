//! Per-language tree-sitter node-name tables.
//!
//! Each `LanguageSpec` says:
//! - which AST node kinds are "function-like" (so we count them as
//!   metric subjects),
//! - which child field name holds the function name,
//! - which field holds the parameter list and which child kinds count
//!   as one parameter,
//! - which AST node kinds count as a "decision point" for cyclomatic
//!   complexity (one branch per occurrence inside the function body).
//!
//! Decision-point sets follow McCabe's classic definition: every
//! `if`, `else if`, `case`, `catch`, `for`, `while`, `&&`, `||`, plus
//! ternary expressions.

use tree_sitter::Language as TsLanguage;

#[derive(Debug, Clone, Copy)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Java,
    C,
    Cpp,
}

pub struct LanguageSpec {
    /// AST node kinds that represent a function/method/closure body.
    pub function_kinds: &'static [&'static str],
    /// Field names to try (in order) for the function's identifier.
    pub name_fields: &'static [&'static str],
    /// Field names to try (in order) for the parameter list.
    pub param_list_fields: &'static [&'static str],
    /// AST node kinds inside a parameter list that represent one parameter.
    pub parameter_kinds: &'static [&'static str],
    /// AST node kinds that increment cyclomatic complexity by 1.
    pub decision_kinds: &'static [&'static str],
    /// Loader for the bundled tree-sitter grammar.
    grammar: fn() -> TsLanguage,
}

impl LanguageSpec {
    pub fn tree_sitter_language(&self) -> TsLanguage {
        (self.grammar)()
    }
}

impl Language {
    pub fn spec(&self) -> &'static LanguageSpec {
        match self {
            Language::Rust => &RUST,
            Language::TypeScript => &TYPESCRIPT,
            Language::Tsx => &TSX,
            Language::JavaScript => &JAVASCRIPT,
            Language::Python => &PYTHON,
            Language::Go => &GO,
            Language::Java => &JAVA,
            Language::C => &C,
            Language::Cpp => &CPP,
        }
    }
}

/// Cheap path-extension check.
pub fn language_for_path(path: &str) -> Option<Language> {
    let ext = path.rsplit('.').next()?;
    Some(match ext.to_ascii_lowercase().as_str() {
        "rs" => Language::Rust,
        "ts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "js" | "mjs" | "cjs" | "jsx" => Language::JavaScript,
        "py" => Language::Python,
        "go" => Language::Go,
        "java" => Language::Java,
        "c" | "h" => Language::C,
        "cc" | "cxx" | "cpp" | "hpp" | "hxx" => Language::Cpp,
        _ => return None,
    })
}

// ---- Rust ----

static RUST: LanguageSpec = LanguageSpec {
    function_kinds: &["function_item", "closure_expression"],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter", "self_parameter"],
    decision_kinds: &[
        "if_expression",
        "else_clause",
        "match_arm",
        "while_expression",
        "for_expression",
        "loop_expression",
        "try_expression",
        // boolean operators (&&, ||) are tokens inside binary_expression,
        // not their own nodes — skipping for simplicity.
    ],
    grammar: || tree_sitter_rust::LANGUAGE.into(),
};

// ---- TypeScript / TSX / JavaScript ----

static TYPESCRIPT: LanguageSpec = LanguageSpec {
    function_kinds: JS_FUNCTION_KINDS,
    name_fields: JS_NAME_FIELDS,
    param_list_fields: JS_PARAM_FIELDS,
    parameter_kinds: JS_PARAM_KINDS,
    decision_kinds: JS_DECISION_KINDS,
    grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
};

static TSX: LanguageSpec = LanguageSpec {
    function_kinds: JS_FUNCTION_KINDS,
    name_fields: JS_NAME_FIELDS,
    param_list_fields: JS_PARAM_FIELDS,
    parameter_kinds: JS_PARAM_KINDS,
    decision_kinds: JS_DECISION_KINDS,
    grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
};

static JAVASCRIPT: LanguageSpec = LanguageSpec {
    function_kinds: JS_FUNCTION_KINDS,
    name_fields: JS_NAME_FIELDS,
    param_list_fields: JS_PARAM_FIELDS,
    parameter_kinds: JS_PARAM_KINDS,
    decision_kinds: JS_DECISION_KINDS,
    grammar: || tree_sitter_javascript::LANGUAGE.into(),
};

static JS_FUNCTION_KINDS: &[&str] = &[
    "function_declaration",
    "function_expression",
    "arrow_function",
    "method_definition",
    "generator_function",
    "generator_function_declaration",
    "function_signature",
];
static JS_NAME_FIELDS: &[&str] = &["name"];
static JS_PARAM_FIELDS: &[&str] = &["parameters"];
static JS_PARAM_KINDS: &[&str] = &[
    "required_parameter",
    "optional_parameter",
    "rest_parameter",
    "identifier",
    "assignment_pattern",
    "object_pattern",
    "array_pattern",
];
static JS_DECISION_KINDS: &[&str] = &[
    "if_statement",
    "else_clause",
    "switch_case",
    "switch_default",
    "for_statement",
    "for_in_statement",
    "while_statement",
    "do_statement",
    "catch_clause",
    "ternary_expression",
];

// ---- Python ----

static PYTHON: LanguageSpec = LanguageSpec {
    function_kinds: &["function_definition", "lambda"],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &[
        "identifier",
        "default_parameter",
        "typed_parameter",
        "typed_default_parameter",
        "list_splat_pattern",
        "dictionary_splat_pattern",
        "tuple_pattern",
    ],
    decision_kinds: &[
        "if_statement",
        "elif_clause",
        "for_statement",
        "while_statement",
        "try_statement",
        "except_clause",
        "with_statement",
        "conditional_expression",
        "boolean_operator",
        "match_statement",
        "case_clause",
    ],
    grammar: || tree_sitter_python::LANGUAGE.into(),
};

// ---- Go ----

static GO: LanguageSpec = LanguageSpec {
    function_kinds: &["function_declaration", "method_declaration", "func_literal"],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter_declaration", "variadic_parameter_declaration"],
    decision_kinds: &[
        "if_statement",
        "for_statement",
        "type_switch_statement",
        "expression_switch_statement",
        "type_case",
        "expression_case",
        "select_statement",
        "communication_case",
    ],
    grammar: || tree_sitter_go::LANGUAGE.into(),
};

// ---- Java ----

static JAVA: LanguageSpec = LanguageSpec {
    function_kinds: &[
        "method_declaration",
        "constructor_declaration",
        "lambda_expression",
    ],
    name_fields: &["name"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["formal_parameter", "spread_parameter"],
    decision_kinds: &[
        "if_statement",
        "switch_label",
        "switch_block_statement_group",
        "for_statement",
        "enhanced_for_statement",
        "while_statement",
        "do_statement",
        "catch_clause",
        "ternary_expression",
    ],
    grammar: || tree_sitter_java::LANGUAGE.into(),
};

// ---- C ----

static C: LanguageSpec = LanguageSpec {
    function_kinds: &["function_definition"],
    name_fields: &["declarator"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter_declaration"],
    decision_kinds: &[
        "if_statement",
        "case_statement",
        "for_statement",
        "while_statement",
        "do_statement",
        "conditional_expression",
    ],
    grammar: || tree_sitter_c::LANGUAGE.into(),
};

// ---- C++ ----

static CPP: LanguageSpec = LanguageSpec {
    function_kinds: &[
        "function_definition",
        "lambda_expression",
    ],
    name_fields: &["declarator"],
    param_list_fields: &["parameters"],
    parameter_kinds: &["parameter_declaration", "optional_parameter_declaration"],
    decision_kinds: &[
        "if_statement",
        "case_statement",
        "for_statement",
        "for_range_loop",
        "while_statement",
        "do_statement",
        "catch_clause",
        "conditional_expression",
    ],
    grammar: || tree_sitter_cpp::LANGUAGE.into(),
};
