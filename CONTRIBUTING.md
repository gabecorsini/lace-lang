# Contributing to Lace

Thanks for contributing.

## Build

```bash
cargo build --workspace
```

## Test

```bash
cargo test --workspace
```

## Run examples

```bash
./target/debug/lace run examples/hello.lace
./target/debug/lace run examples/pipeline.lace
./target/debug/lace run examples/tool_call.lace
./target/debug/lace run examples/uncertainty.lace
```

## Crate overview

- `lace-ast`: shared abstract syntax tree nodes and effect/type carriers
- `lace-lexer`: source text -> tokens
- `lace-parser`: tokens -> AST + parse errors
- `lace-types`: static type analysis and type diagnostics
- `lace-effects`: declared-vs-used effect validation
- `lace-interp`: runtime evaluator, tool-call execution, replay/checkpoint integration
- `lace-stdlib`: standard library declarations and runtime helpers
- `lace-cli`: end-user CLI (`run`, `check`, `repl`, `version`)

## Suggested workflow

1. Create a branch for your change.
2. Make focused commits with clear messages.
3. Run:
   - `cargo fmt --all`
   - `cargo test --workspace`
4. Verify at least one example with `lace check` and `lace run`.
5. Open a PR with a short description, motivation, and testing notes.

## Reporting issues

When filing an issue, please include:
- expected behavior
- actual behavior
- minimal repro `.lace` snippet
- CLI output (including diagnostics)
- rust/cargo versions
