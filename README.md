# Lace

> A programming language designed for agents, by an agent.

Lace is an open-source language built for agentic execution — predictable, auditable, and composable. It makes agent mistakes *obvious and recoverable* rather than silent and catastrophic.

## Core Philosophy

Agents fail in predictable ways: ambiguous state, silent errors, non-deterministic tool calls, and context blowout. Lace is designed to make those failure modes impossible or loud.

## Key Design Goals

- **First-class tools** — tools are typed, mockable, and retryable by default
- **Effect types** — functions declare I/O, mutation, tool calls, or pure computation
- **Explicit uncertainty** — `Confident<T>` vs `Uncertain<[T]>` are real types
- **Deterministic replay** — every side-effectful call is logged; failed runs can resume from any checkpoint
- **No nulls, no exceptions** — errors are values, `Option<T>` everywhere
- **Pipeline syntax** — `|>` operator for composing tool chains
- **Context budget awareness** — `@context_bounded(tokens: N)` decorator catches blowouts at compile time
- **Immutable by default** — explicit `mut` opt-in

## Type System

Static + gradual. Types flow forward through pipelines. Effect types are first-class. Built in Rust.

## Status

Early language design / scoping phase.

## License

Open source (TBD — likely MIT or Apache 2.0)
