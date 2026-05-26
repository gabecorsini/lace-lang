//! Integration tests for lace-lsp diagnostics.
//!
//! These tests verify that parse/type errors in Lace source produce
//! appropriate diagnostic output from the LSP server's handler.

use lace_lsp::compute_diagnostics;

/// A snippet with no errors — should produce no diagnostics.
const CLEAN_SNIPPET: &str = r#"
fn add(a: Int, b: Int) -> Int {
    a + b
}
"#;

/// A snippet with an unknown identifier.
const UNKNOWN_IDENT_SNIPPET: &str = r#"
fn broken() -> Int {
    nonexistent_variable
}
"#;

#[test]
fn clean_snippet_produces_no_diagnostics() {
    let diags = compute_diagnostics(CLEAN_SNIPPET);
    let errors: Vec<_> = diags.iter().filter(|d| d["severity"] == 1).collect();
    assert!(
        errors.is_empty(),
        "expected no error diagnostics for clean snippet, got: {errors:#?}"
    );
}

#[test]
fn unknown_identifier_produces_error_diagnostic() {
    let diags = compute_diagnostics(UNKNOWN_IDENT_SNIPPET);
    assert!(
        !diags.is_empty(),
        "expected diagnostics for unknown identifier, got none"
    );
    let has_error = diags.iter().any(|d| d["severity"] == 1);
    assert!(
        has_error,
        "expected ERROR diagnostic for unknown identifier, got: {diags:#?}"
    );
}

#[test]
fn parse_error_produces_error_diagnostic() {
    let bad_syntax = "fn oops( { }";
    let diags = compute_diagnostics(bad_syntax);
    assert!(
        !diags.is_empty(),
        "expected diagnostics for parse error, got none"
    );
}
