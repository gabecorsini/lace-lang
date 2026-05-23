# Lace

A programming language designed for agentic execution.

Lace is built for a world where software agents call tools, mutate state, and fail in predictable ways. Instead of hiding those failures, Lace makes them explicit: effects are typed, uncertainty is represented in the type system, and side effects can be replayed deterministically.

## Why Lace exists

Agent systems usually break in familiar ways:
- silent tool failures and retries without traceability
- hidden side effects and non-deterministic runs
- ambiguous outputs that downstream code treats as certain
- context blowouts that degrade quality over time

Lace is designed so these problems are loud and inspectable.

## Current highlights

- First-class tool declarations with typed signatures
- Effect-annotated functions (`[Pure]`, `[IO]`, `[ToolCall]`, ...)
- Explicit uncertainty types (`Confident<T>`, `Uncertain<T>`)
- Pipeline syntax (`|>`) for composable data flow
- Checkpoint + replay runtime support for deterministic recovery

## Quick start

Requirements:
- Rust stable toolchain

Build the workspace:

```bash
cargo build --workspace
```

Run CLI help:

```bash
./target/debug/lace --help
```

Check and run an example:

```bash
./target/debug/lace check examples/hello.lace
./target/debug/lace run examples/hello.lace
```

## Language snippets

### 1) Pipeline composition

```lace
fn filter_even(values: List<Int>) -> List<Int> [Pure] { values }
fn scale(values: List<Int>) -> List<Int> [Pure] { values }

fn main() -> List<Int> [Pure] {
  [1, 2, 3, 4, 5]
    |> filter_even()
    |> scale()
}
```

### 2) Effect-typed entrypoint with tool call

```lace
@shell("echo '{\"ok\":true}'")
tool ping() -> Result<Dynamic, String>

fn main() -> Result<Dynamic, String> [ToolCall, IO] {
  println("calling tool")
  ping()
}
```

### 3) Explicit uncertainty

```lace
tool classify(prompt: String) -> Uncertain<List<String>>

fn main() -> Unit [ToolCall, IO] {
  let result = classify("design a robust coding agent")
  match result {
    Uncertain(_candidates) => println("model returned multiple plausible answers"),
  }
}
```

## Project roadmap

Completed:
- Phase 1: core parser/type/effect/interpreter skeleton
- Phase 2: typed values and effect validation expansion
- Phase 3: tool-call surface + replay/checkpoint foundations
- Phase 4: polished CLI (`run/check/repl/version`), public docs, and curated examples

Next:
- stronger pattern-matching/runtime coverage
- richer stdlib surface and collection transforms
- package/module boundaries and import system
- expanded test matrix and conformance suites

## Repository layout

- `crates/lace-ast`: AST definitions shared across compiler/runtime stages
- `crates/lace-lexer`: tokenization
- `crates/lace-parser`: parser + parse diagnostics
- `crates/lace-types`: static type checker
- `crates/lace-effects`: effect checker and effect diagnostics
- `crates/lace-interp`: interpreter runtime with tool execution + replay hooks
- `crates/lace-stdlib`: standard library surface/types
- `crates/lace-cli`: command-line interface (`lace` binary)

## License

Apache-2.0
