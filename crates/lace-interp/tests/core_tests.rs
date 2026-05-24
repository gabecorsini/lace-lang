// Phase 6–8 core interpreter tests
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
fn test_arithmetic() {
    let src = r#"
fn main() -> Int [Pure] {
    let x = 2 + 3 * 4
    x
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(14));
}

#[test]
fn test_string_concat() {
    let src = r#"
fn main() -> String [Pure] {
    "hello" ++ " " ++ "world"
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("hello world".into()));
}

#[test]
fn test_list_range_length() {
    let src = r#"
fn main() -> Int [Pure] {
    let xs = List.range(0, 5)
    List.length(xs)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_list_map_output() {
    let src = r#"
fn double(x: Int) -> Int [Pure] { x * 2 }

fn main() -> Int [Pure] {
    let xs = List.range(0, 3)
    let ys = List.map(xs, double)
    List.length(ys)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(3));
}

#[test]
fn test_for_loop_side_effect_counter() {
    let src = r#"
fn main() -> Int [Mut] {
    mut let count = 0
    for i in List.range(0, 5) {
        count = count + 1
    }
    count
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_assert_err_on_err_value() {
    let src = r#"
fn main() -> Unit [Pure] {
    let r = Err("oops")
    assert_err(r, "expected error")
}
"#;
    assert!(run(src).is_ok());
}
