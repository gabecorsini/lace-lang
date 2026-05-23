# Lace Runtime Model
## Version 0.1 (Design Phase)

---

## 1. Overview

Lace has two execution modes:

1. **Interpreter (REPL mode)** — for interactive use, agent mid-task execution, and rapid iteration
2. **AOT Compiler** — for deployment as a static binary

Both modes share the same runtime core: the same type checker, effect enforcer, side-effect journal, and checkpoint/replay engine. The distinction is in how bytecode is produced and executed.

The runtime is written in Rust.

---

## 2. Interpreter (REPL Mode)

### 2.1 Purpose

The REPL is the primary execution mode for agents. An agent running a Lace program mid-task does not want to compile a binary — it wants to load a script, execute it, and resume control. The interpreter is designed for:

- Interactive sessions where the agent queries results and adjusts course
- Script execution without a compile step
- Testing and debugging tool declarations
- Incremental execution with checkpoints between steps

### 2.2 Architecture

```
Source (.lace file or inline string)
  |
  v
Lexer + Parser  -->  AST
  |
  v
Type Checker + Effect Verifier  -->  Typed AST
  |
  v
Interpreter Core  -->  Values + Side-effect Journal
  |
  v
Result / Checkpoint
```

The interpreter evaluates a Typed AST directly. No bytecode intermediate representation is used in v0.1; this is an explicit trade-off: simplicity over peak performance. The interpreter may be replaced with a bytecode VM in a later version if profiling demands it.

### 2.3 REPL Semantics

The REPL maintains a persistent environment across inputs:

```
lace> let x = 42
lace> fn double(n: Int) -> Int [Pure] { n * 2 }
lace> double(x)
84
lace> :checkpoint save my-point
Checkpoint saved: my-point
```

REPL commands (prefixed with `:`) are not Lace syntax — they are runtime directives:

| Command                   | Description                                      |
|---------------------------|--------------------------------------------------|
| `:checkpoint save <name>` | Persist current state to a named checkpoint      |
| `:checkpoint load <name>` | Restore state from a checkpoint                  |
| `:checkpoint list`        | List available checkpoints                       |
| `:replay <name>`          | Replay from checkpoint, re-executing journal     |
| `:journal`                | Show current side-effect journal                 |
| `:tools`                  | List declared tools and their current mock status |
| `:mock on / off`          | Toggle mock mode globally                        |
| `:dry-run on / off`       | Toggle dry-run mode globally                     |
| `:type <expr>`            | Show inferred type without evaluating            |
| `:effects <expr>`         | Show inferred effects without evaluating         |
| `:quit`                   | Exit the REPL                                    |

### 2.4 Performance Target

The interpreter is not optimized for throughput. It is optimized for:
- Fast startup (< 50ms to first prompt)
- Correct effect enforcement with useful error messages
- Reliable checkpoint/replay semantics

For performance-critical inner loops, use the `extern` FFI to call compiled Rust.

---

## 3. AOT Compilation

### 3.1 Purpose

AOT compilation produces a self-contained binary for:
- Deployment in automated pipelines (no runtime installation required)
- Performance-critical agent scripts
- Distribution as a standalone tool

### 3.2 Compilation Pipeline

```
Source (.lace files)
  |
  v
Lexer + Parser  -->  AST
  |
  v
Type Checker + Effect Verifier  -->  Typed AST
  |
  v
MIR (Mid-level IR — flat, typed, explicit control flow)
  |
  v
LLVM IR  (via Rust's inkwell or direct LLVM bindings)
  |
  v
Native Binary (.elf / .macho / .exe)
```

The MIR is a Lace-specific intermediate representation that:
- Flattens nested expressions into a 3-address form
- Makes all memory allocations explicit
- Eliminates closures (they are hoisted to named functions with an explicit environment struct)
- Lowers `|>` chains to explicit temporaries

MIR is an internal representation and is not part of the public spec. It is documented separately for compiler implementors.

### 3.3 Binary Output

The AOT binary:
- Statically links the Lace runtime (journal, checkpoint engine, effect tracker)
- Accepts runtime flags: `--mock`, `--dry-run`, `--checkpoint-dir <path>`, `--log-level <level>`
- Embeds the program's expected effect signatures for runtime verification (defense-in-depth against FFI violations)
- Produces structured logs (JSON by default, configurable)

### 3.4 Compile-Time Checks

The AOT compiler performs additional static analysis beyond what the interpreter enforces:

- **Context budget estimation** — for `@context_bounded(tokens: N)`, the compiler attempts to statically bound token consumption via call graph analysis. It warns when it cannot prove the bound is satisfied.
- **Effect completeness** — the compiler verifies that all `IO` and `ToolCall` paths are reachable via declared tool declarations (no orphaned tool references)
- **Dead code detection** — unreachable arms in `match` expressions are reported
- **Tool mock coverage** — warns if a `tool` has no `mock:` declared (useful for deployment-time audit)

---

## 4. Side-Effect Journal

### 4.1 Structure

The journal is a sequential log of all `IO` and `ToolCall` invocations during a run. Each entry contains:

```
JournalEntry {
    id: String,          -- deterministic hash of (run_id, sequence_num)
    run_id: String,
    seq: Int,            -- monotonically increasing
    timestamp: Int,      -- unix milliseconds
    effect: EffectTag,   -- IO | ToolCall
    fn_name: String,
    module: String,
    inputs: JSON,        -- serialized inputs
    output: JSON,        -- serialized return value (Ok or Err)
    duration_ms: Int,
}
```

The journal is written to disk as a line-delimited JSON file (`.lace-journal`). Each line is one `JournalEntry`. The file is append-only during a run.

### 4.2 Serialization

All inputs and outputs must be serializable to JSON for journaling. The type system enforces this via a `Serialize` trait:
- All primitive types satisfy `Serialize`
- All records and enums composed of serializable types satisfy `Serialize`
- `?`-typed values are serialized as raw JSON blobs

The compiler warns if a `tool` parameter or return type does not satisfy `Serialize`. This is a compile error in strict mode.

---

## 5. Checkpoints

### 5.1 What a Checkpoint Contains

A checkpoint is a persisted execution snapshot created at `@checkpoint`-annotated function boundaries (before and after the call). It contains:

```
Checkpoint {
    id: String,
    run_id: String,
    name: Option<String>,   -- user-assigned label (from :checkpoint save or @checkpoint(name:))
    timestamp: Int,
    journal_offset: Int,    -- offset into journal at checkpoint time
    env_snapshot: JSON,     -- serialized bindings in scope
    stack_frame: JSON,      -- current call stack state
    pending_effects: List<JournalEntry>,  -- effects since last checkpoint
}
```

### 5.2 Checkpoint Storage

Checkpoints are stored in a configurable directory (default: `./.lace-checkpoints/`). Each checkpoint is a single JSON file named `<run_id>-<seq>-<name?>.json`.

The runtime garbage-collects checkpoints older than a configurable TTL (default: 7 days).

### 5.3 Replay Protocol

Replay from checkpoint `C`:

1. Load `C`'s `env_snapshot` to restore bindings
2. Load the journal at offset `C.journal_offset`
3. Resume execution from the point after `C` was taken
4. For each subsequent `IO`/`ToolCall` encountered:
   - If a matching journal entry exists: return the recorded output (no side effect)
   - If not: execute the side effect, log the new entry
5. When execution reaches a point past the last journal entry: resume normal execution

This protocol ensures that:
- Deterministic re-execution produces the same results as the original run
- New code paths (edits after the checkpoint) execute live
- Tool calls that have already been made are not repeated

### 5.4 @checkpoint Annotation

```lace
@checkpoint
fn expensive_tool_step(data: Data) -> Result<Output, Error> [IO, ToolCall] { .. }
```

The runtime automatically creates a checkpoint before and after this function. If the function fails (returns `Err`), the pre-call checkpoint is retained. The post-call checkpoint is only written on `Ok`.

Optional named checkpoint:
```lace
@checkpoint(name: "post-enrichment")
fn enrich(data: Data) -> Result<EnrichedData, Error> [ToolCall] { .. }
```

---

## 6. Error Handling at the Runtime Level

The runtime does not use Rust panics for Lace-level errors. All errors are:
- Represented as `Result::Err(e)` values propagated through the program
- Logged to the journal with the full call context
- Reported to the runtime's error reporter (structured JSON to stderr by default)

The only runtime panics are internal consistency violations (bugs in the interpreter/compiler). These are reported as `LACE_INTERNAL_ERROR` with a bug report prompt.

---

## 7. Execution Modes Summary

| Mode        | Use Case                          | Startup | Peak Throughput | Checkpoint | Mock Support |
|-------------|-----------------------------------|---------|-----------------|------------|--------------|
| REPL        | Agent mid-task, interactive       | Fast    | Moderate        | Yes        | Yes          |
| AOT Binary  | Pipelines, deployment             | ~0ms    | High            | Yes        | Yes (flag)   |
| Dry-run     | Audit / testing                   | Fast    | N/A             | No         | Implied      |

---

## 8. Platform Targets

Initial release targets:
- Linux x86_64 (primary)
- macOS ARM64 (Apple Silicon)
- macOS x86_64

Windows support is deferred. The core runtime is written to avoid platform-specific code, so Windows support should require only CI and build tooling work.
