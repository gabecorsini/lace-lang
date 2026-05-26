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
fn test_math_sqrt() {
    let src = r#"fn main() -> Float [Pure] { Math.sqrt(4.0) }"#;
    assert_eq!(run(src).unwrap(), Value::Float(2.0));
}

#[test]
fn test_math_pow() {
    let src = r#"fn main() -> Float [Pure] { Math.pow(2.0, 10.0) }"#;
    assert_eq!(run(src).unwrap(), Value::Float(1024.0));
}

#[test]
fn test_math_floor() {
    let src = r#"fn main() -> Int [Pure] { Math.floor(3.7) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_math_round() {
    let src = r#"fn main() -> Int [Pure] { Math.round(3.5) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(4));
}

#[test]
fn test_math_abs() {
    let src = r#"fn main() -> Float [Pure] { Math.abs(-5.0) }"#;
    assert_eq!(run(src).unwrap(), Value::Float(5.0));
}

#[test]
fn test_int_abs() {
    let src = r#"fn main() -> Int [Pure] { Int.abs(-42) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_math_clamp() {
    let src = r#"fn main() -> Float [Pure] { Math.clamp(10.0, 0.0, 5.0) }"#;
    assert_eq!(run(src).unwrap(), Value::Float(5.0));
}

#[test]
fn test_math_pi() {
    let src = r#"fn main() -> Bool [Pure] { Math.pi() > 3.14 }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_math_ceil() {
    let src = r#"fn main() -> Int [Pure] { Math.ceil(3.2) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(4));
}

#[test]
fn test_math_sin() {
    let src = r#"fn main() -> Bool [Pure] { Math.sin(0.0) == 0.0 }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_int_pow() {
    let src = r#"fn main() -> Int [Pure] { Int.pow(2, 8) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(256));
}

#[test]
fn test_int_clamp() {
    let src = r#"fn main() -> Int [Pure] { Int.clamp(10, 0, 5) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_float_is_nan() {
    let src = r#"fn main() -> Bool [Pure] { Float.is_nan(1.0) }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_int_to_float() {
    let src = r#"fn main() -> Float [Pure] { Int.to_float(42) }"#;
    assert_eq!(run(src).unwrap(), Value::Float(42.0));
}

#[test]
fn test_float_to_int() {
    let src = r#"fn main() -> Int [Pure] { Float.to_int(3.9) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}
