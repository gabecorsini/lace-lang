# Lace: Make It Real Roadmap

The goal: a language a serious developer could plausibly reach for when building agent tooling.
Not production-ready, but *credibly good*. Docs, ergonomics, error messages, stdlib — the works.

---

## Phase 9 ✓ (done)
- String stdlib (split, contains, trim, len, to_upper, to_lower, starts_with, ends_with)
- Full arithmetic (+, -, *, /, %, //)
- ? error propagation

---

## Phase 10 ✓ (done)
- `record` types: `record Response { status: Int, body: String }`
- Field access: `resp.status`
- File I/O stdlib: `File.read(path)`, `File.write(path, content)`, `File.exists(path)`
- `Result<T,E>` constructors: `Ok(v)`, `Err(e)` usable in user code
- Fixed type checker: `Named` ↔ `Record` compatibility + `FieldAccess` on named types
- Fixed effect checker: calling `[Pure]` functions no longer leaks Pure into caller's required effects
- Fixed effect checker: method-style module calls (e.g. `File.read`) now infer IO correctly
- Updated examples and tests

---

## Phase 11 — Match exhaustiveness + control flow
- Exhaustive `match` on variants and records (compiler error if arms missing)
- `if/else if/else` chains (not just binary if)
- `break` and `continue` in loops
- `return` from any function position
- Improve error messages: show source line + caret on parse/type errors

---

## Phase 12 — Map/Dict stdlib + closures
- `Map<K,V>` type: `Map.new()`, `Map.insert(m, k, v)`, `Map.get(m, k)`, `Map.contains(m, k)`, `Map.keys(m)`, `Map.values(m)`
- First-class closures / anonymous functions: `fn(x: Int) -> Int { x * 2 }`
- `List.fold`, `List.zip`, `List.flat_map`, `List.sort`, `List.reverse`
- `Option<T>` stdlib: `Option.map`, `Option.unwrap_or`

---

## Phase 13 — Package system + lace.toml
- `lace.toml` project manifest (name, version, dependencies)
- `lace new <project>` scaffold command
- Multi-file projects via `import` resolved from project root
- `lace build` → single binary output
- Namespace isolation between modules

---

## Phase 14 — Error messages & developer experience
- Rich error output: filename, line, column, caret pointing at the bad token
- "Did you mean X?" suggestions on unknown identifiers
- Warning system (unused variables, unreachable code)
- `lace fmt` — opinionated formatter
- `lace check` shows all errors, not just the first one

---

## Phase 15 — HTTP tool stdlib + agent primitives
- Built-in `Http.get(url)`, `Http.post(url, body)`, `Http.headers(req, map)`
- `Json.parse(str)`, `Json.stringify(val)`, `Json.get(val, key)`
- `Env.get(key)` for environment variables
- `Sleep.ms(n)` for delays
- `@retry` and `@timeout` decorators on tool declarations (runtime-enforced)
- Structured tool output: tools must return typed records, not Dynamic

---

## Phase 16 — Documentation + website
- `lace doc` command: generate HTML docs from annotated source
- `///` doc comments on functions, types, tools
- Publish a GitHub Pages site with:
  - Language tour (interactive examples)
  - Stdlib reference
  - "Why Lace?" page (honest about the origin story)
- Update README with badges, install instructions, quick demo GIF

---

## Phase 17 — LSP + VS Code extension
- Language Server Protocol implementation (hover, go-to-def, diagnostics)
- VS Code extension: syntax highlighting, inline errors, run button
- `.lace` file icon

---

## Success criteria
A developer who finds this repo should be able to:
1. Install lace in one command
2. Write a 50-line agent script that hits an HTTP API, parses JSON, and writes results to a file
3. Run `lace test` and see passing tests with clear output
4. Get a useful error message when they make a typo
5. Find stdlib docs without reading source code
