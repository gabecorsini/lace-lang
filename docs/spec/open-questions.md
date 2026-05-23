# Lace Open Questions
## Version 0.1 (Design Phase)

This document tracks architecturally unresolved questions. Each item has a status: **Deferred**, **Recommended** (recommendation made, awaiting decision), or **Resolved**.

---

## Q1: Effect Inference for Named Functions

**Status: Resolved**

**Question:**
Should Lace infer effects for named functions, or require explicit annotations on all named functions?

**Context:**
Effect inference is theoretically possible but has known trade-offs. Full inference can produce surprising effects at call sites (a function that "looks pure" infers a ToolCall because of a transitive callee three levels deep). Explicit annotations make effects visible at the declaration, which is the whole point of having them.

**Decision:**
Explicit effect annotations are required on all named `fn` declarations. Inference is allowed only for anonymous closures, where the body is short and visible at the call site. A `#[infer_effects]` escape hatch may be added in a later version if friction proves too high in practice.

Rationale: the entire value of the effect system is that effects are visible at declaration time. Silent inference defeats this. The friction of explicit annotations is acceptable and produces better documentation.

---

## Q2: Effect Polymorphism — Scope and Syntax

**Status: Resolved**

**Question:**
How expressive should effect polymorphism be in v0.1?

**Context:**
Full effect polymorphism (quantifying over effect sets in generic functions) requires non-trivial type system machinery. The use case is higher-order functions like `map`, `filter`, and `apply` that should carry the effects of their function arguments without hardcoding them.

Options:
1. No effect polymorphism — higher-order functions over effectful callables are not expressible. Workaround: duplicate functions for each effect combination. (Bad DX.)
2. Predefined effect variables — a small fixed set of effect variable names (`efx`, `efx1`, `efx2`) with constraint syntax. Simpler to implement.
3. Full row polymorphism — effects are a row type; arbitrary composition. Correct but complex.

**Decision:**
Option 2 for v0.1. Effect variables are lowercase identifiers declared alongside type parameters: `<A, B, efx>`. A function argument's effect is expressed as `fn(A) -> B [efx]`. The caller's declared effect must include all effect variables bound at the call site. Full row polymorphism is deferred to a later version.

Rationale: this covers the primary use case (higher-order stdlib functions like `map` and `apply`) without requiring a full row-type implementation. The predefined variable names make the grammar extension minimal and the semantics easy to explain.

---

## Q3: Uncertain\<\[T\]\> — Confidence Scores

**Status: Resolved**

**Question:**
Should `Uncertain<[T]>` carry confidence scores alongside the candidates, or leave scoring to the contained type?

**Context:**
If `Uncertain<[SearchResult]>` doesn't carry scores, then `SearchResult` must embed a score field. But what if the user doesn't want scores on `SearchResult` itself? A wrapper `Scored<T> { value: T, score: Float }` is one option. Another is `Uncertain<[T]>` always storing `List<Scored<T>>` internally.

**Decision:**
`Uncertain<[T]>` stores `List<T>` where `T` is the user's type. Scores are the user's responsibility to embed if needed. The stdlib provides a `Scored<T>` helper type:

```lace
record Scored<T> {
    value: T,
    score: Float,
}
```

A typical LLM classification step would return `Uncertain<[Scored<Category>]>`.

Rationale: baking score semantics into `Uncertain` would force every consumer to deal with scores even when scores are meaningless (e.g. when ranking by vote count or recency instead). Keeping `Uncertain` general and providing `Scored<T>` as a composable helper is the right layering.

---

## Q4: Mutable References vs. Ownership

**Status: Resolved**

**Question:**
How does Lace handle mutable data? Does it adopt Rust-style ownership/borrowing, or a simpler model?

**Context:**
The `[Mut]` effect marks mutation. But the question is: how is mutable state passed and tracked at the value level? Options:
1. Rust-style borrow checker — correct, safe, but significant learning curve
2. Copy-on-write semantics — every assignment copies; `Mut` is syntactic sugar for "this function replaces its argument"
3. Explicit mutable reference type (`&mut T`) — simpler than full Rust but similar in spirit
4. Pure value model — no mutation; `Mut` is reserved for external state only

**Decision:**
Option 4 for v0.1. Lace values are immutable. `mut let` bindings are rebindable (the variable can point to a new value) but do not allow in-place mutation of data structures. Functions that appear to "mutate" a collection (e.g. `push`, `insert`) return a new value — they do not modify in place. The `[Mut]` effect is reserved exclusively for external state mutation: writing to shared data stores, modifying global registries, updating external databases.

Rationale: this sidesteps the borrow checker complexity entirely for v0.1 while remaining sound. The immutable-value model is familiar from functional languages and plays well with the replay and checkpoint system (snapshots are cheap when values do not alias). If in-place mutation of large data structures proves too expensive in practice, copy-on-write optimization can be added at the runtime level without changing the language semantics.

---

## Q5: Module System — Single File vs. Package

**Status: Resolved**

**Question:**
Is a Lace "program" always a single file, or does the language support multi-file packages in v0.1?

**Context:**
Multi-file packages require a module resolution algorithm, import paths, and build tooling. These are non-trivial. The package manager is explicitly out of scope.

**Decision:**
v0.1 supports single-file programs and the stdlib. Multi-file support via relative `use` paths (e.g. `use ./helpers`) can be added as a thin module-resolution layer without a package manager and will be considered for v0.2. Full package management (versioned dependencies, a registry) is deferred past v0.2.

Rationale: for the agentic use case, single-file programs are the norm. Forcing a package layout on every project adds friction with no benefit at this stage. Relative imports are a natural extension when programs grow beyond a single file and do not require a package manager.

---

## Q6: Trait System — Scope and Syntax

**Status: Resolved**

**Question:**
How rich should the trait system be in v0.1?

**Context:**
The type system uses trait bounds in two places: generic type parameters (`T: Serialize + Eq`) and `extern` declarations. A minimal trait system needs: `Serialize`, `Deserialize`, `Eq`, `Hash`, `Ord`, `Display`. Whether user-defined traits are supported in v0.1 is unresolved.

**Decision:**
v0.1 supports a fixed set of built-in traits only: `Serialize`, `Deserialize`, `Eq`, `Hash`, `Ord`, `Display`, `Clone`, `Debug`. User-defined traits are deferred to v0.2. The compiler derives these traits automatically for records and enums whose fields all satisfy the relevant trait.

Rationale: user-defined traits require trait coherence rules, impl blocks, and potentially orphan rules — a significant chunk of type system machinery. The stdlib surface and the language spec both only require the listed built-in traits. Deferring keeps the v0.1 type checker tractable.

---

## Q7: Error Type Hierarchy — ToolError vs. Domain Errors

**Status: Resolved**

**Question:**
Should tool errors be isolated to `ToolError`, or should tools be able to return domain-specific error types?

**Context:**
The stdlib defines `ToolError` with variants for timeout, network failure, auth failure, etc. But a `tool web_search` returning `ToolError` loses information about what *semantically* went wrong (e.g. "no results found" is not a `ToolError` — it is a domain fact). Two options:
1. `ToolError` is the only error type for tools (transport-level only). Domain errors are encoded in the success value.
2. Tools return `Result<T, E>` where `E` is user-defined, and `ToolError` is one possible `E`.

**Decision:**
Option 2. A `tool` declaration's return type is `Result<T, E>` where `E` is specified by the user. `ToolError` is available in `lace.stdlib.tool` and is the conventional choice for transport failures, but is not forced. Users may define domain-specific error enums and use them directly as the error type.

Rationale: constraining tools to `ToolError` forces users to encode domain errors inside `Ok` values or add an extra `Result` layer, both of which are awkward. Making `E` user-defined gives tools the same expressive range as normal Lace functions while keeping `ToolError` as the idiomatic transport-layer error.

---

## Q8: @context_bounded — What Gets Counted?

**Status: Resolved**

**Question:**
What does "token consumption" mean at compile time? What is being counted — prompt tokens, completion tokens, total? Over what scope?

**Context:**
`@context_bounded(tokens: 2048)` is a compile-time annotation. For it to be meaningful, the compiler needs a model of token consumption. This is non-trivial:
- String literals contribute to prompt tokens
- LLM tool calls consume tokens based on inputs + outputs
- The annotation presumably bounds the total context window consumed by this function's execution

**Decision:**
For v0.1, `@context_bounded` is advisory rather than enforced:
- The annotation binds on **total tokens** (prompt + completion combined) for the annotated function's execution scope
- The compiler estimates a lower bound based on static string sizes and known-constant inputs and warns when a statically-detectable overrun is likely
- Dynamic token tracking is done via `lace.stdlib.context` at runtime; `context_assert()` is the enforceable runtime counterpart
- Full static enforcement is deferred to a later version when the call-graph token analysis is more mature

The annotation syntax is stable; only the enforcement level is advisory in v0.1.

Rationale: static token estimation for arbitrary LLM call graphs is an open research problem. Advisory semantics let the annotation be used productively in v0.1 — as documentation and a soft guard — without blocking the release on unsolved analysis.

---

## Q9: Deterministic Replay — Randomness and Time

**Status: Resolved**

**Question:**
How does the replay engine handle functions that consume randomness or wall-clock time?

**Context:**
`IO` functions include `now_unix()` and (implicitly) any RNG. During replay, returning the original timestamp/random value is correct for determinism. But the stdlib currently lumps time and randomness under `IO`. Should they have their own effect tags?

**Decision:**
Add two sub-tags:
- `Time` — functions that read the clock
- `Rand` — functions that consume randomness

Both are implicitly `IO` (a function declaring `[Time]` is also `[IO]`). A function that declares only `[IO]` does not need to re-declare `[Time]` or `[Rand]` unless it wants to signal the specific sub-effect. The runtime journals `Time` and `Rand` calls in separate journal entry categories so replay can substitute recorded values precisely.

User code rarely needs to declare `[Time]` or `[Rand]` directly — only stdlib functions (`now_unix`, `now_millis`, RNG calls) use them. Having them in the type system makes journal entries machine-readable and replay behavior unambiguous across all three categories.

Rationale: lumping time and randomness under plain `IO` makes replay code fragile — the replay engine has to special-case these functions by name rather than by type. Sub-tags make the distinction first-class and allow the journal to be self-describing.
