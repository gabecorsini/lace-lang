# Lace Standard Library Surface Spec
## Version 0.1 (Design Phase)

This document covers the standard library surface for the initial Lace release. It is a surface spec — it defines types, function signatures, and semantics. It does not define implementation details.

---

## 1. Module Structure

```
lace.stdlib
  .types          -- core algebraic types (Option, Result, Confident, Uncertain)
  .tool           -- Tool, ToolError, tool invocation primitives
  .pipeline       -- pipeline helpers, collection transforms
  .retry          -- retry/fallback decorators and policies
  .io             -- file, network, env, clock
  .fmt            -- string formatting
  .json           -- JSON encode/decode
  .context        -- context budget tracking
```

---

## 2. Core Types (`lace.stdlib.types`)

### 2.1 Option\<T\>

```lace
enum Option<T> {
    Some(T),
    None,
}
```

Functions:
```lace
fn Option::unwrap_or<T>(self: Option<T>, default: T) -> T [Pure]
fn Option::map<T, U>(self: Option<T>, f: fn(T) -> U [Pure]) -> Option<U> [Pure]
fn Option::flat_map<T, U>(self: Option<T>, f: fn(T) -> Option<U> [Pure]) -> Option<U> [Pure]
fn Option::is_some<T>(self: Option<T>) -> Bool [Pure]
fn Option::is_none<T>(self: Option<T>) -> Bool [Pure]
fn Option::ok_or<T, E>(self: Option<T>, err: E) -> Result<T, E> [Pure]
fn Option::filter<T>(self: Option<T>, pred: fn(T) -> Bool [Pure]) -> Option<T> [Pure]
```

### 2.2 Result\<T, E\>

```lace
enum Result<T, E> {
    Ok(T),
    Err(E),
}
```

Functions:
```lace
fn Result::unwrap_or<T, E>(self: Result<T, E>, default: T) -> T [Pure]
fn Result::map<T, U, E>(self: Result<T, E>, f: fn(T) -> U [Pure]) -> Result<U, E> [Pure]
fn Result::map_err<T, E, F>(self: Result<T, E>, f: fn(E) -> F [Pure]) -> Result<T, F> [Pure]
fn Result::flat_map<T, U, E>(self: Result<T, E>, f: fn(T) -> Result<U, E> [Pure]) -> Result<U, E> [Pure]
fn Result::is_ok<T, E>(self: Result<T, E>) -> Bool [Pure]
fn Result::is_err<T, E>(self: Result<T, E>) -> Bool [Pure]
fn Result::ok<T, E>(self: Result<T, E>) -> Option<T> [Pure]
fn Result::err<T, E>(self: Result<T, E>) -> Option<E> [Pure]
```

### 2.3 Confident\<T\>

Represents a high-certainty single-value output from a reasoning or tool step.

```lace
enum Confident<T> {
    Confident(T),
}
```

Functions:
```lace
fn Confident::unwrap<T>(self: Confident<T>) -> T [Pure]
fn Confident::map<T, U>(self: Confident<T>, f: fn(T) -> U [Pure]) -> Confident<U> [Pure]
fn Confident::to_result<T, E>(self: Confident<T>) -> Result<T, E> [Pure]
```

Typically produced by tool calls or LLM steps that express high confidence. Callers receive a `Confident<T>` when they can proceed without asking for disambiguation.

### 2.4 Uncertain\<\[T\]\>

Represents ambiguous output — a ranked list of candidates. Callers MUST handle this type explicitly.

```lace
enum Uncertain<T> {
    Uncertain(List<T>),
}
```

Functions:
```lace
fn Uncertain::candidates<T>(self: Uncertain<T>) -> List<T> [Pure]
fn Uncertain::top<T>(self: Uncertain<T>) -> Option<T> [Pure]
fn Uncertain::resolve<T>(self: Uncertain<T>, f: fn(List<T>) -> T [Pure]) -> Confident<T> [Pure]
fn Uncertain::map<T, U>(self: Uncertain<T>, f: fn(T) -> U [Pure]) -> Uncertain<U> [Pure]
fn Uncertain::filter<T>(self: Uncertain<T>, pred: fn(T) -> Bool [Pure]) -> Uncertain<T> [Pure]
```

Example:
```lace
let result: Uncertain<[Category]> = classify(input);
let resolved = result
    |> Uncertain::filter(|c| c.score > 0.7)
    |> Uncertain::resolve(|cs| cs[0]);
```

---

## 2.5 Scored\<T\>

Helper type for attaching a confidence score to any value. Use with `Uncertain` when ranking is important.

```lace
record Scored<T> {
    value: T,
    score: Float,
}
```

Example:
```lace
let result: Uncertain<[Scored<Category>]> = classify(input);
```

---

## 3. Tool Primitives (`lace.stdlib.tool`)

### 3.1 ToolError

```lace
enum ToolError {
    Timeout { after: Duration },
    NetworkFailure { message: String },
    AuthFailure { message: String },
    RateLimited { retry_after: Option<Duration> },
    UnexpectedResponse { status: Int, body: String },
    MockNotConfigured { tool_name: String },
    Unknown { message: String },
}
```

### 3.2 Tool Declaration Semantics

A `tool` declaration is a module-level item. It defines:
- The typed signature (parameters + return type)
- The retry policy (`retries: N`)
- The timeout (`timeout: DURATION`)
- The mock function (used in test/sandbox mode)

```lace
tool web_search(query: String, limit: Int = 10) -> Result<List<SearchResult>, ToolError>
    retries: 3
    timeout: 30s
    mock: mock_web_search

fn mock_web_search(query: String, limit: Int) -> Result<List<SearchResult>, ToolError> [Pure] {
    Ok([SearchResult { url: "mock://", title: "Mock Result", snippet: query, score: 1.0 }])
}
```

### 3.3 Tool Execution Context

The runtime provides a `ToolContext` that tools receive implicitly:
```lace
record ToolContext {
    run_id: String,
    attempt: Int,          -- 0-indexed retry count
    dry_run: Bool,         -- if true, log intent without executing
    mock_mode: Bool,       -- if true, use mock function
    timeout_remaining: Duration,
}
```

Tools can inspect their context for conditional logic (e.g. cheaper fallback on retry N).

---

## 4. Pipeline Primitives (`lace.stdlib.pipeline`)

### 4.1 Collection Transforms

```lace
fn map<A, B>(list: List<A>, f: fn(A) -> B [Pure]) -> List<B> [Pure]
fn filter<A>(list: List<A>, pred: fn(A) -> Bool [Pure]) -> List<A> [Pure]
fn fold<A, B>(list: List<A>, init: B, f: fn(B, A) -> B [Pure]) -> B [Pure]
fn flat_map<A, B>(list: List<A>, f: fn(A) -> List<B> [Pure]) -> List<B> [Pure]
fn zip<A, B>(a: List<A>, b: List<B>) -> List<Tuple<A, B>> [Pure]
fn take<A>(list: List<A>, n: Int) -> List<A> [Pure]
fn drop<A>(list: List<A>, n: Int) -> List<A> [Pure]
fn find<A>(list: List<A>, pred: fn(A) -> Bool [Pure]) -> Option<A> [Pure]
fn sort_by<A, B: Ord>(list: List<A>, key: fn(A) -> B [Pure]) -> List<A> [Pure]
fn group_by<A, K: Eq + Hash>(list: List<A>, key: fn(A) -> K [Pure]) -> Map<K, List<A>> [Pure]
fn unique<A: Eq + Hash>(list: List<A>) -> List<A> [Pure]
fn count<A>(list: List<A>) -> Int [Pure]
fn first<A>(list: List<A>) -> Option<A> [Pure]
fn last<A>(list: List<A>) -> Option<A> [Pure]
fn flatten<A>(list: List<List<A>>) -> List<A> [Pure]
fn partition<A>(list: List<A>, pred: fn(A) -> Bool [Pure]) -> Tuple<List<A>, List<A>> [Pure]
```

### 4.2 Result-Collecting Pipeline

When mapping over a list with a fallible function, collect all results:

```lace
fn try_map<A, B, E>(list: List<A>, f: fn(A) -> Result<B, E> [IO]) -> Result<List<B>, E> [IO]
-- Returns Ok(all_successes) or the first Err encountered.

fn try_map_all<A, B, E>(list: List<A>, f: fn(A) -> Result<B, E> [IO]) -> List<Result<B, E>> [IO]
-- Returns all results, successes and failures alike.
```

### 4.3 Async Pipeline (Reserved)

Reserved for future concurrency support. Will expose `par_map` and `par_filter` for `Pure` functions once the parallelism model is finalized.

---

## 5. Retry and Fallback (`lace.stdlib.retry`)

### 5.1 Retry Policy

```lace
record RetryPolicy {
    max_attempts: Int,
    backoff: Backoff,
    on: List<RetryCondition>,
}

enum Backoff {
    Fixed { delay: Duration },
    Exponential { base: Duration, max: Duration },
    None,
}

enum RetryCondition {
    AnyError,
    OnError(fn(ToolError) -> Bool [Pure]),
    OnStatusCode(Int),
}
```

### 5.2 Retry Decorator

```lace
@retry(policy: RetryPolicy)
fn fragile_call(input: String) -> Result<String, ToolError> [ToolCall] { .. }
```

When `@retry` is applied, the runtime wraps the function with the policy. The function's declared effect remains `ToolCall`. The retry is transparent to callers.

Standalone retry combinator:
```lace
fn with_retry<T, E>(
    policy: RetryPolicy,
    f: fn() -> Result<T, E> [ToolCall]
) -> Result<T, E> [ToolCall]
```

### 5.3 Fallback

```lace
fn fallback<T, E1, E2>(
    primary: fn() -> Result<T, E1> [ToolCall],
    secondary: fn() -> Result<T, E2> [ToolCall]
) -> Result<T, E2> [ToolCall]
-- Calls primary; if it returns Err, calls secondary.
```

Decorator form:
```lace
@fallback(to: cheap_summarize)
fn expensive_summarize(text: String) -> Result<String, ToolError> [ToolCall] { .. }

fn cheap_summarize(text: String) -> Result<String, ToolError> [ToolCall] { .. }
```

### 5.4 Circuit Breaker (Reserved)

Reserved for future release. Will expose `@circuit_breaker(threshold: N, window: Duration)` to prevent cascading failures in high-volume tool call loops.

---

## 6. IO (`lace.stdlib.io`)

```lace
-- File system
fn read_file(path: String) -> Result<String, IoError> [IO]
fn write_file(path: String, content: String) -> Result<Unit, IoError> [IO]
fn append_file(path: String, content: String) -> Result<Unit, IoError> [IO]
fn file_exists(path: String) -> Bool [IO]
fn delete_file(path: String) -> Result<Unit, IoError> [IO]
fn list_dir(path: String) -> Result<List<String>, IoError> [IO]

-- Environment
fn env_var(name: String) -> Option<String> [IO]
fn env_var_required(name: String) -> Result<String, EnvError> [IO]

-- Clock
fn now_unix() -> Int [Time]         -- seconds since epoch (Time implies IO; journaled for deterministic replay)
fn now_millis() -> Int [Time]       -- milliseconds since epoch (Time implies IO)
fn sleep(duration: Duration) -> Unit [IO]

-- Stdin / Stdout
fn print(msg: String) -> Unit [IO]
fn println(msg: String) -> Unit [IO]
fn read_line() -> Result<String, IoError> [IO]
```

---

## 7. Formatting (`lace.stdlib.fmt`)

```lace
fn format(template: String, args: Map<String, ?>) -> String [Pure]
-- "Hello, {name}!" with args = { "name": "Gabe" } => "Hello, Gabe!"

fn to_string<T: Display>(value: T) -> String [Pure]
fn join(items: List<String>, sep: String) -> String [Pure]
fn trim(s: String) -> String [Pure]
fn split(s: String, delimiter: String) -> List<String> [Pure]
fn starts_with(s: String, prefix: String) -> Bool [Pure]
fn ends_with(s: String, suffix: String) -> Bool [Pure]
fn contains(s: String, sub: String) -> Bool [Pure]
fn replace(s: String, from: String, to: String) -> String [Pure]
fn to_upper(s: String) -> String [Pure]
fn to_lower(s: String) -> String [Pure]
fn len(s: String) -> Int [Pure]
```

---

## 8. JSON (`lace.stdlib.json`)

```lace
fn json_encode<T: Serialize>(value: T) -> Result<String, JsonError> [Pure]
fn json_decode<T: Deserialize>(input: String) -> Result<T, JsonError> [Pure]
fn json_decode_dynamic(input: String) -> Result<?, JsonError> [Pure]
-- Returns a ?-typed value for when schema is unknown at compile time.
```

---

## 9. Context Budget (`lace.stdlib.context`)

```lace
fn context_remaining() -> Int [IO]
-- Returns estimated remaining token budget for the current execution context.

fn context_used() -> Int [IO]
-- Returns estimated tokens consumed so far in the current execution context.

fn context_assert(tokens: Int) -> Result<Unit, ContextError> [IO]
-- Returns Err if fewer than `tokens` remain.
```

These functions are the runtime counterpart to the `@context_bounded` compile-time annotation. Use them for dynamic budget checks inside loops or when the static estimator cannot provide guarantees.

```lace
enum ContextError {
    BudgetExceeded { used: Int, limit: Int },
    BudgetInsufficient { required: Int, remaining: Int },
}
```
