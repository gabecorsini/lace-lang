# Lace — Glossary

**Tool** — An external callable with a typed signature, declared with the `tool` keyword. Tools are mockable, retryable, and logged by default.

**Effect type** — A type annotation on functions declaring what side-effects they produce: `IO`, `Mut`, `ToolCall`, or `Pure`. Used by the runtime for sandboxing, retry, and parallelism decisions.

**Pipeline** — A chain of operations composed with the `|>` operator. Pipelines surface errors at collection points rather than crashing mid-chain.

**Confident<T>** — A result type indicating high-certainty output from a reasoning step.

**Uncertain<[T]>** — A result type indicating the agent is not sure; the caller must handle ambiguity explicitly.

**Result<T, E>** — Standard fallible return type. No exceptions — errors are values.

**Option<T>** — Nullable replacement. No null in Lace — ever.

**Context budget** — The token limit a function is allowed to consume. Annotated via `@context_bounded(tokens: N)`. Compiler warns on potential overruns.

**Replay** — Deterministic re-execution of a past run from a checkpoint. Enabled by the runtime's side-effect log.

**Checkpoint** — A persisted snapshot of execution state at a side-effect boundary. Used for replay and crash recovery.

**AOT** — Ahead-of-time compilation to a single static binary for deployment.

**REPL** — Interactive interpreted mode for agent use mid-task.
