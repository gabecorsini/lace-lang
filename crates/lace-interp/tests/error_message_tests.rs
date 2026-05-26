// error_message_tests.rs — tests for improved error messages and did-you-mean suggestions
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
fn test_did_you_mean_fss_suggests_fs() {
    let src = r#"fn main() -> Unit [IO] { Fss.read("x") }"#;
    let result = run(src);
    assert!(result.is_err(), "expected error for unknown 'Fss'");
    let err = result.unwrap_err();
    assert!(
        err.contains("did you mean 'Fs'"),
        "expected \"did you mean 'Fs'\" in error: {}",
        err
    );
}

#[test]
fn test_did_you_mean_maths_suggests_math() {
    let src = r#"fn main() -> Unit [Pure] { let _ = Maths.pi() }"#;
    let result = run(src);
    assert!(result.is_err(), "expected error for unknown 'Maths'");
    let err = result.unwrap_err();
    assert!(
        err.contains("did you mean 'Math'"),
        "expected \"did you mean 'Math'\" in error: {}",
        err
    );
}
