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
fn test_map_literal_basic() {
    let src = r#"
fn main() -> Int [Pure] {
    let m = {"a": 1, "b": 2}
    Map.length(m)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn test_map_literal_empty() {
    let src = r#"
fn main() -> Int [Pure] {
    let m = {}
    Map.length(m)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(0));
}

#[test]
fn test_map_literal_get() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Map.get({"x": 42}, "x")
    match r {
        Some(v) => v == 42,
        None => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_map_literal_string_value() {
    let src = r#"
fn main() -> Bool [Pure] {
    let m = {"k": "v"}
    let r = Map.get(m, "k")
    match r {
        Some(v) => v == "v",
        None => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_map_literal_nested() {
    let src = r#"
fn main() -> Int [Pure] {
    let outer = {"user": {"id": 1}}
    let r = Map.get(outer, "user")
    match r {
        Some(inner) => {
            let id_r = Map.get(inner, "id")
            match id_r {
                Some(id) => id,
                None => -1,
            }
        },
        None => -1,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_map_literal_json_stringify() {
    // Json.stringify on a map literal should produce valid JSON that can be re-parsed
    let src = r#"
fn main() -> Bool [Pure] {
    let m = {"name": "Alice"}
    let s = Json.stringify(m)
    let r = Json.parse(s)
    r.is_ok()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
