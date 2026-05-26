// stdlib_io_tests.rs — Json, Shell, Http stdlib tests
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

// ── Json tests ─────────────────────────────────────────────────────────────

#[test]
fn test_json_parse_object() {
    let src = r#"
fn main() -> String [IO] {
    let result = Json.parse("{\"name\": \"Alice\", \"age\": 30}")
    match result {
        Ok(data) => match Map.get(data, "name") {
            Some(v) => v
            None => "missing"
        }
        Err(e) => e
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("Alice".into()));
}

#[test]
fn test_json_parse_int_field() {
    let src = r#"
fn main() -> Int [IO] {
    let result = Json.parse("{\"name\": \"Alice\", \"age\": 30}")
    match result {
        Ok(data) => match Map.get(data, "age") {
            Some(v) => v
            None => 0
        }
        Err(_) => 0
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(30));
}

#[test]
fn test_json_stringify_roundtrip() {
    let src = r#"
fn main() -> Bool [IO] {
    let result = Json.parse("{\"x\": 42}")
    match result {
        Ok(data) => {
            let s = Json.stringify(data)
            let result2 = Json.parse(s)
            match result2 {
                Ok(data2) => match Map.get(data2, "x") {
                    Some(v) => v == 42
                    None => false
                }
                Err(_) => false
            }
        }
        Err(_) => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_pretty() {
    let src = r#"
fn main() -> Bool [IO] {
    let result = Json.parse("{\"x\": 1}")
    match result {
        Ok(data) => {
            let s = Json.pretty(data)
            Str.contains(s, "\n")
        }
        Err(_) => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

// ── Shell tests ────────────────────────────────────────────────────────────

#[test]
fn test_shell_run_stdout() {
    let src = r#"
fn main() -> Bool [IO] {
    let result = Shell.run("echo hello")
    let stdout = Map.get(result, "stdout")
    match stdout {
        Some(s) => Str.contains(s, "hello")
        None => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_shell_run_exit_code() {
    let src = r#"
fn main() -> Bool [IO] {
    let result = Shell.run("true")
    let code = Map.get(result, "exit_code")
    match code {
        Some(c) => c == 0
        None => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_shell_success_true() {
    let src = r#"
fn main() -> Bool [IO] {
    Shell.success("true")
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_shell_success_false() {
    let src = r#"
fn main() -> Bool [IO] {
    Shell.success("false")
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_shell_capture() {
    let src = r#"
fn main() -> String [IO] {
    Shell.capture("echo hello")
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("hello".into()));
}

#[test]
fn test_shell_env_path() {
    let src = r#"
fn main() -> Bool [IO] {
    let v = Shell.env("PATH")
    match v {
        Some(_) => true
        None => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_shell_set_env() {
    let src = r#"
fn main() -> Bool [IO] {
    Shell.set_env("LACE_TEST_VAR", "hello_lace")
    let v = Shell.env("LACE_TEST_VAR")
    match v {
        Some(s) => s == "hello_lace"
        None => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

// ── HTTP tests (ignored in CI — require network) ───────────────────────────

#[test]
#[ignore]
fn test_http_get() {
    let src = r#"
fn main() -> Bool [Http] {
    let resp = Http.get("https://httpbin.org/get")
    match resp {
        Ok(body) => Str.len(body) > 0
        Err(_) => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
#[ignore]
fn test_http_get_json() {
    let src = r#"
fn main() -> Bool [Http] {
    let data = Http.get_json("https://httpbin.org/json")
    match Map.get(data, "error") {
        Some(_) => false
        None => true
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
#[ignore]
fn test_http_post() {
    let src = r#"
fn main() -> Bool [Http] {
    let resp = Http.post("https://httpbin.org/post", "{\"key\": \"value\"}")
    match resp {
        Ok(body) => Str.contains(body, "key")
        Err(_) => false
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
