// Phase 35 tests: Http.response constructor and Http.get_header
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
fn test_http_response_status() {
    let src = r#"
fn main() -> Dynamic [Pure] {
    let resp = Http.response(200, "text/plain", "hello")
    Map.get(resp, "status")
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Some".into(),
            payload: vec![Value::Int(200)],
        }
    );
}

#[test]
fn test_http_response_body_and_content_type() {
    let src = r#"
fn main() -> Dynamic [Pure] {
    let resp = Http.response(201, "application/json", "ok")
    let b = Map.get(resp, "body")
    let ct = Map.get(resp, "content_type")
    assert_eq(b, Some("ok"))
    assert_eq(ct, Some("application/json"))
    Map.get(resp, "status")
}
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::Variant {
            name: "Some".into(),
            payload: vec![Value::Int(201)],
        }
    );
}
