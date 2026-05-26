# Lace Language Support for VS Code

Provides rich language support for the [Lace](https://github.com/your-org/lace) programming language.

## Features

- **Syntax Highlighting** — keywords, types, strings with interpolation, comments, effect annotations
- **Diagnostics** — type errors and warnings via the `lace lsp` language server
- **Hover Info** — type information on hover

## Requirements

The `lace` binary must be installed and available on your `PATH`. The extension launches `lace lsp` as a stdio language server.

## Getting Started

1. Install the `lace` toolchain
2. Install this extension
3. Open any `.lace` file

## Syntax Overview

```lace
## Doc comment
# Line comment

const PI: Float = 3.14159

fn greet(name: String) -> String [Pure] {
  "Hello, ${name}!"
}

let result = greet("World")
```

## Effect Annotations

Effects appear after `->` in function signatures:
- `[IO]` — performs I/O
- `[Pure]` — no side effects
- `[ToolCall]` — calls an AI tool
