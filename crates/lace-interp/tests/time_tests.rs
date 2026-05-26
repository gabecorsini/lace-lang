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
fn test_time_now_returns_float() {
    let src = r#"fn main() -> Float [Pure] { Time.now() }"#;
    let val = run(src).unwrap();
    match val {
        Value::Float(f) => assert!(f > 1_000_000_000.0, "expected timestamp > 1e9, got {}", f),
        _ => panic!("expected Float, got {:?}", val),
    }
}

#[test]
fn test_time_now_ms_returns_int() {
    let src = r#"fn main() -> Int [Pure] { Time.now_ms() }"#;
    let val = run(src).unwrap();
    match val {
        Value::Int(i) => assert!(i > 1_000_000_000_000, "expected ms > 1e12, got {}", i),
        _ => panic!("expected Int, got {:?}", val),
    }
}

#[test]
fn test_time_format_year() {
    let src = r#"fn main() -> String [Pure] { Time.format(Time.now(), "%Y") }"#;
    let val = run(src).unwrap();
    match val {
        Value::String(s) => assert!(s.starts_with("20"), "expected year starting with 20, got {}", s),
        _ => panic!("expected String, got {:?}", val),
    }
}

#[test]
fn test_time_parse_valid() {
    let src = r#"fn main() -> Bool [Pure] {
        let r = Time.parse("2024-01-15 00:00:00", "%Y-%m-%d %H:%M:%S")
        match r {
            Some(_) => true,
            None => false,
        }
    }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_time_parse_invalid() {
    let src = r#"fn main() -> Bool [Pure] {
        let r = Time.parse("not-a-date", "%Y-%m-%d")
        match r {
            Some(_) => false,
            None => true,
        }
    }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_args_count() {
    let src = r#"fn main() -> Bool [Pure] { Args.count() >= 1 }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_args_program() {
    let src = r#"fn main() -> Bool [Pure] { Str.len(Args.program()) > 0 }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
