// Phase 9 – string stdlib, full arithmetic, ? propagation
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
fn test_string_len() {
    let src = r#"fn main() -> Int [Pure] { "hello".len() }"#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_string_split() {
    let src = r#"
fn main() -> Int [Pure] {
    let parts = "a,b,c".split(",")
    List.length(parts)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_string_contains() {
    let src = r#"fn main() -> Bool [Pure] { "hello world".contains("world") }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_trim() {
    let src = r#"fn main() -> String [Pure] { "  hi  ".trim() }"#;
    assert_eq!(run(src).unwrap(), Value::String("hi".into()));
}

#[test]
fn test_string_to_upper_lower() {
    let src = r#"fn main() -> Bool [Pure] {
    let up = "hello".to_upper()
    let down = "WORLD".to_lower()
    up == "HELLO" && down == "world"
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_starts_ends_with() {
    let src = r#"fn main() -> Bool [Pure] {
    "foobar".starts_with("foo") && "foobar".ends_with("bar")
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_int_arithmetic() {
    let src = r#"fn main() -> Int [Pure] { 10 // 3 }"#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_float_arithmetic() {
    let src = r#"fn main() -> Bool [Pure] { 1.5 + 0.5 == 2.0 }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_mixed_arithmetic() {
    let src = r#"fn main() -> Int [Pure] { 10 % 3 }"#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_question_ok_propagation() {
    let src = r#"
fn get_ok() -> Int [Pure] {
    let r = Ok(42)
    r?
}
fn main() -> Int [Pure] { get_ok() }
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_question_err_propagation() {
    let src = r#"
fn may_fail() -> Int [Pure] {
    let r = Err("bad")
    r?
}
fn main() -> Unit [Pure] {
    let result = may_fail()
    assert_err(result, "should be err")
}
"#;
    assert!(run(src).is_ok());
}
