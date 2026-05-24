use lace_vm::run_source;

#[test]
fn hello_world() {
    let src = r#"print("hello world")"#;
    run_source(src, false).expect("hello_world should not error");
}

#[test]
fn arithmetic() {
    let src = r#"
let x = 2 + 3 * 4
print(x)
"#;
    run_source(src, false).expect("arithmetic should not error");
}

#[test]
fn if_else() {
    let src = r#"
let x = 10
if x > 5 { print("big") } else { print("small") }
"#;
    run_source(src, false).expect("if_else should not error");
}

#[test]
fn fn_call() {
    let src = r#"
fn double(n: Int) -> Int [Pure] { n * 2 }
print(double(21))
"#;
    run_source(src, false).expect("fn_call should not error");
}
