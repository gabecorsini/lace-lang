// Tests for ? (error propagation) operator and partial application
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

// ---- ? operator tests ----

#[test]
fn test_try_ok_unwraps() {
    let src = r#"
fn safe_div(a: Int, b: Int) -> Result<Int, String> [Pure] {
    if b == 0 {
        Err("division by zero")
    } else {
        Ok(a / b)
    }
}

fn main() -> Result<Int, String> [Pure] {
    let a = safe_div(10, 2)?
    Ok(a)
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Ok".into(),
            payload: vec![Value::Int(5)],
        }
    );
}

#[test]
fn test_try_err_propagates() {
    let src = r#"
fn safe_div(a: Int, b: Int) -> Result<Int, String> [Pure] {
    if b == 0 {
        Err("division by zero")
    } else {
        Ok(a / b)
    }
}

fn compute(x: Int) -> Result<Int, String> [Pure] {
    let a = safe_div(x, 2)?
    let b = safe_div(a, 0)?
    Ok(a + b)
}

fn main() -> Result<Int, String> [Pure] {
    compute(10)
}
"#;
    let result = run(src).unwrap();
    match result {
        Value::Variant { name, payload } => {
            assert_eq!(name, "Err");
            assert_eq!(payload.len(), 1);
            assert_eq!(payload[0], Value::String("division by zero".into()));
        }
        other => panic!("expected Err variant, got {:?}", other),
    }
}

#[test]
fn test_try_some_unwraps() {
    let src = r#"
fn find_first(xs: List<Int>) -> Option<Int> [Pure] {
    List.find(xs, fn(x) { x > 0 })
}

fn main() -> Option<Int> [Pure] {
    let v = find_first([1, 2, 3])?
    Some(v * 2)
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Some".into(),
            payload: vec![Value::Int(2)],
        }
    );
}

#[test]
fn test_try_none_propagates() {
    let src = r#"
fn find_first(xs: List<Int>) -> Option<Int> [Pure] {
    List.find(xs, fn(x) { x > 100 })
}

fn main() -> Option<Int> [Pure] {
    let v = find_first([1, 2, 3])?
    Some(v * 2)
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "None".into(),
            payload: vec![],
        }
    );
}

// ---- partial application tests ----

#[test]
fn test_partial_application_first_arg() {
    let src = r#"
fn add(a: Int, b: Int) -> Int [Pure] {
    a + b
}

fn main() -> Int [Pure] {
    let add5 = add(_, 5)
    add5(3)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(8));
}

#[test]
fn test_partial_application_second_arg() {
    let src = r#"
fn add(a: Int, b: Int) -> Int [Pure] {
    a + b
}

fn main() -> Int [Pure] {
    let add3 = add(3, _)
    add3(7)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}
