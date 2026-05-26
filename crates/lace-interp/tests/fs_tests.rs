// fs_tests.rs — Fs stdlib tests
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

fn tmpdir() -> String {
    let src = r#"fn main() -> String [IO] { Shell.capture("mktemp -d") }"#;
    match run(src).unwrap() {
        Value::String(s) => s.trim().to_string(),
        _ => panic!("mktemp -d failed"),
    }
}

#[test]
fn test_fs_write_and_read() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> String [IO] {{
    Fs.write("{}/hello.txt", "hello world")
    match Fs.read("{}/hello.txt") {{
        Ok(s) => s
        Err(e) => e
    }}
}}
"#, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::String("hello world".into()));
}

#[test]
fn test_fs_exists_true_false() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> Bool [IO] {{
    Fs.write("{}/x.txt", "data")
    Fs.exists("{}/x.txt")
}}
"#, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::Bool(true));

    let src2 = format!(r#"
fn main() -> Bool [IO] {{
    Fs.exists("{}/does_not_exist_9999.txt")
}}
"#, tmp);
    assert_eq!(run(&src2).unwrap(), Value::Bool(false));
}

#[test]
fn test_fs_list_returns_list() {
    let src = r#"
fn main() -> Bool [IO] {
    let files = Fs.list(".")
    List.length(files) >= 0
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_fs_mkdir_and_exists() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> Bool [IO] {{
    Fs.mkdir("{}/a/b/c")
    Fs.exists("{}/a/b/c")
}}
"#, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn test_fs_basename() {
    let src = r#"
fn main() -> String [IO] {
    Fs.basename("dir/sub/file.txt")
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("file.txt".into()));
}

#[test]
fn test_fs_dirname() {
    let src = r#"
fn main() -> String [IO] {
    Fs.dirname("dir/sub/file.txt")
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("dir/sub".into()));
}

#[test]
fn test_fs_extension() {
    let src = r#"
fn main() -> String [IO] {
    Fs.extension("archive.tar.gz")
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("gz".into()));
}

#[test]
fn test_fs_join_two() {
    let src = r#"
fn main() -> String [IO] {
    Fs.join("dir", "file.txt")
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("dir/file.txt".into()));
}

#[test]
fn test_fs_join_three() {
    let src = r#"
fn main() -> String [IO] {
    Fs.join("a", "b", "c.txt")
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("a/b/c.txt".into()));
}

#[test]
fn test_fs_append() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> String [IO] {{
    Fs.write("{}/app.txt", "line1\n")
    Fs.append("{}/app.txt", "line2\n")
    match Fs.read("{}/app.txt") {{
        Ok(s) => s
        Err(e) => e
    }}
}}
"#, tmp, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::String("line1\nline2\n".into()));
}

#[test]
fn test_fs_remove_file() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> Bool [IO] {{
    Fs.write("{}/del.txt", "bye")
    Fs.remove("{}/del.txt")
    Fs.exists("{}/del.txt")
}}
"#, tmp, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::Bool(false));
}

#[test]
fn test_fs_copy() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> String [IO] {{
    Fs.write("{}/src.txt", "copied")
    Fs.copy("{}/src.txt", "{}/dst.txt")
    match Fs.read("{}/dst.txt") {{
        Ok(s) => s
        Err(e) => e
    }}
}}
"#, tmp, tmp, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::String("copied".into()));
}

#[test]
fn test_fs_move() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> Bool [IO] {{
    Fs.write("{}/old.txt", "moved")
    Fs.move("{}/old.txt", "{}/new.txt")
    Fs.exists("{}/new.txt")
}}
"#, tmp, tmp, tmp, tmp);
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn test_fs_cwd() {
    let src = r#"
fn main() -> Bool [IO] {
    let cwd = Fs.cwd()
    Str.len(cwd) > 0
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_fs_stat() {
    let tmp = tmpdir();
    let src = format!(r#"
fn main() -> Bool [IO] {{
    Fs.write("{}/f.txt", "hello")
    let info = Fs.stat("{}/f.txt")
    Map.get(info, "is_file")
}}
"#, tmp, tmp);
    // Map.get returns Option so check for Some(true)
    match run(&src).unwrap() {
        Value::Variant { name, payload } if name == "Some" => {
            assert_eq!(payload[0], Value::Bool(true));
        }
        other => panic!("unexpected: {:?}", other),
    }
}
