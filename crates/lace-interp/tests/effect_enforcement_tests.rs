/// Effect enforcement tests: [Pure] fn calling IO is a hard error (not a warning).
use lace_effects::{check_program, IssueLevel};
use lace_parser::parse_program;

fn effect_check(src: &str) -> Vec<lace_effects::EffectIssue> {
    let (prog, errs) = parse_program(src);
    assert!(errs.is_empty(), "parse errors: {errs:?}");
    let prog = prog.expect("no program");
    check_program(&prog)
}

fn has_error(issues: &[lace_effects::EffectIssue]) -> bool {
    issues.iter().any(|i| i.level == IssueLevel::Error)
}

// ── A [Pure] fn calling println (IO) must produce a hard error ───────────────

#[test]
fn test_pure_fn_calling_println_is_hard_error() {
    let src = r#"
fn greet(name: String) -> String [Pure] {
    println(name)
    name
}
fn main() [IO] {
    let _ = greet("world")
}
"#;
    let issues = effect_check(src);
    assert!(
        has_error(&issues),
        "[Pure] fn calling println should produce a hard effect error; issues: {issues:?}"
    );
    // Ensure the error mentions the function name
    let errors: Vec<_> = issues.iter().filter(|i| i.level == IssueLevel::Error).collect();
    assert!(
        errors.iter().any(|i| i.function == "greet"),
        "error should be on 'greet', got: {errors:?}"
    );
}

// ── A [Pure] fn doing only arithmetic must be accepted (no errors) ────────────

#[test]
fn test_pure_fn_arithmetic_is_accepted() {
    let src = r#"
fn add(a: Int, b: Int) -> Int [Pure] {
    a + b
}
fn main() [IO] {
    let x = add(2, 3)
    println(to_string(x))
}
"#;
    let issues = effect_check(src);
    let errors: Vec<_> = issues.iter().filter(|i| i.level == IssueLevel::Error).collect();
    assert!(
        errors.is_empty(),
        "pure arithmetic fn should produce no errors; errors: {errors:?}"
    );
}

// ── An [IO] fn calling another [IO] fn must be accepted ──────────────────────

#[test]
fn test_io_fn_calling_io_fn_is_accepted() {
    let src = r#"
fn log_msg(msg: String) [IO] {
    println(msg)
}
fn run_job() [IO] {
    log_msg("starting")
    log_msg("done")
}
fn main() [IO] {
    run_job()
}
"#;
    let issues = effect_check(src);
    let errors: Vec<_> = issues.iter().filter(|i| i.level == IssueLevel::Error).collect();
    assert!(
        errors.is_empty(),
        "[IO] fn calling [IO] fn should produce no errors; errors: {errors:?}"
    );
}

// ── fn main() is always allowed to be IO — no error even if body has IO ──────

#[test]
fn test_main_with_io_is_always_valid() {
    let src = r#"
fn main() [IO] {
    println("hello")
}
"#;
    let issues = effect_check(src);
    let errors: Vec<_> = issues.iter().filter(|i| i.level == IssueLevel::Error).collect();
    assert!(
        errors.is_empty(),
        "fn main() [IO] should never produce errors; errors: {errors:?}"
    );
}

// ── A [Pure] fn calling a [ToolCall] fn is also a hard error ─────────────────

#[test]
fn test_pure_fn_calling_io_fn_is_hard_error() {
    let src = r#"
fn do_io() [IO] {
    println("side effect")
}
fn pure_caller() -> Int [Pure] {
    do_io()
    42
}
fn main() [IO] {
    let _ = pure_caller()
}
"#;
    let issues = effect_check(src);
    assert!(
        has_error(&issues),
        "[Pure] fn calling [IO] fn should produce a hard effect error; issues: {issues:?}"
    );
    let errors: Vec<_> = issues.iter().filter(|i| i.level == IssueLevel::Error).collect();
    assert!(
        errors.iter().any(|i| i.function == "pure_caller"),
        "error should be on 'pure_caller', got: {errors:?}"
    );
}
