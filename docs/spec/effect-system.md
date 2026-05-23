# Lace Effect System
## Version 0.1 (Design Phase)

---

## 1. Purpose

The effect system is the mechanism by which Lace makes agent failure modes loud at compile time. Every function in Lace carries a compile-time declaration of its side-effects. The compiler enforces these declarations, the runtime uses them for sandboxing, retry decisions, and parallelism, and they serve as machine-readable documentation for any automated system consuming Lace code.

The core thesis: **if you cannot see the effects, you cannot reason about replay, isolation, or safety.** Lace makes effects visible.

---

## 2. Effect Tags

### 2.1 Pure

```lace
fn add(a: Int, b: Int) -> Int [Pure] { a + b }
```

A `Pure` function:
- Produces the same output for the same inputs, always
- Does not read or write any state outside its arguments
- Does not call any `IO`, `Mut`, or `ToolCall` functions
- Is safe to memoize, parallelize, replay, and reorder

Violation: calling an `IO` function inside a `[Pure]` body is a compile error.

The runtime may cache `Pure` function results keyed on inputs. This is not guaranteed but is permitted.

### 2.2 IO

```lace
fn read_file(path: String) -> Result<String, IoError> [IO] { .. }
fn write_file(path: String, content: String) -> Result<Unit, IoError> [IO] { .. }
fn http_get(url: String) -> Result<String, HttpError> [IO] { .. }
```

An `IO` function:
- Reads from or writes to: file system, network sockets, environment variables, stdin/stdout, clock
- May produce different results on repeated calls
- Is logged by the runtime's side-effect journal
- Is not safe to parallelize without explicit coordination

`IO` does NOT imply mutation of in-memory state. A function that reads a file is `[IO]`, not `[Mut]`.

### 2.3 Mut

```lace
fn register(registry: mut Map<String, Handler>, key: String, h: Handler) -> Unit [Mut] { .. }
fn update_counter(store: mut GlobalStore, delta: Int) -> Unit [Mut] { .. }
```

A `Mut` function:
- Mutates **external** state that outlives the function call — a shared data store, a global registry, or an externally-managed structure passed by mutable reference
- Is tracked by the runtime for state isolation during replay
- Cannot be called in a `pure { .. }` block

Note: Lace values are immutable. `mut let` bindings allow rebinding the variable to a new value but do not allow in-place mutation of data structures. Functions that appear to "update" a collection (e.g. `push`, `insert`) return a new value. The `[Mut]` effect is reserved exclusively for observable mutation of external state.

### 2.4 Time and Rand (IO sub-tags)

```lace
fn now_unix() -> Int [Time]
fn now_millis() -> Int [Time]
fn random_float() -> Float [Rand]
fn random_int(min: Int, max: Int) -> Int [Rand]
```

`Time` and `Rand` are sub-tags of `IO`. A function declaring `[Time]` is also implicitly `[IO]`. These sub-tags exist to give the runtime's journal enough information to replay deterministically:
- `Time` calls are journaled separately; replay substitutes the recorded timestamp
- `Rand` calls are journaled separately; replay substitutes the recorded value

User code rarely declares `[Time]` or `[Rand]` directly. Only stdlib functions use them. Callers that invoke a `[Time]` or `[Rand]` function must declare at minimum `[IO]`. The sub-tag distinction matters for the journal, not for the caller's effect obligations.

### 2.5 ToolCall

```lace
tool web_search(query: String) -> Result<List<SearchResult>, ToolError>
    retries: 3
    timeout: 30s

fn search_and_summarize(topic: String) -> Result<String, Error> [ToolCall] {
    let results = web_search(topic)?;
    Ok(format_results(results))
}
```

A `ToolCall` function:
- Invokes one or more `tool` declarations
- Carries all the semantics of `IO` plus tool-specific logging, retry, and mock substitution
- Is always logged with full input/output pairs by the runtime
- Is the primary effect used by agent-facing code

`ToolCall` implies `IO` (a ToolCall function is always at least as effectful as IO). A function annotated `[ToolCall]` does not need to also declare `[IO]`.

---

## 3. Effect Composition

When a function calls other functions, its declared effects must be the union of all callee effects.

```lace
fn step_a(x: String) -> String [IO] { .. }
fn step_b(x: String) -> String [ToolCall] { .. }
fn step_c(x: String) -> String [Mut] { .. }

fn pipeline(input: String) -> Result<String, Error> [IO, ToolCall, Mut] {
    let a = step_a(input);
    let b = step_b(a)?;
    step_c(b)
}
```

The compiler rejects underdeclared effects:

```lace
// ERROR: function calls ToolCall but declares only [IO]
fn bad(input: String) -> String [IO] {
    call_llm(input)   // call_llm is [ToolCall]
}
```

Overdeclaring is allowed (declaring `[IO, ToolCall]` on a function that only uses `IO`) but triggers a compiler warning.

---

## 4. Effect Polymorphism

Functions that accept function arguments must be able to express "I carry whatever effects my argument carries":

```lace
fn apply<A, B, efx>(f: fn(A) -> B [efx], value: A) -> B [efx] {
    f(value)
}
```

Effect variables (`efx` above) are quantified like type variables. This allows higher-order functions to remain effect-correct without hardcoding effect sets.

This is supported in v0.1 via predefined effect variables (lowercase identifiers like `efx`). Effect variables are declared alongside type parameters: `<A, B, efx>`. The initial implementation may restrict composition to a fixed set of patterns. See open-questions.md Q2 (resolved: predefined effect variables).

---

## 5. Pure Blocks

A `pure { .. }` block is a compile-time assertion that the enclosed code is free of side effects:

```lace
fn process(data: Data) -> Summary [IO] {
    let raw = read_raw(data)?;   // IO is fine outside pure block
    pure {
        // compile error if any IO/Mut/ToolCall call appears here
        let normalized = normalize(raw);
        let scored = score(normalized);
        scored
    }
}
```

Pure blocks are useful for clearly demarcating computation-only zones within otherwise effectful functions. The compiler enforces the purity constraint at block boundaries.

---

## 6. Runtime Implications

### 6.1 Side-Effect Journal

Every `IO` and `ToolCall` invocation is logged to the runtime's side-effect journal with:
- Timestamp
- Effect tag
- Function name and module
- Input arguments (serialized)
- Return value (serialized)
- Duration

The journal is append-only and is the source of truth for deterministic replay.

### 6.2 Replay Mode

When a Lace program is replayed from a checkpoint:
1. The runtime loads the journal from the checkpoint
2. For each `IO`/`ToolCall` call encountered during replay, it checks the journal
3. If a matching journal entry exists (same function + inputs), the recorded output is returned — no actual side effect is performed
4. If no journal entry exists (new code path since checkpoint), the side effect executes normally and is logged

`Pure` functions are re-executed during replay (they have no journal entries by definition, and are safe to recompute).

`Mut` operations during replay must be applied to the checkpoint's state snapshot, not to current live state.

### 6.3 Retry Safety

The runtime uses effect tags to determine whether a failed call can be safely retried:
- `Pure`: always safe to retry
- `IO`: safe to retry if idempotent; retry policy is declared on the `tool` or manually via `@retryable`
- `ToolCall`: retry behavior is declared on the `tool` declaration (`retries: N`)
- `Mut`: NOT automatically retried — mutations may have partially applied

### 6.4 Sandbox Isolation

In agent execution environments, `ToolCall` functions can be sandboxed:
- Mock substitution: the `mock:` field on a `tool` declaration names a `Pure` function used in test/sandbox mode
- Dry-run mode: the runtime logs what tool calls would be made without executing them
- Capability grants: a future access-control layer will gate which `ToolCall` effects are permitted per execution context

### 6.5 Parallelism

Currently reserved for future release. The effect system is designed to enable it:
- Two `Pure` functions may always run in parallel
- Two `IO` functions may run in parallel if they do not share file handles or sockets
- Two `Mut` functions on the same binding cannot run in parallel
- The scheduler will use effect tags to derive a safe execution DAG

---

## 7. Effect Inference

The initial implementation requires explicit effect annotations on all named functions. Inference is provided for anonymous closures:

```lace
let f = |x: Int| { x + 1 }           // inferred [Pure]
let g = |path: String| { read_file(path) }  // inferred [IO]
```

Named functions must be explicit. Explicit annotations are preferred on all functions, including closures, for readability and maintainability. The compiler will warn on inferred effects in non-trivial closures.

See open-questions.md Q1 (resolved: explicit annotations required).
