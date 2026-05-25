// Phase 36 tests: did-you-mean suggestions for stdlib typos and unknown identifiers
use lace_interp::{Interpreter, Value};
use lace_parser::parse_program;

fn run(src: &str) -> Result<Value, String> {
    let (prog, errs) = parse_program(src);
    if !errs.is_empty() {
        return Err(errs.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; "));
    }
    let prog = prog.ok_or("parse returned None")?;
    Interpreter::new(None)
        .run_program(&prog)
        .map_err(|e| e.message)
}

#[test]
fn test_did_you_mean_list_filter() {
    let result = run("let x = List.fliter([1,2,3], fn(x) { x > 1 })");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("did you mean"), "Expected did-you-mean in: {}", err);
    assert!(err.contains("List.filter"), "Expected suggestion 'List.filter' in: {}", err);
}

#[test]
fn test_did_you_mean_str_split() {
    let result = run(r#"let y = Str.splitt("hello world", " ")"#);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("did you mean"), "Expected did-you-mean in: {}", err);
    assert!(err.contains("Str.split"), "Expected suggestion 'Str.split' in: {}", err);
}

#[test]
fn test_did_you_mean_list_length() {
    let result = run("let n = List.lenght([1,2,3])");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("did you mean"), "Expected did-you-mean in: {}", err);
    assert!(err.contains("List.length"), "Expected suggestion 'List.length' in: {}", err);
}

#[test]
fn test_did_you_mean_unknown_ident() {
    let result = run("let foo = 42\nlet x = fooo + 1");
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("did you mean"), "Expected did-you-mean in: {}", err);
    assert!(err.contains("foo"), "Expected suggestion 'foo' in: {}", err);
}

#[test]
fn test_no_suggestion_for_very_different_name() {
    // "zzz" is very different from any known method, no suggestion expected
    let result = run("let x = List.zzz([1,2,3])");
    assert!(result.is_err());
    let err = result.unwrap_err();
    // Should still produce unsupported method error, just without a suggestion
    assert!(err.contains("unsupported method"), "Expected unsupported method in: {}", err);
}

#[test]
fn test_valid_calls_still_work() {
    let result = run("let x = List.filter([1, 2, 3, 4], fn(n) { n > 2 })");
    assert!(result.is_ok(), "Expected ok but got: {:?}", result);
    let result2 = run(r#"let y = Str.split("hello world", " ")"#);
    assert!(result2.is_ok(), "Expected ok but got: {:?}", result2);
    let result3 = run("let _n = List.length([1, 2, 3])");
    assert!(result3.is_ok(), "Expected ok but got: {:?}", result3);
}
