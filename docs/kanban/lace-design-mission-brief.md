# Lace — Language Design Mission Brief

## What is Lace?

Lace is an open-source programming language designed for agentic execution. It is being built by Hermes (an AI agent) at the direction of Gabe Corsini.

The name is intentional: it implies structure, threading-together, and composability — all of which are core to the language's purpose.

It is also, partially, a meme: Vercel shipped "Zero" (a new language) and Gabe thinks it's kinda stupid. Lace is the counterpunch — a language that actually earns its existence by solving real problems agents face.

## Problem Statement

Agents using general-purpose languages (Python, TypeScript) fail in predictable ways:
- Ambiguous state after partial execution
- Silent type coercion errors
- No standard model for tool calls
- No first-class uncertainty representation
- No context budget enforcement
- Non-deterministic behavior on retries

Lace makes these failure modes **impossible or loud at the type/effect level**.

## In-Scope for Initial Mission (Language Design + Spec)

1. Language specification document
2. Type system design (static + gradual, effect types, `Option`, `Result`, `Confident`, `Uncertain`)
3. Syntax grammar definition (EBNF or similar)
4. Core standard library surface (tool declarations, pipeline ops, retry/fallback decorators)
5. Runtime model (interpreted REPL + AOT compilation to native binary)
6. Implementation language decision (strong prior: Rust)
7. Open-source project structure (repo, license, CONTRIBUTING.md)

## Out of Scope for Now

- Actual compiler implementation
- Package manager
- LSP / IDE tooling
- Community / governance

## Architecture Priors (Already Decided by Design Session)

- **Static + gradual typing** (statically typed core with `?` escape hatch)
- **Effect types** on all functions (IO, Mut, ToolCall, Pure)
- **No nulls** — `Option<T>` only
- **No exceptions** — errors are values (`Result<T, E>`)
- **Pipeline operator** `|>`
- **Immutable by default**, explicit `mut`
- **Deterministic replay** via side-effect log + checkpoints
- **`@context_bounded(tokens: N)`** compile-time annotation
- **`Confident<T>` / `Uncertain<[T]>`** as first-class uncertainty types
- **Built in Rust**
- **First-class `tool` keyword** for typed, mockable, retryable tool declarations

## Definition of Done (PM Spec Phase)

- Full language spec document with all major type constructs and syntax examples
- EBNF grammar (or equivalent) for the Lace syntax
- Effect system design doc with runtime implications
- Standard library surface spec (tool, pipeline, uncertainty, retry primitives)
- Runtime model doc (interpreter + AOT decision)
- Repo structure recommendation (crates layout, CI, license)
- Open questions surfaced and resolved (or explicitly deferred)

## Project Path

`/home/hermes/projects/lace`

## Kanban Tenant

`lace`
