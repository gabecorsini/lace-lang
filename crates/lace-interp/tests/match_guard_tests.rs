use lace_interp::{Interpreter, Value};
use lace_parser::parse_program;

fn run(src: &str) -> Value {
    let (prog, errs) = parse_program(src);
    assert!(errs.is_empty(), "parse errors: {:?}", errs);
    let prog = prog.expect("parse returned None");
    Interpreter::new(None)
        .run_program(&prog)
        .expect("runtime error")
}

#[test]
fn test_match_guard_positive_negative_zero() {
    let src = r#"
fn classify(x: Int) -> String [Pure] {
    match x {
        n if n > 0 => { "positive" },
        n if n < 0 => { "negative" },
        _ => { "zero" },
    }
}
fn main() -> String [Pure] {
    let a = classify(5)
    let b = classify(-3)
    let c = classify(0)
    a ++ "," ++ b ++ "," ++ c
}
"#;
    let result = run(src);
    assert_eq!(result, Value::String("positive,negative,zero".to_string()));
}

#[test]
fn test_match_guard_uses_bound_variable() {
    let src = r#"
fn big_or_small(x: Int) -> String [Pure] {
    match x {
        n if n > 10 => { "big" },
        n if n > 0 => { "small" },
        _ => { "nonpositive" },
    }
}
fn main() -> String [Pure] {
    let a = big_or_small(42)
    let b = big_or_small(3)
    let c = big_or_small(0)
    a ++ "," ++ b ++ "," ++ c
}
"#;
    let result = run(src);
    assert_eq!(result, Value::String("big,small,nonpositive".to_string()));
}
