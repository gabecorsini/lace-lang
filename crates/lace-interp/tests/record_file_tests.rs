// Phase 10 – record types and file I/O stdlib
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
fn test_record_field_access() {
    let src = r#"
record Point {
    x: Int,
    y: Int,
}
fn main() -> Int [Pure] {
    let p = Point { x: 3, y: 4, }
    p.x + p.y
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(7));
}

#[test]
fn test_record_in_function_return() {
    let src = r#"
record Pair {
    a: Int,
    b: Int,
}
fn make(x: Int) -> Pair [Pure] {
    Pair { a: x, b: x * 2, }
}
fn main() -> Int [Pure] {
    let p = make(5)
    p.b
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_record_nested_fields() {
    let src = r#"
record Inner {
    v: Int,
}
record Outer {
    inner: Inner,
}
fn main() -> Int [Pure] {
    let i = Inner { v: 99, }
    let o = Outer { inner: i, }
    o.inner.v
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(99));
}

#[test]
fn test_file_write_and_read() {
    let path = "/tmp/lace_phase10_test.txt";
    let src = format!(r#"
fn main() -> String [IO] {{
    let _ = File.write("{path}", "hello from lace")
    let r = File.read("{path}")
    match r {{
        Ok(s) => s,
        Err(e) => "error",
    }}
}}
"#);
    assert_eq!(run(&src).unwrap(), Value::String("hello from lace".into()));
}

#[test]
fn test_file_exists_after_write() {
    let path = "/tmp/lace_phase10_exists_test.txt";
    let src = format!(r#"
fn main() -> Bool [IO] {{
    let _ = File.write("{path}", "data")
    File.exists("{path}")
}}
"#);
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn test_file_exists_false_for_missing() {
    let src = r#"
fn main() -> Bool [IO] {
    File.exists("/tmp/lace_phase10_no_such_file_xyz_999.txt")
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_file_read_err_on_missing() {
    let src = r#"
fn main() -> Unit [IO] {
    let r = File.read("/tmp/lace_no_such_file_abc_123.txt")
    assert_err(r, "expected error for missing file")
}
"#;
    assert!(run(src).is_ok());
}
