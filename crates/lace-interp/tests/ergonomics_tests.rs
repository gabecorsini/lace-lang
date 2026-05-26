// Ergonomics tests: pipeline |>, Result/Option combinators
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

// --- Pipeline tests ---

#[test]
fn test_pipeline_list_reverse() {
    let src = r#"
fn main() -> List [Pure] {
    [1, 2, 3] |> List.reverse()
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::List(vec![Value::Int(3), Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn test_pipeline_filter_map() {
    let src = r#"
fn main() -> List [Pure] {
    [1, 2, 3, 4, 5]
        |> List.filter(fn(x) { x > 2 })
        |> List.map(fn(x) { x * 2 })
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::List(vec![Value::Int(6), Value::Int(8), Value::Int(10)])
    );
}

// --- Result/Option combinator tests ---

#[test]
fn test_ok_map() {
    let src = r#"
fn main() -> Result [Pure] {
    Ok(5).map(fn(x) { x * 2 })
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Ok".into(),
            payload: vec![Value::Int(10)]
        }
    );
}

#[test]
fn test_err_unwrap_or() {
    let src = r#"
fn main() -> Int [Pure] {
    Err("x").unwrap_or(99)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(99));
}

#[test]
fn test_some_is_some() {
    let src = r#"
fn main() -> Bool [Pure] {
    Some(1).is_some()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_none_unwrap_or() {
    let src = r#"
fn main() -> Int [Pure] {
    None().unwrap_or(0)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(0));
}

#[test]
fn test_ok_is_ok() {
    let src = r#"fn main() -> Bool [Pure] { Ok(42).is_ok() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_err_is_err() {
    let src = r#"fn main() -> Bool [Pure] { Err("oops").is_err() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_err_map_passthrough() {
    let src = r#"
fn main() -> Result [Pure] {
    Err("oops").map(fn(x) { x * 2 })
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Err".into(),
            payload: vec![Value::String("oops".into())]
        }
    );
}

#[test]
fn test_some_map() {
    let src = r#"
fn main() -> Option [Pure] {
    Some(5).map(fn(x) { x + 1 })
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Some".into(),
            payload: vec![Value::Int(6)]
        }
    );
}

#[test]
fn test_none_map_passthrough() {
    let src = r#"
fn main() -> Option [Pure] {
    None().map(fn(x) { x + 1 })
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "None".into(),
            payload: vec![]
        }
    );
}

#[test]
fn test_none_is_none() {
    let src = r#"fn main() -> Bool [Pure] { None().is_none() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_ok_and_then() {
    let src = r#"
fn main() -> Result [Pure] {
    Ok(42).and_then(fn(x) { Ok(x + 1) })
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Ok".into(),
            payload: vec![Value::Int(43)]
        }
    );
}

#[test]
fn test_some_unwrap_or() {
    let src = r#"fn main() -> Int [Pure] { Some(5).unwrap_or(0) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}
