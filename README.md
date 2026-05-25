# Lace 🧵

> A programming language I don't fully understand, built by an AI I asked nicely.

---

I saw [Vercel ship Zero](https://github.com/vercel-labs/zerolang) — a programming language built by one of the best engineering teams in the world.

I am not that. I'm an identity security consultant who thinks about Entra ID all day. I don't know how compilers work. I've never written a lexer. I have no business making a programming language.

So naturally, I made one anyway. I just asked my AI agent (Hermes) to do it while I made coffee.

This is Lace.

---

## What even is this?

Lace is a statically-typed, effect-annotated scripting language designed for agentic workloads — built to run on headless servers, Raspberry Pis, and anywhere else you'd want a small program to do real things reliably.

The core idea: **`tool` and `fn` are different things.** A `fn` is pure computation. A `tool` does I/O — HTTP calls, file reads, external APIs. Tools are automatically logged, retryable, and auditable. You don't have to instrument anything. The runtime does it.

**What's in the box:**

- `tool` declarations — distinct from `fn`, effect-enforced, auto-logged to structured JSON
- `@retry(max: N)` and `@timeout(ms: N)` decorators on any tool
- Static type checker with multi-error reporting, error codes (E001–E005), and "did you mean?" suggestions
- Full stdlib: `List`, `String`, `Map`, `Math`, `Http`, `Json`, `Env`, `Regex`, `Time`, `File`, `Process`, `Async`
- Pipeline syntax (`|>`) for readable data transforms
- `Option<T>` and `Result<T, E>` with `?` propagation — no nulls, no exceptions
- Records, closures, pattern matching, `break`/`continue`/`return`
- Multi-file modules + `lace.toml` project manifest
- `lace fmt` — formatter
- `lace doc` — generates a dark-themed HTML docs site from `///` doc comments
- `lace explain E001` — explains any error code with an example
- LSP server (`lace lsp`) with hover, completion, diagnostics, go-to-def, and formatting
- Works in Neovim, Helix, and Zed

I didn't write any of this. Hermes did. Over several sessions. Using GitHub Copilot (claude-sonnet-4.6), because I switched from Claude Code after it hit a rate limit and annoyed me.

---

## Installation

You'll need [Rust](https://rustup.rs) (stable).

```bash
git clone git@github.com:gabecorsini/lace-lang.git
cd lace-lang
cargo install --path crates/lace-cli
```

This installs the `lace` binary to `~/.cargo/bin/`. If that's not on your `$PATH` yet:

```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

Verify it worked:

```bash
lace --version
```

### Update

Pull the latest and reinstall — `cargo install` overwrites the old binary:

```bash
git pull
cargo install --path crates/lace-cli
```

### Uninstall

```bash
cargo uninstall lace-cli
```

---

## Quick start

```bash
# Run a file
lace run examples/weather.lace

# Start a new project
lace new myproject
cd myproject
lace run src/main.lace

# Type-check without running
lace check src/main.lace

# Compile to bytecode
lace compile src/main.lace

# Bundle into a standalone binary
lace bundle src/main.lace
```

---

## Example

```lace
## Fetch and parse a user from an API.
## Retries up to 3 times on failure.
@retry(max: 3)
tool fetch_user(id: Int) -> Result<String, String> {
    let response = Http.get("https://api.example.com/users/" ++ id.to_string())?
    let data = Json.parse(response)?
    Json.get(data, "name")
      |> Option.unwrap_or("unknown")
      |> Ok
}

fn greet(name: String) -> String {
    "Hello, " ++ name ++ "!"
}

let result = fetch_user(42)
match result {
    Ok(name) => print(greet(name)),
    Err(e)   => print("failed: " ++ e),
}
```

When `fetch_user` runs, the runtime automatically emits structured logs:
```json
{"event":"tool_call","tool":"fetch_user","args":[42],"timestamp":1748123456789}
{"event":"tool_ok","tool":"fetch_user","duration_ms":142}
```

No instrumentation required.

---

## Editor setup (LSP)

**Neovim** — see `editors/nvim-lspconfig.lua`  
**Helix** — see `editors/helix-languages.toml`  
**Zed** — see `editors/zed-extension.json`

All editors use the same command: `lace lsp`

---

## CLI reference

| Command | What it does |
|---|---|
| `lace run <file>` | Run a `.lace` file |
| `lace check <file>` | Type-check without running |
| `lace build` | Type-check whole project |
| `lace fmt <file>` | Format in place |
| `lace doc [path]` | Generate HTML docs |
| `lace explain <code>` | Explain an error code (e.g. E001) |
| `lace lsp` | Start the LSP server |
| `lace new <name>` | Scaffold a new project |

Flags: `--no-warn` suppresses warnings. `--no-tool-log` suppresses runtime tool logs.

---

## Roadmap

### Done
- [x] Lexer, parser, AST
- [x] Type checker — multi-error, error codes, did-you-mean suggestions
- [x] Tree-walking interpreter
- [x] Records, closures, match, break/continue/return
- [x] `?` error propagation
- [x] Full stdlib: `List`, `String`, `Map`, `Math`, `Http`, `Json`, `Regex`, `Time`, `File`, `Process`, `Async`
  - `Http.get`, `Http.get_with_headers`, `Http.serve`, `Http.serve_routes`
  - `Json.validate`, `Json.parse` (returns structured `Map`/`List`)
  - `File.read`, `File.write`, `File.append`
  - `Process.run`, `Process.env`
  - `Time.now`, `Time.parse`, `Time.parse_date`, `Time.format`
  - `Async.spawn`, `Async.await_handle`, `Async.all`, `Async.race`
- [x] List HOFs: `map`, `filter`, `fold`, `reduce`, `sort_by`, `find`, `any`, `all`, `for_each`, `join`, `filter_map`
- [x] `Map.keys`, `Map.values`, `Map.get`, `Map.insert`
- [x] Generics / parametric types (`fn identity(x: T) -> T`, `List<T>`)
- [x] String interpolation (`"Hello ${name}, score: ${score}!"`)
- [x] `lace run --quiet` / `-q` flag — raw output for scripting/cron use
- [x] "Did you mean?" suggestions on unknown identifiers (Levenshtein, 88-method table)

### Planned
- [ ] `tool` keyword — effect enforcement + automatic structured JSON logging
- [ ] `@retry(max: N)` / `@timeout(ms: N)` decorators
- [ ] Multi-file modules + `lace.toml` project manifest
- [ ] `lace fmt` — auto-formatter
- [ ] `lace doc` — HTML docs from `///` comments
- [ ] `lace explain <code>` — error code explainer
- [ ] `lace new <name>` — project scaffolding
- [ ] LSP server (hover, completion, diagnostics, go-to-def)
- [ ] Bytecode VM (faster, Pi-portable, produces `.lacec` files)
- [ ] `lace bundle` — single self-contained binary for deployment
- [ ] Package registry (`lace add`)
- [ ] Native compilation via Cranelift

---

## Docs

[🌐 Full docs & tutorial](https://gabecorsini.github.io/lace-lang/) — language tour, stdlib reference, and examples.  
[📄 In-repo docs](docs/) — spec, stdlib surface, and raw reference files tracked alongside the source.

---

## Why "Lace"?

**L**ogic + **A**ction + **C**omposition **E**ngine.

Hermes named it. I thought it sounded cool. We kept it.

---

## License

Apache-2.0

Built by [Hermes](https://hermes-agent.nousresearch.com). Directed by someone who just watched it happen.
