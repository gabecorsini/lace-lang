# Lace Language Specification
## Version 0.1 (Design Phase)

---

## 1. Overview

Lace is a statically-typed, effect-annotated programming language designed for agentic execution. Its core purpose is to make agent failure modes — ambiguous state, silent errors, non-deterministic retries, context overruns — impossible or loudly detectable at compile time.

Lace is not a general-purpose language. It is purpose-built for programs where:
- Execution may pause, replay, or resume from a checkpoint
- External tool calls are first-class, typed, and mockable
- Uncertainty is a valid return value, not an exception
- Context budgets (token limits) are compile-time constraints

Implementation language: Rust.

---

## 2. Type System

### 2.1 Primitive Types

| Type     | Description                                      |
|----------|--------------------------------------------------|
| `Int`    | 64-bit signed integer                            |
| `Float`  | 64-bit IEEE 754 floating point                   |
| `Bool`   | `true` or `false`                                |
| `String` | UTF-8 encoded immutable string                   |
| `Bytes`  | Raw byte buffer                                  |
| `Unit`   | The empty return type (like Rust's `()`)         |

### 2.2 Composite Types

```
List<T>          -- ordered, homogeneous sequence
Map<K, V>        -- key-value store, keys must be Eq + Hash
Tuple<T1, T2..>  -- fixed-size heterogeneous product type
Record { .. }    -- named-field struct
Enum { .. }      -- sum type / tagged union
```

### 2.3 Fallibility and Optionality

**No nulls. No exceptions.** All absence and failure is represented as values.

```
Option<T>        -- Some(T) or None
Result<T, E>     -- Ok(T) or Err(E)
```

Errors must be handled at the call site. Unhandled `Result` values are a compile error.

### 2.4 Uncertainty Types

First-class types for agent reasoning outputs:

```
Confident<T>         -- high-certainty single value
Uncertain<[T]>       -- agent is not sure; carries a ranked candidate list
```

`Uncertain<[T]>` is not an error — it is a valid output. Callers must pattern-match on it explicitly. You cannot pass an `Uncertain<[T]>` where a `T` is expected without unwrapping.

```lace
fn classify(text: String) -> Uncertain<[Category]> [ToolCall] {
    // returns multiple candidates with confidence scores
}

let result = classify("schedule meeting with Gabe");
match result {
    Confident(cat)     => handle_certain(cat),
    Uncertain(choices) => resolve_ambiguity(choices),
}
```

### 2.5 Gradual Typing

The core type system is static. The `?` escape hatch opts a binding into dynamic typing:

```lace
let x: ? = external_json_blob();
```

`?`-typed values must be narrowed before use in typed contexts. The compiler tracks narrowing state and warns if a `?` value flows unnarrowed into a statically-typed function.

### 2.6 Generics

Lace supports parametric generics with trait bounds:

```lace
fn map<A, B>(list: List<A>, f: fn(A) -> B) -> List<B> [Pure] { .. }

fn fetch<T: Deserialize>(url: String) -> Result<T, HttpError> [IO] { .. }
```

Trait bounds are written after the type parameter with `:`. Multiple bounds: `T: Serialize + Eq`.

---

## 3. Effect System

Every function in Lace carries an effect annotation declaring what side-effects it produces. This is enforced at compile time.

### 3.1 Effect Tags

| Tag        | Meaning                                                         |
|------------|----------------------------------------------------------------|
| `Pure`     | No side effects. Deterministic. Safe to memoize/replay.        |
| `IO`       | Reads or writes to file system, network, or environment.       |
| `Mut`      | Mutates state outside the function's own scope.                |
| `ToolCall` | Invokes an external `tool` declaration.                        |

Effect annotations are declared in brackets after the parameter list:

```lace
fn add(a: Int, b: Int) -> Int [Pure] { a + b }

fn read_file(path: String) -> Result<String, IoError> [IO] { .. }

fn call_llm(prompt: String) -> Confident<String> [ToolCall] { .. }
```

### 3.2 Effect Composition

A function that calls effectful functions must declare the union of effects:

```lace
fn summarize(path: String) -> Result<String, Error> [IO, ToolCall] {
    let content = read_file(path)?;     // IO
    call_llm("Summarize: " + content)   // ToolCall
}
```

### 3.3 Effect Constraints

Function types carry their effects in the signature:

```lace
fn run_pure_only(f: fn(Int) -> Int [Pure]) -> Int {
    f(42)
}
```

Passing an `IO` function where `Pure` is required is a compile error.

### 3.4 Pure Contexts

Blocks annotated `pure { .. }` are compile-checked: any effectful call inside is a compile error. Useful for declaring computation-only zones.

---

## 4. Syntax

### 4.1 Variables and Bindings

Immutable by default:

```lace
let name = "Hermes"
let count: Int = 0

mut let counter = 0
counter = counter + 1
```

`let` bindings are immutable. `mut let` bindings allow reassignment. Shadowing is allowed (a new `let` in the same scope hides the previous binding).

### 4.2 Functions

```lace
fn greet(name: String) -> String [Pure] {
    "Hello, " + name
}
```

The return type is mandatory for non-trivial functions. The effect annotation is mandatory for all functions except those inferred as `Pure` (inference is allowed but explicit is preferred — see open questions).

Anonymous functions (closures):

```lace
let double = |x: Int| -> Int [Pure] { x * 2 }
let items = [1, 2, 3] |> map(double)
```

### 4.3 Pipeline Operator

`|>` pipes the left-hand value as the first argument of the right-hand function:

```lace
let result =
    "raw text"
    |> clean()
    |> tokenize()
    |> embed()
    |> store_in_vector_db()
```

Pipeline errors surface as `Result` values and propagate via `?` or explicit match.

### 4.4 Pattern Matching

```lace
match value {
    Some(x)        => use(x),
    None           => default(),
    Ok(v)          => process(v),
    Err(e)         => log_error(e),
    Confident(v)   => v,
    Uncertain(vs)  => vs[0],  // pick highest-confidence candidate
}
```

Matches are exhaustive. The compiler rejects non-exhaustive matches.

### 4.5 Error Propagation

The `?` operator unwraps `Result::Ok` or short-circuits with `Err`:

```lace
fn pipeline(input: String) -> Result<Output, Error> [IO, ToolCall] {
    let parsed = parse(input)?;
    let enriched = enrich(parsed)?;
    transform(enriched)
}
```

### 4.6 Tool Declarations

`tool` is a first-class keyword for declaring typed external callables:

```lace
tool web_search(query: String, limit: Int = 10) -> Result<List<SearchResult>, ToolError>
    retries: 3
    timeout: 30s
    mock: mock_web_search
```

Tool declarations live at the module level and are not function bodies. The runtime handles retries, timeout, and mock substitution transparently.

### 4.7 Annotations

```lace
@context_bounded(tokens: 2048)
fn summarize_long_doc(doc: String) -> String [ToolCall] { .. }

@checkpoint
fn expensive_step(data: Data) -> Result<Output, Error> [IO, ToolCall] { .. }
```

`@context_bounded` is a compile-time constraint. The compiler statically estimates token consumption where possible and warns on potential overruns.

`@checkpoint` tells the runtime to persist state before and after this function call. Enables deterministic replay from this point.

### 4.8 Modules

```lace
module lace.agent.pipeline

use lace.stdlib.tool.{ Tool, ToolError }
use lace.stdlib.types.{ Confident, Uncertain }
```

### 4.9 Records and Enums

```lace
record SearchResult {
    url: String,
    title: String,
    snippet: String,
    score: Float,
}

enum TaskStatus {
    Pending,
    Running { started_at: Int },
    Done { result: String },
    Failed { error: String },
}
```

---

## 5. Control Flow

### 5.1 Conditionals

```lace
if x > 0 {
    "positive"
} else if x < 0 {
    "negative"
} else {
    "zero"
}
```

`if` is an expression — it returns the last value of the taken branch. Both branches must have the same type.

### 5.2 Loops

```lace
for item in collection {
    process(item)
}

while condition {
    step()
}
```

Loop bodies are `Unit`. Loop-as-expression is not supported (use `map`, `fold`, `filter` instead).

### 5.3 Early Return

Explicit `return` is allowed but discouraged. Prefer expression-oriented style.

---

## 6. Concurrency Model

Lace's initial version targets sequential execution. Concurrency is reserved for a future release.

The effect system lays the groundwork: `Pure` functions are trivially parallelizable, and `Mut` functions are isolated by the scheduler. The runtime will eventually support parallel pipelines when the type system can prove absence of shared mutable state.

---

## 7. Interoperability

Lace can call Rust functions via an FFI boundary declared with `extern`:

```lace
extern fn fast_tokenize(text: String) -> List<String> [Pure]
    from: "lace_native::tokenize"
```

FFI functions must declare effects explicitly. The compiler trusts the declared effects — incorrect declarations cause undefined runtime behavior.

---

## 8. Grammar

See `grammar.md` for the complete EBNF definition.

---

## 9. Standard Library

See `stdlib-surface.md` for the full surface spec.

---

## 10. Runtime

See `runtime-model.md` for the interpreter, AOT compilation, and replay mechanism.
