# Lace Repository Structure Recommendation
## Version 0.1 (Design Phase)

---

## 1. License Recommendation

**Recommendation: Apache 2.0**

Rationale:
- Apache 2.0 includes an explicit patent grant, which matters for a language runtime (patents on language features are rare but not unheard of)
- Apache 2.0 is widely adopted in the Rust ecosystem (rustc itself is MIT + Apache 2.0 dual-licensed)
- Apache 2.0 is compatible with MIT (consumers can treat an Apache 2.0 library as MIT-compatible for most purposes)
- It is permissive enough to allow commercial use without friction, which is appropriate for a language intended for broad adoption

If maximum simplicity is the goal and patent concerns are low-priority, MIT is acceptable. The marginal protection of Apache 2.0 costs nothing.

**Decision: Apache 2.0, with an SPDX identifier in every source file header.**

---

## 2. Repository Layout

```
lace/
  Cargo.toml            -- workspace manifest
  Cargo.lock
  LICENSE               -- Apache-2.0
  README.md
  CONTRIBUTING.md
  GLOSSARY.md
  .github/
    workflows/
      ci.yml            -- main CI pipeline
      release.yml       -- binary release pipeline
    CODEOWNERS
    PULL_REQUEST_TEMPLATE.md
    ISSUE_TEMPLATE/
      bug_report.md
      feature_request.md
  docs/
    spec/               -- this directory (language spec, grammar, etc.)
    design/             -- ADRs and design decision records
    book/               -- future: mdBook user documentation
  crates/
    lace-lexer/         -- tokenizer
    lace-parser/        -- AST producer
    lace-ast/           -- AST types shared across crates
    lace-types/         -- type representations, type checker
    lace-effects/       -- effect system verifier
    lace-mir/           -- mid-level IR (AOT only)
    lace-interp/        -- tree-walking interpreter
    lace-codegen/       -- MIR -> LLVM IR (AOT)
    lace-runtime/       -- journal, checkpoint, replay engine
    lace-stdlib/        -- standard library implementation
    lace-repl/          -- REPL frontend
    lace-cli/           -- `lace` binary (entry point)
  tests/
    integration/        -- end-to-end .lace test programs
    fixtures/           -- shared test data
  examples/
    hello_world.lace
    tool_call.lace
    pipeline.lace
    checkpoint_replay.lace
    uncertain_classification.lace
```

---

## 3. Crate Responsibilities

### lace-lexer

Input: raw source string
Output: `Vec<Token>` + `Vec<LexError>`

No dependencies on other lace crates. Tokenizes all Lace keywords, literals, operators, and annotations. Preserves spans for error reporting.

### lace-parser

Input: `Vec<Token>`
Output: untyped AST (`lace-ast` types)

Produces a concrete syntax tree matching the EBNF in `grammar.md`. Error recovery: attempts to continue after parse errors to report multiple issues in one pass.

### lace-ast

Shared types: `Expr`, `Stmt`, `Type`, `FnDecl`, `ToolDecl`, `RecordDecl`, `EnumDecl`, `Annotation`, `Span`.

This crate has zero dependencies on other lace crates. It is the shared language between frontend (parser) and backend (type checker, interpreter, codegen).

### lace-types

Input: untyped AST
Output: typed AST (same structure, nodes decorated with resolved types)

Implements:
- Type inference and checking
- Generic instantiation
- Trait bound resolution
- Gradual type narrowing for `?`-typed values
- `@context_bounded` compile-time analysis

### lace-effects

Input: typed AST
Output: effect-annotated AST (or compile errors for effect violations)

Implements:
- Effect annotation verification
- Effect composition checking (callee effects ⊆ caller declared effects)
- `pure { .. }` block enforcement
- Effect polymorphism resolution

### lace-mir

Input: effect-annotated AST (AOT path only)
Output: MIR (flat, typed, explicit control flow)

Implements closure lifting, `|>` lowering, and memory layout decisions. Internal to the AOT pipeline.

### lace-interp

Input: effect-annotated AST
Output: runtime values, side effects (via `lace-runtime`)

Tree-walking interpreter. Shares the `lace-runtime` journal and checkpoint engine with the AOT runtime.

### lace-codegen

Input: MIR
Output: LLVM IR (via `inkwell`)

Lowers MIR to LLVM IR. Calls LLVM to produce native code. Links against the `lace-runtime` Rust library.

### lace-runtime

The shared runtime library:
- `Journal` — append-only side-effect log
- `CheckpointEngine` — save/load/replay checkpoints
- `EffectTracker` — runtime effect assertions (defense-in-depth)
- `ToolRegistry` — registered tools, mock substitution, dry-run mode
- `ContextBudget` — token budget tracking

Used by both interpreter and AOT-compiled binaries.

### lace-stdlib

Implementation of `lace.stdlib.*`. Compiled to a Lace module that is automatically available in all programs. Thin Rust wrappers where needed (e.g. for file I/O).

### lace-repl

The interactive REPL. Depends on `lace-interp` + `lace-runtime`. Uses `rustyline` for line editing and history.

### lace-cli

The `lace` binary. Subcommands:

```
lace run <file.lace> [-- args..]     -- run a script
lace repl                             -- start interactive REPL
lace build <file.lace> [-o <out>]    -- AOT compile
lace check <file.lace>               -- type check without running
lace fmt <file.lace>                 -- format source (future)
lace doc <file.lace>                 -- generate docs (future)
```

---

## 4. Dependency Graph

```
lace-cli
  ├── lace-repl
  │     └── lace-interp
  │           ├── lace-ast
  │           ├── lace-types
  │           ├── lace-effects
  │           └── lace-runtime
  ├── lace-codegen (AOT path)
  │     ├── lace-mir
  │     │     ├── lace-ast
  │     │     ├── lace-types
  │     │     └── lace-effects
  │     └── lace-runtime
  ├── lace-parser
  │     └── lace-lexer
  └── lace-stdlib
```

All crates depend on `lace-ast`. No circular dependencies.

---

## 5. CI Pipeline

### .github/workflows/ci.yml

Triggers: push to any branch, pull request to `main`.

Jobs:

```yaml
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace

  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy }
      - run: cargo clippy --workspace -- -D warnings

  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt }
      - run: cargo fmt --check

  integration:
    runs-on: ubuntu-latest
    steps:
      - run: cargo build --workspace
      - run: ./scripts/run_integration_tests.sh
```

### .github/workflows/release.yml

Triggers: tag push matching `v*.*.*`.

Builds binaries for:
- `x86_64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-apple-darwin`

Uses `cross` for cross-compilation. Uploads binaries as GitHub Release assets.

---

## 6. Workspace Cargo.toml

```toml
[workspace]
members = [
    "crates/lace-ast",
    "crates/lace-lexer",
    "crates/lace-parser",
    "crates/lace-types",
    "crates/lace-effects",
    "crates/lace-mir",
    "crates/lace-interp",
    "crates/lace-codegen",
    "crates/lace-runtime",
    "crates/lace-stdlib",
    "crates/lace-repl",
    "crates/lace-cli",
]
resolver = "2"

[workspace.package]
version = "0.1.0"
authors = ["Gabe Corsini", "Hermes"]
license = "Apache-2.0"
edition = "2021"
repository = "https://github.com/gcorsini/lace"

[workspace.dependencies]
# Shared deps across crates — pin versions here
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
```

---

## 7. CONTRIBUTING.md Outline

The `CONTRIBUTING.md` should cover:
1. Dev environment setup (Rust toolchain, `cargo install` requirements)
2. Running tests (`cargo test --workspace`)
3. Code style (enforced via `cargo fmt` + `cargo clippy`)
4. PR process (feature branches, PR template, CI must pass)
5. Issue triage labels (bug, enhancement, spec-question, help-wanted)
6. Spec change process (changes to `docs/spec/` require a design discussion issue before a PR)
7. License agreement (DCO or CLA — recommendation: DCO, lighter weight)

---

## 8. Initial README.md Outline

```markdown
# Lace

A programming language for agentic execution.

## Why Lace?

[Brief problem statement from the mission brief]

## Quick Start

[REPL installation + hello world in 5 lines]

## Documentation

- [Language Specification](docs/spec/language-spec.md)
- [EBNF Grammar](docs/spec/grammar.md)
- [Effect System](docs/spec/effect-system.md)
- [Standard Library](docs/spec/stdlib-surface.md)
- [Runtime Model](docs/spec/runtime-model.md)

## Status

Lace is in the design/spec phase. No compiler exists yet.

## License

Apache 2.0
```
