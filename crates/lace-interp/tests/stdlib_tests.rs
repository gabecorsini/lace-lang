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

// ── Regression tests for bug sweep ────────────────────────────────────────────

/// Division by zero must produce a clean runtime error, not a panic.
#[test]
fn test_div_by_zero_error() {
    let src = r#"fn main() -> Int [Pure] { 10 / 0 }"#;
    let err = run(src).unwrap_err();
    assert!(err.contains("division by zero"), "got: {err}");
}

/// Integer division by zero must also produce a clean error.
#[test]
fn test_int_div_by_zero_error() {
    let src = r#"fn main() -> Int [Pure] { 10 // 0 }"#;
    let err = run(src).unwrap_err();
    assert!(err.contains("division by zero"), "got: {err}");
}

/// Remainder by zero must produce a clean error, not a panic.
#[test]
fn test_rem_by_zero_error() {
    let src = r#"fn main() -> Int [Pure] { 10 % 0 }"#;
    let err = run(src).unwrap_err();
    assert!(err.contains("remainder by zero") || err.contains("zero"), "got: {err}");
}

/// Integer overflow (addition) must produce a clean error, not a panic.
#[test]
fn test_int_overflow_add() {
    let src = r#"fn main() -> Int [Pure] {
    let x = 9223372036854775807
    x + 1
}"#;
    let err = run(src).unwrap_err();
    assert!(err.contains("overflow"), "got: {err}");
}

/// Integer overflow (multiplication) must produce a clean error.
#[test]
fn test_int_overflow_mul() {
    let src = r#"fn main() -> Int [Pure] {
    let x = 9223372036854775807
    x * 2
}"#;
    let err = run(src).unwrap_err();
    assert!(err.contains("overflow"), "got: {err}");
}

/// String.replace method.
#[test]
fn test_string_replace() {
    let src = r#"fn main() -> String [Pure] { "hello world".replace("world", "lace") }"#;
    assert_eq!(run(src).unwrap(), Value::String("hello lace".into()));
}

/// String.is_empty method.
#[test]
fn test_string_is_empty() {
    let src = r#"fn main() -> Bool [Pure] { "".is_empty() && !("x".is_empty()) }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

/// List.is_empty method.
#[test]
fn test_list_is_empty() {
    let src = r#"fn main() -> Bool [Pure] {
    let empty = []
    empty.is_empty()
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

/// String.char_at returns Some for valid index and None for out-of-bounds.
#[test]
fn test_string_char_at_valid() {
    let src = r#"fn main() -> Bool [Pure] {
    let c = "hello".char_at(1)
    c == Some("e")
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_char_at_oob() {
    let src = r#"fn main() -> Bool [Pure] {
    let c = "hello".char_at(100)
    match c {
        None => true,
        Some(_) => false,
    }
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

/// String.parse_int returns Ok on success, Err on failure.
#[test]
fn test_string_parse_int_ok() {
    let src = r#"fn main() -> Bool [Pure] {
    "42".parse_int() == Ok(42)
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_parse_int_err() {
    let src = r#"fn main() -> Bool [Pure] {
    match "abc".parse_int() {
        Err(_) => true,
        Ok(_) => false,
    }
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

/// String.parse_float returns Ok on success.
#[test]
fn test_string_parse_float_ok() {
    let src = r#"fn main() -> Bool [Pure] {
    match "3.14".parse_float() {
        Ok(_) => true,
        Err(_) => false,
    }
}"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

/// Numeric .to_int() and .to_float() conversions.
#[test]
fn test_float_to_int() {
    let src = r#"fn main() -> Int [Pure] { (3.9).to_int() }"#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_int_to_float() {
    let src = r#"fn main() -> Bool [Pure] { 42.to_float() == 42.0 }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

/// Numeric .abs() on negative int.
#[test]
fn test_int_abs() {
    let src = r#"fn main() -> Int [Pure] { (-7).abs() }"#;
    assert_eq!(run(src).unwrap(), Value::Int(7));
}

/// Float .floor(), .ceil(), .round().
#[test]
fn test_float_floor() {
    let src = r#"fn main() -> Float [Pure] { (3.9).floor() }"#;
    assert_eq!(run(src).unwrap(), Value::Float(3.0));
}

#[test]
fn test_float_ceil() {
    let src = r#"fn main() -> Float [Pure] { (3.1).ceil() }"#;
    assert_eq!(run(src).unwrap(), Value::Float(4.0));
}

#[test]
fn test_float_round() {
    let src = r#"fn main() -> Float [Pure] { (3.5).round() }"#;
    assert_eq!(run(src).unwrap(), Value::Float(4.0));
}

/// Float .sqrt().
#[test]
fn test_float_sqrt() {
    let src = r#"fn main() -> Float [Pure] { (4.0).sqrt() }"#;
    assert_eq!(run(src).unwrap(), Value::Float(2.0));
}

/// Float .pow().
#[test]
fn test_float_pow() {
    let src = r#"fn main() -> Float [Pure] { (2.0).pow(10) }"#;
    assert_eq!(run(src).unwrap(), Value::Float(1024.0));
}

/// Nested closures: inner closure defined via let is callable within outer body.
#[test]
fn test_nested_closure_typecheck() {
    let src = r#"
fn main() -> Int [Pure] {
    let outer = fn(x) {
        let inner = fn(y) { x + y }
        inner(10)
    }
    outer(5)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(15));
}

/// Empty list literal parses and has length 0.
#[test]
fn test_empty_list_literal() {
    let src = r#"fn main() -> Int [Pure] {
    let xs = []
    List.length(xs)
}"#;
    assert_eq!(run(src).unwrap(), Value::Int(0));
}

/// Integer division (floor division) with negative numbers.
#[test]
fn test_int_div_negative() {
    let src = r#"fn main() -> Int [Pure] { -7 // 2 }"#;
    // div_euclid(-7, 2) == -4 (rounds toward negative infinity)
    assert_eq!(run(src).unwrap(), Value::Int(-4));
}

