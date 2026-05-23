# Lace Open Questions
## Version 0.1 (Design Phase)

This document tracks architecturally unresolved questions. Each item has a status: **Deferred**, **Recommended** (recommendation made, awaiting decision), or **Resolved**.

---

## Q1: Effect Inference for Named Functions

**Status: Recommended**

**Question:**
Should Lace infer effects for named functions, or require explicit annotations on all named functions?

**Context:**
Effect inference is theoretically possible but has known trade-offs. Full inference can produce surprising effects at call sites (a function that "looks pure" infers a ToolCall because of a transitive callee three levels deep). Explicit annotations make effects visible at the declaration, which is the whole point of having them.

**Recommendation:**
Require explicit effect annotations on all named `fn` declarations. Allow inference only for anonymous closures, where the body is short and visible. A `#[infer_effects]` escape hatch could be added later if the friction proves too high in practice.

**Deferred to: Implementation phase.** The spec mandates explicit annotations. If the community pushes back during the implementation phase, revisit.

---

## Q2: Effect Polymorphism — Scope and Syntax

**Status: Deferred**

**Question:**
How expressive should effect polymorphism be in v0.1?

**Context:**
Full effect polymorphism (quantifying over effect sets in generic functions) requires non-trivial type system machinery. The use case is higher-order functions like `map`, `filter`, and `apply` that should carry the effects of their function arguments without hardcoding them.

Options:
1. No effect polymorphism — higher-order functions over effectful callables are not expressible. Workaround: duplicate functions for each effect combination. (Bad DX.)
2. Predefined effect variables — a small fixed set of effect variable names (`efx`, `efx1`, `efx2`) with constraint syntax. Simpler to implement.
3. Full row polymorphism — effects are a row type; arbitrary composition. Correct but complex.

**Recommendation:**
Option 2 for v0.1. Define a small set of effect variables with simple composition syntax. Full row polymorphism can be introduced in a later version.

**Deferred to: Implementation phase.** Needs prototyping to validate syntax ergonomics.

---

## Q3: Uncertain\<\[T\]\> — Confidence Scores

**Status: Recommended**

**Question:**
Should `Uncertain<[T]>` carry confidence scores alongside the candidates, or leave scoring to the contained type?

**Context:**
If `Uncertain<[SearchResult]>` doesn't carry scores, then `SearchResult` must embed a score field. But what if the user doesn't want scores on `SearchResult` itself? A wrapper `Scored<T> { value: T, score: Float }` is one option. Another is `Uncertain<[T]>` always storing `List<Scored<T>>` internally.

**Recommendation:**
`Uncertain<[T]>` stores `List<T>` where `T` is the user's type. Scores are the user's responsibility to embed if needed. Provide a `Scored<T>` helper type in the stdlib:

```lace
record Scored<T> {
    value: T,
    score: Float,
}
```

A typical LLM classification step would return `Uncertain<[Scored<Category>]>`.

This keeps `Uncertain` general and avoids baking score semantics into the core type.

**Awaiting: Final design review before implementation.**

---

## Q4: Mutable References vs. Ownership

**Status: Deferred**

**Question:**
How does Lace handle mutable data? Does it adopt Rust-style ownership/borrowing, or a simpler model?

**Context:**
The `[Mut]` effect marks mutation. But the question is: how is mutable state passed and tracked at the value level? Options:
1. Rust-style borrow checker — correct, safe, but significant learning curve
2. Copy-on-write semantics — every assignment copies; `Mut` is syntactic sugar for "this function replaces its argument"
3. Explicit mutable reference type (`&mut T`) — simpler than full Rust but similar in spirit
4. Pure value model — no mutation; `Mut` is reserved for external state only

**Recommendation:**
Option 4 for v0.1: Lace values are immutable. `mut let` bindings are rebindable (the variable can point to a new value) but do not allow in-place mutation of data structures. The `[Mut]` effect is reserved for external state mutation (writing to shared data stores, modifying global registries). This sidesteps the borrow checker complexity entirely for v0.1 while remaining sound.

**Deferred to: Implementation phase.** The choice significantly affects the type system implementation.

---

## Q5: Module System — Single File vs. Package

**Status: Deferred**

**Question:**
Is a Lace "program" always a single file, or does the language support multi-file packages in v0.1?

**Context:**
Multi-file packages require a module resolution algorithm, import paths, and build tooling. These are non-trivial. The package manager is explicitly out of scope.

**Recommendation:**
v0.1 supports single-file programs + the stdlib. Multi-file support (local imports from relative paths) can be added as a thin layer over the module system without a package manager. Full package management is deferred.

**Deferred to: Implementation phase.**

---

## Q6: Trait System — Scope and Syntax

**Status: Deferred**

**Question:**
How rich should the trait system be in v0.1?

**Context:**
The type system uses trait bounds in two places: generic type parameters (`T: Serialize + Eq`) and `extern` declarations. A minimal trait system needs: `Serialize`, `Deserialize`, `Eq`, `Hash`, `Ord`, `Display`. Whether user-defined traits are supported in v0.1 is unresolved.

**Recommendation:**
v0.1 supports a fixed set of built-in traits (listed above + `Clone`, `Debug`). User-defined traits are deferred. This is sufficient for the standard library surface spec and keeps the type system tractable.

**Deferred to: Implementation phase.**

---

## Q7: Error Type Hierarchy — ToolError vs. Domain Errors

**Status: Recommended**

**Question:**
Should tool errors be isolated to `ToolError`, or should tools be able to return domain-specific error types?

**Context:**
The stdlib defines `ToolError` with variants for timeout, network failure, auth failure, etc. But a `tool web_search` returning `ToolError` loses information about what *semantically* went wrong (e.g. "no results found" is not a `ToolError` — it is a domain fact). Two options:
1. `ToolError` is the only error type for tools (transport-level only). Domain errors are encoded in the success value.
2. Tools return `Result<T, E>` where `E` is user-defined, and `ToolError` is one possible `E`.

**Recommendation:**
Option 2. The `tool` declaration's return type is `Result<T, E>` where `E` is specified by the user. `ToolError` is available in `lace.stdlib.tool` and is the conventional choice for transport failures, but is not forced. This gives tools the expressive range of normal Lace functions.

**Awaiting: Validation against stdlib surface spec implementation.**

---

## Q8: @context_bounded — What Gets Counted?

**Status: Deferred**

**Question:**
What does "token consumption" mean at compile time? What is being counted — prompt tokens, completion tokens, total? Over what scope?

**Context:**
`@context_bounded(tokens: 2048)` is a compile-time annotation. For it to be meaningful, the compiler needs a model of token consumption. This is non-trivial:
- String literals contribute to prompt tokens
- LLM tool calls consume tokens based on inputs + outputs
- The annotation presumably bounds the total context window consumed by this function's execution

**Recommendation:**
For v0.1, `@context_bounded` is advisory rather than enforced:
- The compiler estimates a lower bound based on static string sizes and known-constant inputs
- It warns when a statically-detectable overrun is likely
- Dynamic token tracking is done via `lace.stdlib.context` at runtime
- Full static enforcement is deferred to a later version when the analysis is more mature

**Deferred to: Post-v0.1.** The annotation syntax is stable; the enforcement level is not.

---

## Q9: Deterministic Replay — Randomness and Time

**Status: Recommended**

**Question:**
How does the replay engine handle functions that consume randomness or wall-clock time?

**Context:**
`IO` functions include `now_unix()` and (implicitly) any RNG. During replay, returning the original timestamp/random value is correct for determinism. But the stdlib currently lumps time and randomness under `IO`. Should they have their own effect tags?

**Recommendation:**
Add two sub-tags:
- `Time` — functions that read the clock
- `Rand` — functions that consume randomness

Both are implicitly `IO` (a function declaring `[Time]` is also `[IO]`). The runtime journals `Time` and `Rand` calls separately so replay can substitute recorded values precisely.

User code rarely needs to declare `[Time]` or `[Rand]` directly — only stdlib functions use them. But having them in the type system makes the journal entries machine-readable and the replay behavior unambiguous.

**Awaiting: Review against the effect system spec.**
