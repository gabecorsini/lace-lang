// Phase 34 generics tests
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
fn test_generic_identity_int() {
    let src = r#"
fn identity(x: T) -> T [Pure] { x }
fn main() -> ? [Pure] { identity(42) }
"#;
    let val = run(src).expect("should run");
    assert_eq!(val, Value::Int(42));
}

#[test]
fn test_generic_identity_string() {
    let src = r#"
fn identity(x: T) -> T [Pure] { x }
fn main() -> ? [Pure] { identity("hello") }
"#;
    let val = run(src).expect("should run");
    assert_eq!(val, Value::String("hello".into()));
}

#[test]
fn test_generic_multi_params() {
    let src = r#"
fn pair_first(a: T, b: U) -> T [Pure] { a }
fn main() -> ? [Pure] { pair_first("yes", 99) }
"#;
    let val = run(src).expect("should run");
    assert_eq!(val, Value::String("yes".into()));
}

#[test]
fn test_generic_list_param() {
    let src = r#"
fn wrap(x: T) -> List<T> [Pure] { [x] }
fn main() -> ? [Pure] {
    let lst = wrap(7)
    List.get(lst, 0)
}
"#;
    let val = run(src).expect("should run");
    assert_eq!(val, Value::Variant { name: "Some".into(), payload: vec![Value::Int(7)] });
}
