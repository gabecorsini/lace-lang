//! Integration tests for lace-lsp diagnostics.
//!
//! These tests call `compute_diagnostics` directly (no network/stdio) and
//! verify that a Lace source with a type error produces at least one LSP
//! Diagnostic with severity Error.

use lace_lsp::compute_diagnostics;
use tower_lsp::lsp_types::DiagnosticSeverity;

/// A snippet with a clear type error: assigning a String where an Int is expected.
const TYPE_ERROR_SNIPPET: &str = r#"
fn bad() -> Int {
    "hello"
}
"#;

/// A snippet with no errors — should produce no diagnostics.
const CLEAN_SNIPPET: &str = r#"
fn add(a: Int, b: Int) -> Int [Pure] {
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
fn type_error_produces_error_diagnostic() {
    let diags = compute_diagnostics(TYPE_ERROR_SNIPPET);
    assert!(
        !diags.is_empty(),
        "expected at least one diagnostic for a type-error snippet, got none"
    );
    let has_error = diags
        .iter()
        .any(|d| d.severity == Some(DiagnosticSeverity::ERROR));
    assert!(
        has_error,
        "expected at least one ERROR-severity diagnostic, got: {diags:#?}"
    );
}

#[test]
fn clean_snippet_produces_no_diagnostics() {
    let diags = compute_diagnostics(CLEAN_SNIPPET);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
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
    let has_error = diags
        .iter()
        .any(|d| d.severity == Some(DiagnosticSeverity::ERROR));
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
