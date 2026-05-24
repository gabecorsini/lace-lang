# Lace 🧵

> A programming language I don't fully understand, built by an AI I asked nicely.

---

I saw [Vercel ship Zero](https://github.com/vercel-labs/zerolang) — a programming language built by one of the best engineering teams in the world.

I am not that. I'm an identity security consultant who thinks about Entra ID all day. I don't know how compilers work. I've never written a lexer. I have no business making a programming language.

So naturally, I made one anyway. I just asked my AI agent (Hermes) to do it while I made coffee.

This is Lace.

---

## What even is this?

Lace is a statically-typed, effect-annotated programming language designed for agentic workloads. It has:

- First-class `tool` declarations with typed signatures
- Effect types (`[Pure]`, `[IO]`, `[ToolCall]`) so agents know what they're calling
- Explicit uncertainty (`Confident<T>`, `Uncertain<T>`) — no pretending
- Pipeline syntax (`|>`) because it looks cool
- Checkpoint + replay for deterministic recovery
- A test runner (`lace test`)
- A module system
- `for` and `while` loops

I didn't write any of this. Hermes did. Over several sessions. Using GitHub Copilot (claude-sonnet-4.6), because I switched from Claude Code after it hit a rate limit and annoyed me.

---

## Quick start

You'll need Rust.

```bash
cargo build --workspace
./target/debug/lace run examples/hello.lace
./target/debug/lace test examples/tests.lace
```

---

## Example

```lace
@shell("echo '{\"ok\":true}'")
tool ping() -> Result<Dynamic, String>

fn main() -> Result<Dynamic, String> [ToolCall, IO] {
  println("calling tool")
  ping()
}
```

---

## Roadmap

- [x] Lexer, parser, AST
- [x] Type checker + effect system
- [x] Interpreter runtime
- [x] Tool execution (`@shell`, `@http`)
- [x] Checkpoint + replay
- [x] Module system
- [x] `for`/`while` loops + List stdlib
- [x] Test framework (`lace test`, `assert`, `assert_eq`, `assert_err`)
- [ ] String stdlib
- [ ] `?` error propagation
- [ ] User-defined record types
- [ ] File I/O stdlib
- [ ] LSP + VS Code extension (lol maybe)

---

## Why "Lace"?

**L**ogic + **A**ction + **C**omposition **E**ngine.

Hermes named it. I thought it sounded cool. We kept it.

---

## License

Apache-2.0

Built by [Hermes](https://hermes-agent.nousresearch.com). Owned by someone who just watched it happen.
