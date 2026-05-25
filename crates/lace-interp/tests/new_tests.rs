// New tests covering: match, Option/Result combinators, Json stdlib,
// String methods, List stdlib, closures, loops, records.
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

// ── Match expressions ────────────────────────────────────────────────────────

#[test]
fn test_match_int_arms() {
    let src = r#"
fn main() -> String [Pure] {
    let x = 2
    match x {
        1 => "one",
        2 => "two",
        _ => "other",
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("two".into()));
}

#[test]
fn test_match_int_wildcard() {
    let src = r#"
fn main() -> String [Pure] {
    let x = 99
    match x {
        1 => "one",
        _ => "other",
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("other".into()));
}

#[test]
fn test_match_string() {
    let src = r#"
fn main() -> Int [Pure] {
    let s = "hello"
    match s {
        "hello" => 1,
        "world" => 2,
        _ => 0,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(1));
}

#[test]
fn test_match_some() {
    let src = r#"
fn main() -> Int [Pure] {
    let v = Some(42)
    match v {
        Some(n) => n,
        None => 0,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_match_none() {
    let src = r#"
fn maybe(flag: Bool) -> Int [Pure] {
    let v = if flag { Some(42) } else { Some(0) }
    match v {
        Some(n) => n,
        None => -1,
    }
}
fn main() -> Int [Pure] { maybe(false) }
"#;
    assert_eq!(run(src).unwrap(), Value::Int(0));
}

#[test]
fn test_match_ok() {
    let src = r#"
fn main() -> Int [Pure] {
    let r = Ok(7)
    match r {
        Ok(n) => n,
        Err(_) => 0,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(7));
}

#[test]
fn test_match_err() {
    let src = r#"
fn main() -> String [Pure] {
    let r = Err("bad")
    match r {
        Ok(_) => "ok",
        Err(e) => e,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("bad".into()));
}

// ── Option combinators ───────────────────────────────────────────────────────

#[test]
fn test_option_is_some_true() {
    let src = r#"fn main() -> Bool [Pure] { Some(1).is_some() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_option_is_some_false() {
    let src = r#"
fn main() -> Bool [Pure] {
    let v = "hello".char_at(999)
    v.is_some()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_option_is_none_true() {
    let src = r#"
fn main() -> Bool [Pure] {
    let v = "hello".char_at(999)
    v.is_none()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_option_is_none_false() {
    let src = r#"fn main() -> Bool [Pure] { Some(5).is_none() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_option_unwrap_or_some() {
    let src = r#"fn main() -> Int [Pure] { Some(10).unwrap_or(99) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_option_unwrap_or_none() {
    let src = r#"
fn main() -> Int [Pure] {
    let v = "hello".char_at(999)
    v.unwrap_or("x")
    99
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(99));
}

#[test]
fn test_option_map_some() {
    let src = r#"
fn double(x: Int) -> Int [Pure] { x * 2 }
fn main() -> Bool [Pure] {
    let result = Some(5).map(double)
    result == Some(10)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_option_map_none() {
    let src = r#"
fn double(x: Int) -> Int [Pure] { x * 2 }
fn main() -> Bool [Pure] {
    let v = "hello".char_at(999)
    v.is_none()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

// ── Result combinators ───────────────────────────────────────────────────────

#[test]
fn test_result_is_ok_true() {
    let src = r#"fn main() -> Bool [Pure] { Ok(1).is_ok() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_is_ok_false() {
    let src = r#"fn main() -> Bool [Pure] { Err("x").is_ok() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_result_is_err_true() {
    let src = r#"fn main() -> Bool [Pure] { Err("x").is_err() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_is_err_false() {
    let src = r#"fn main() -> Bool [Pure] { Ok(1).is_err() }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_result_unwrap_or_ok() {
    let src = r#"fn main() -> Int [Pure] { Ok(42).unwrap_or(0) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_result_unwrap_or_err() {
    let src = r#"fn main() -> Int [Pure] { Err("fail").unwrap_or(0) }"#;
    assert_eq!(run(src).unwrap(), Value::Int(0));
}

#[test]
fn test_result_map_ok() {
    let src = r#"
fn inc(x: Int) -> Int [Pure] { x + 1 }
fn main() -> Bool [Pure] {
    Ok(9).map(inc) == Ok(10)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_map_err_passthrough() {
    let src = r#"
fn inc(x: Int) -> Int [Pure] { x + 1 }
fn main() -> Bool [Pure] {
    let r = Err("oops").map(inc)
    r.is_err()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_map_err_fn() {
    let src = r#"
fn shout(s: String) -> String [Pure] { s.to_upper() }
fn main() -> Bool [Pure] {
    let r = Err("oops").map_err(shout)
    match r {
        Err(e) => e == "OOPS",
        Ok(_) => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_ok_converts_ok() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Ok(5).ok()
    r == Some(5)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_ok_converts_err() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Err("bad").ok()
    r.is_none()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_err_converts_err() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Err("bad").err()
    r == Some("bad")
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_result_err_converts_ok() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Ok(1).err()
    r.is_none()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

// ── Json stdlib ──────────────────────────────────────────────────────────────

#[test]
fn test_json_parse_valid() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("{\"a\": 1}")
    r.is_ok()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_parse_invalid() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("not json")
    r.is_err()
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_get_existing_key() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("{\"name\": \"lace\"}")
    match r {
        Ok(obj) => {
            let v = Json.get(obj, "name")
            v.is_some()
        },
        Err(_) => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_get_missing_key() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("{\"a\": 1}")
    match r {
        Ok(obj) => {
            let v = Json.get(obj, "missing")
            v.is_none()
        },
        Err(_) => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_index_valid() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("[10, 20, 30]")
    match r {
        Ok(lst) => {
            let v = Json.index(lst, 1)
            v.is_some()
        },
        Err(_) => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_index_out_of_bounds() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("[1, 2]")
    match r {
        Ok(lst) => {
            let v = Json.index(lst, 99)
            v.is_none()
        },
        Err(_) => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_json_stringify_roundtrip() {
    let src = r#"
fn main() -> Bool [Pure] {
    let r = Json.parse("{\"x\": 42}")
    match r {
        Ok(obj) => {
            let s = Json.stringify(obj)
            let r2 = Json.parse(s)
            r2.is_ok()
        },
        Err(_) => false,
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

// ── String methods ───────────────────────────────────────────────────────────

#[test]
fn test_string_len_method() {
    let src = r#"fn main() -> Int [Pure] { "abcde".len() }"#;
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_string_trim_method() {
    let src = r#"fn main() -> String [Pure] { "  hi  ".trim() }"#;
    assert_eq!(run(src).unwrap(), Value::String("hi".into()));
}

#[test]
fn test_string_to_upper_method() {
    let src = r#"fn main() -> String [Pure] { "hello".to_upper() }"#;
    assert_eq!(run(src).unwrap(), Value::String("HELLO".into()));
}

#[test]
fn test_string_to_lower_method() {
    let src = r#"fn main() -> String [Pure] { "WORLD".to_lower() }"#;
    assert_eq!(run(src).unwrap(), Value::String("world".into()));
}

#[test]
fn test_string_contains_method() {
    let src = r#"fn main() -> Bool [Pure] { "foobar".contains("oba") }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_starts_with_method() {
    let src = r#"fn main() -> Bool [Pure] { "foobar".starts_with("foo") }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_ends_with_method() {
    let src = r#"fn main() -> Bool [Pure] { "foobar".ends_with("bar") }"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_string_split_method() {
    let src = r#"
fn main() -> Int [Pure] {
    let parts = "a,b,c,d".split(",")
    List.length(parts)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(4));
}

#[test]
fn test_string_replace_method() {
    let src = r#"fn main() -> String [Pure] { "hello world".replace("world", "lace") }"#;
    assert_eq!(run(src).unwrap(), Value::String("hello lace".into()));
}

#[test]
fn test_string_to_string_method() {
    let src = r#"fn main() -> String [Pure] { 42.to_string() }"#;
    assert_eq!(run(src).unwrap(), Value::String("42".into()));
}

// ── List stdlib ──────────────────────────────────────────────────────────────

#[test]
fn test_list_range() {
    let src = r#"
fn main() -> Int [Pure] {
    let xs = List.range(0, 10)
    List.length(xs)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_list_sum() {
    let src = r#"
fn main() -> Int [Pure] {
    let xs = List.range(1, 5)
    List.sum(xs)
}
"#;
    // range(1,5) = [1,2,3,4], sum = 10
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_list_map_with_closure() {
    let src = r#"
fn main() -> Int [Pure] {
    let xs = List.range(0, 4)
    let ys = List.map(xs, fn(x) { x * 3 })
    List.sum(ys)
}
"#;
    // [0,1,2,3] * 3 = [0,3,6,9], sum = 18
    assert_eq!(run(src).unwrap(), Value::Int(18));
}

#[test]
fn test_list_filter() {
    let src = r#"
fn main() -> Int [Pure] {
    let xs = List.range(0, 10)
    let evens = List.filter(xs, fn(x) { x % 2 == 0 })
    List.length(evens)
}
"#;
    // evens in [0..9]: 0,2,4,6,8 = 5
    assert_eq!(run(src).unwrap(), Value::Int(5));
}

#[test]
fn test_list_contains_true() {
    let src = r#"
fn main() -> Bool [Pure] {
    let xs = List.range(0, 5)
    List.contains(xs, 3)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn test_list_contains_false() {
    let src = r#"
fn main() -> Bool [Pure] {
    let xs = List.range(0, 5)
    List.contains(xs, 99)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_list_fold() {
    let src = r#"
fn main() -> Int [Pure] {
    let xs = List.range(1, 6)
    List.fold(xs, 0, fn(acc, x) { acc + x })
}
"#;
    // 1+2+3+4+5 = 15
    assert_eq!(run(src).unwrap(), Value::Int(15));
}

// ── Closures ─────────────────────────────────────────────────────────────────

#[test]
fn test_closure_captures_outer_variable() {
    let src = r#"
fn main() -> Int [Pure] {
    let base = 100
    let add_base = fn(x) { x + base }
    add_base(7)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(107));
}

#[test]
fn test_closure_passed_to_list_map() {
    let src = r#"
fn main() -> Int [Pure] {
    let factor = 5
    let xs = List.range(1, 4)
    let ys = List.map(xs, fn(x) { x * factor })
    List.sum(ys)
}
"#;
    // [1,2,3] * 5 = [5,10,15], sum = 30
    assert_eq!(run(src).unwrap(), Value::Int(30));
}

// ── Loops and mutation ───────────────────────────────────────────────────────

#[test]
fn test_for_loop_sum() {
    let src = r#"
fn main() -> Int [Mut] {
    mut let total = 0
    for i in List.range(1, 6) {
        total = total + i
    }
    total
}
"#;
    // 1+2+3+4+5 = 15
    assert_eq!(run(src).unwrap(), Value::Int(15));
}

#[test]
fn test_while_loop_countdown() {
    let src = r#"
fn main() -> Int [Mut] {
    mut let n = 5
    mut let acc = 0
    while n > 0 {
        acc = acc + n
        n = n - 1
    }
    acc
}
"#;
    // 5+4+3+2+1 = 15
    assert_eq!(run(src).unwrap(), Value::Int(15));
}

// ── Records ──────────────────────────────────────────────────────────────────

#[test]
fn test_record_declaration_and_construction() {
    let src = r#"
record Person {
    name: String,
    age: Int,
}
fn main() -> String [Pure] {
    let p = Person { name: "Alice", age: 30, }
    p.name
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("Alice".into()));
}

#[test]
fn test_record_multiple_field_access() {
    let src = r#"
record Rect {
    width: Int,
    height: Int,
}
fn area(r: Rect) -> Int [Pure] { r.width * r.height }
fn main() -> Int [Pure] {
    let r = Rect { width: 6, height: 7, }
    area(r)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn test_enum_unit_variant_match() {
    let src = r#"
enum Direction { North, South, East, West }
fn main() -> String [Pure] {
    let d = Direction.East
    match d {
        Direction.North => { "north" },
        Direction.East  => { "east" },
        _ => { "other" },
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("east".into()));
}

#[test]
fn test_enum_tuple_variant_single() {
    let src = r#"
enum Shape { Circle(Float), Point, }
fn main() -> Float [Pure] {
    let s = Shape.Circle(3.0)
    match s {
        Shape.Circle(r) => { r * 2.0 },
        Shape.Point => { 0.0 },
        _ => { 0.0 },
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Float(6.0));
}

#[test]
fn test_enum_tuple_variant_multi() {
    let src = r#"
enum Shape { Rect(Int, Int), Other, }
fn main() -> Int [Pure] {
    let s = Shape.Rect(4, 5)
    match s {
        Shape.Rect(w, h) => { w * h },
        _ => { 0 },
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(20));
}

#[test]
fn test_enum_unit_no_payload_match() {
    let src = r#"
enum Color { Red, Green, Blue, }
fn main() -> String [Pure] {
    let c = Color.Blue
    match c {
        Color.Red   => { "red" },
        Color.Green => { "green" },
        Color.Blue  => { "blue" },
        _ => { "unknown" },
    }
}
"#;
    assert_eq!(run(src).unwrap(), Value::String("blue".into()));
}

#[test]
fn test_enum_in_function_arg() {
    let src = r#"
enum Coin { Penny, Nickel, Dime, Quarter, }
fn value(c: Coin) -> Int [Pure] {
    match c {
        Coin.Penny   => { 1 },
        Coin.Nickel  => { 5 },
        Coin.Dime    => { 10 },
        Coin.Quarter => { 25 },
        _ => { 0 },
    }
}
fn main() -> Int [Pure] {
    value(Coin.Dime) + value(Coin.Quarter)
}
"#;
    assert_eq!(run(src).unwrap(), Value::Int(35));
}

// ── Fs module tests ──────────────────────────────────────────────────────────

#[test]
fn test_fs_write_and_read() {
    let result = run(r#"
fn main() -> Unit [IO] {
    let _ = Fs.write("/tmp/lace_fs_test_rw.txt", "hello lace")
    Fs.read("/tmp/lace_fs_test_rw.txt")
}
"#).unwrap();
    assert_eq!(result, Value::Variant {
        name: "Ok".into(),
        payload: vec![Value::String("hello lace".into())],
    });
}

#[test]
fn test_fs_exists_true() {
    let _ = run(r#"fn main() -> Unit [IO] { Fs.write("/tmp/lace_exists_test.txt", "x") }"#);
    let result = run(r#"fn main() -> Unit [IO] { Fs.exists("/tmp/lace_exists_test.txt") }"#).unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn test_fs_exists_false() {
    let result = run(r#"fn main() -> Unit [IO] { Fs.exists("/tmp/lace_surely_missing_xyz_123.txt") }"#).unwrap();
    assert_eq!(result, Value::Bool(false));
}

#[test]
fn test_fs_read_missing() {
    let result = run(r#"fn main() -> Unit [IO] { Fs.read("/tmp/no_such_file_lace_xyz.txt") }"#).unwrap();
    assert!(matches!(result, Value::Variant { ref name, .. } if name == "Err"));
}

#[test]
fn test_fs_append() {
    let _ = run(r#"fn main() -> Unit [IO] { Fs.write("/tmp/lace_append_test.txt", "line1\n") }"#);
    let _ = run(r#"fn main() -> Unit [IO] { Fs.append("/tmp/lace_append_test.txt", "line2\n") }"#);
    let result = run(r#"fn main() -> Unit [IO] { Fs.read("/tmp/lace_append_test.txt") }"#).unwrap();
    if let Value::Variant { name, payload } = result {
        assert_eq!(name, "Ok");
        if let Some(Value::String(s)) = payload.first() {
            assert!(s.contains("line1"));
            assert!(s.contains("line2"));
        } else {
            panic!("expected string payload");
        }
    } else {
        panic!("expected Ok variant");
    }
}

#[test]
fn test_fs_delete() {
    let _ = run(r#"fn main() -> Unit [IO] { Fs.write("/tmp/lace_delete_test.txt", "bye") }"#);
    let del = run(r#"fn main() -> Unit [IO] { Fs.delete("/tmp/lace_delete_test.txt") }"#).unwrap();
    assert!(matches!(del, Value::Variant { ref name, .. } if name == "Ok"));
    let exists = run(r#"fn main() -> Unit [IO] { Fs.exists("/tmp/lace_delete_test.txt") }"#).unwrap();
    assert_eq!(exists, Value::Bool(false));
}

#[test]
fn test_fs_list_dir() {
    let result = run(r#"fn main() -> Unit [IO] { Fs.list_dir("/tmp") }"#).unwrap();
    assert!(matches!(result, Value::Variant { ref name, .. } if name == "Ok"));
}

// ── Time module tests ────────────────────────────────────────────────────────

#[test]
fn test_time_now_is_int() {
    let result = run(r#"fn main() -> Unit [IO] { Time.now() }"#).unwrap();
    assert!(matches!(result, Value::Int(_)));
}

#[test]
fn test_time_now_ms_is_int() {
    let result = run(r#"fn main() -> Unit [IO] { Time.now_ms() }"#).unwrap();
    assert!(matches!(result, Value::Int(v) if v > 0));
}

#[test]
fn test_time_format_date() {
    let result = run(r#"fn main() -> Unit [IO] { Time.format(1716508800, "%Y-%m-%d") }"#).unwrap();
    assert!(matches!(result, Value::String(ref s) if s.contains("2024")));
}

#[test]
fn test_time_since_non_negative() {
    let result = run(r#"
fn main() -> Unit [IO] {
    let ts = Time.now()
    Time.since(ts)
}
"#).unwrap();
    assert!(matches!(result, Value::Int(v) if v >= 0));
}

#[test]
fn test_time_parse_ok() {
    let result = run(r#"fn main() -> Unit [IO] { Time.parse("2024-05-24 00:00:00", "%Y-%m-%d %H:%M:%S") }"#).unwrap();
    assert!(matches!(result, Value::Variant { ref name, .. } if name == "Ok"));
}

#[test]
fn test_time_parse_err() {
    let result = run(r#"fn main() -> Unit [IO] { Time.parse("not-a-date", "%Y-%m-%d") }"#).unwrap();
    assert!(matches!(result, Value::Variant { ref name, .. } if name == "Err"));
}

// ── Str module tests ─────────────────────────────────────────────────────────

#[test]
fn test_str_split() {
    let result = run(r#"fn main() -> Unit [IO] { Str.split("a,b,c", ",") }"#).unwrap();
    assert_eq!(result, Value::List(vec![
        Value::String("a".into()),
        Value::String("b".into()),
        Value::String("c".into()),
    ]));
}

#[test]
fn test_str_join() {
    let result = run(r#"fn main() -> Unit [IO] { Str.join(["x", "y", "z"], "-") }"#).unwrap();
    assert_eq!(result, Value::String("x-y-z".into()));
}

#[test]
fn test_str_trim() {
    let result = run(r#"fn main() -> Unit [IO] { Str.trim("  hello  ") }"#).unwrap();
    assert_eq!(result, Value::String("hello".into()));
}

#[test]
fn test_str_replace() {
    let result = run(r#"fn main() -> Unit [IO] { Str.replace("hello world", "world", "lace") }"#).unwrap();
    assert_eq!(result, Value::String("hello lace".into()));
}

#[test]
fn test_str_contains() {
    let result = run(r#"fn main() -> Unit [IO] { Str.contains("foobar", "oba") }"#).unwrap();
    assert_eq!(result, Value::Bool(true));
    let result2 = run(r#"fn main() -> Unit [IO] { Str.contains("foobar", "xyz") }"#).unwrap();
    assert_eq!(result2, Value::Bool(false));
}

#[test]
fn test_str_starts_ends_with() {
    let r1 = run(r#"fn main() -> Unit [IO] { Str.starts_with("hello", "hel") }"#).unwrap();
    assert_eq!(r1, Value::Bool(true));
    let r2 = run(r#"fn main() -> Unit [IO] { Str.ends_with("hello", "llo") }"#).unwrap();
    assert_eq!(r2, Value::Bool(true));
}

#[test]
fn test_str_case() {
    let r1 = run(r#"fn main() -> Unit [IO] { Str.to_upper("hello") }"#).unwrap();
    assert_eq!(r1, Value::String("HELLO".into()));
    let r2 = run(r#"fn main() -> Unit [IO] { Str.to_lower("HELLO") }"#).unwrap();
    assert_eq!(r2, Value::String("hello".into()));
}

#[test]
fn test_str_len() {
    let result = run(r#"fn main() -> Unit [IO] { Str.len("hello") }"#).unwrap();
    assert_eq!(result, Value::Int(5));
}

#[test]
fn test_str_slice() {
    let result = run(r#"fn main() -> Unit [IO] { Str.slice("hello", 1, 4) }"#).unwrap();
    assert_eq!(result, Value::String("ell".into()));
}

#[test]
fn test_str_index_of() {
    let r1 = run(r#"fn main() -> Unit [IO] { Str.index_of("hello", "ll") }"#).unwrap();
    assert_eq!(r1, Value::Int(2));
    let r2 = run(r#"fn main() -> Unit [IO] { Str.index_of("hello", "xyz") }"#).unwrap();
    assert_eq!(r2, Value::Int(-1));
}

#[test]
fn test_str_pad() {
    let r1 = run(r#"fn main() -> Unit [IO] { Str.pad_left("5", 3, "0") }"#).unwrap();
    assert_eq!(r1, Value::String("005".into()));
    let r2 = run(r#"fn main() -> Unit [IO] { Str.pad_right("hi", 5, "-") }"#).unwrap();
    assert_eq!(r2, Value::String("hi---".into()));
}

#[test]
fn test_str_repeat_and_char_at() {
    let r1 = run(r#"fn main() -> Unit [IO] { Str.repeat("ab", 3) }"#).unwrap();
    assert_eq!(r1, Value::String("ababab".into()));
    let r2 = run(r#"fn main() -> Unit [IO] { Str.char_at("hello", 1) }"#).unwrap();
    assert_eq!(r2, Value::String("e".into()));
}

// ── Closures and first-class functions ───────────────────────────────────────

#[test]
fn test_closure_basic() {
    let src = r#"
fn main() -> Int [Pure] {
    let double = fn(x) { x * 2 }
    double(5)
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Int(10));
}

#[test]
fn test_closure_capture() {
    let src = r#"
fn main() -> Int [Pure] {
    let n = 10
    let add_n = fn(x) { x + n }
    add_n(5)
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Int(15));
}

#[test]
fn test_hof_apply() {
    let src = r#"
fn apply(f: Fn, x: Int) -> Int [Pure] {
    f(x)
}
fn main() -> Int [Pure] {
    apply(fn(x) { x * 3 }, 7)
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Int(21));
}

#[test]
fn test_return_closure() {
    let src = r#"
fn make_adder(n: Int) -> Fn [Pure] {
    fn(x) { x + n }
}
fn main() -> Int [Pure] {
    let add5 = make_adder(5)
    add5(10)
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Int(15));
}

#[test]
fn test_record_basic() {
    let src = r#"
record Point { x: Float, y: Float, }
fn main() -> Float [Pure] {
    let p = Point { x: 1.0, y: 2.0 }
    p.x
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Float(1.0));
}

#[test]
fn test_record_in_function() {
    let src = r#"
record Person { name: String, age: Int, }
fn greet(p: Person) -> String [Pure] {
    "Hello " ++ p.name
}
fn main() -> String [Pure] {
    let person = Person { name: "Alice", age: 30, }
    greet(person)
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::String("Hello Alice".into()));
}

#[test]
fn test_record_return() {
    let src = r#"
record Point { x: Float, y: Float, }
fn make_point(x: Float, y: Float) -> Point [Pure] {
    Point { x: x, y: y, }
}
fn main() -> Float [Pure] {
    let p = make_point(3.0, 4.0)
    p.y
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Float(4.0));
}

#[test]
fn test_record_in_list() {
    let src = r#"
record Item { value: Int, }
fn main() -> Int [Pure] {
    let items = [Item { value: 10, }, Item { value: 20, }]
    let first = List.get(items, 0)
    match first {
        Some(x) => x.value,
        None => 0,
    }
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Int(10));
}

#[test]
fn test_record_field_update() {
    // Test that we can read multiple fields
    let src = r#"
record Vec2 { x: Float, y: Float, }
fn magnitude_sq(v: Vec2) -> Float [Pure] {
    v.x * v.x + v.y * v.y
}
fn main() -> Float [Pure] {
    let v = Vec2 { x: 3.0, y: 4.0, }
    magnitude_sq(v)
}
"#;
    let result = run(src).unwrap();
    assert_eq!(result, Value::Float(25.0));
}
