use std::io::Write;

use lace_ast::{FnDecl, Param, PrimitiveType, ToolDecl, TopLevelItem, TypeExpr};
use lace_parser::{offset_to_line_col, parse_program};
use lace_types::check_program_full;
use serde_json::{json, Value};

use crate::document::DocumentStore;

pub struct LspServer<W: Write> {
    out: W,
    docs: DocumentStore,
    initialized: bool,
}

impl<W: Write> LspServer<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            docs: DocumentStore::default(),
            initialized: false,
        }
    }

    // ── I/O ────────────────────────────────────────────────────────────────

    fn send(&mut self, msg: Value) {
        let body = serde_json::to_string(&msg).unwrap_or_default();
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let _ = self.out.write_all(header.as_bytes());
        let _ = self.out.write_all(body.as_bytes());
        let _ = self.out.flush();
    }

    fn respond(&mut self, id: &Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn respond_null(&mut self, id: &Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": null }));
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    // ── Dispatch ───────────────────────────────────────────────────────────

    pub fn handle(&mut self, msg: Value) {
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        match method.as_str() {
            "initialize" => self.on_initialize(id.as_ref().unwrap()),
            "initialized" => self.initialized = true,
            "shutdown" => {
                if let Some(id) = &id {
                    self.respond_null(id);
                }
            }
            "exit" => std::process::exit(0),
            "textDocument/didOpen" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                let text = params["textDocument"]["text"].as_str().unwrap_or("").to_string();
                self.docs.open(uri.clone(), text);
                self.publish_diagnostics(&uri);
            }
            "textDocument/didChange" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                // Full sync: take last content change
                if let Some(changes) = params["contentChanges"].as_array() {
                    if let Some(last) = changes.last() {
                        let text = last["text"].as_str().unwrap_or("").to_string();
                        self.docs.update(&uri, text);
                    }
                }
                self.publish_diagnostics(&uri);
            }
            "textDocument/didClose" => {
                let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                self.docs.close(uri);
            }
            "textDocument/hover" => {
                let result = id.as_ref().map(|id| self.on_hover(id, &params));
                if let (Some(id), Some(result)) = (id.as_ref(), result) {
                    self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
                }
            }
            "textDocument/completion" => {
                if let Some(id) = &id {
                    let result = self.on_completion(&params);
                    self.respond(id, result);
                }
            }
            "textDocument/definition" => {
                if let Some(id) = &id {
                    let result = self.on_definition(&params);
                    self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
                }
            }
            _ => {
                // Unknown request with id — send null result to avoid stalling clients
                if let Some(id) = &id {
                    if msg.get("method").is_some() {
                        self.respond_null(id);
                    }
                }
            }
        }
    }

    // ── Handlers ───────────────────────────────────────────────────────────

    fn on_initialize(&mut self, id: &Value) {
        self.respond(
            id,
            json!({
                "capabilities": {
                    "textDocumentSync": 1,
                    "hoverProvider": true,
                    "completionProvider": {
                        "triggerCharacters": [".", " "]
                    },
                    "definitionProvider": true
                },
                "serverInfo": {
                    "name": "lace-lsp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        );
    }

    // ── Diagnostics ────────────────────────────────────────────────────────

    fn publish_diagnostics(&mut self, uri: &str) {
        let source = match self.docs.get(uri) {
            Some(s) => s.to_string(),
            None => return,
        };

        let mut diagnostics = Vec::new();

        let (program, parse_errors) = parse_program(&source);

        // Parse errors → diagnostics
        for e in &parse_errors {
            use lace_parser::ParseError;
            let (span_start, span_end) = match e {
                ParseError::Message { span_start, span_end, .. } => (*span_start, *span_end),
            };
            let (sl, sc) = offset_to_line_col(&source, span_start);
            let (el, ec) = offset_to_line_col(&source, span_end.max(span_start));
            diagnostics.push(json!({
                "range": lsp_range(sl, sc, el, ec),
                "severity": 1,
                "source": "lace",
                "message": e.to_string()
            }));
        }

        // Type errors → diagnostics
        if let Some(prog) = program {
            let (type_errors, _warnings) = check_program_full(&prog);
            for e in &type_errors {
                let (span_start, span_end) = type_error_span(e);
                let (sl, sc) = offset_to_line_col(&source, span_start);
                let (el, ec) = offset_to_line_col(&source, span_end.max(span_start));
                diagnostics.push(json!({
                    "range": lsp_range(sl, sc, el, ec),
                    "severity": 1,
                    "code": e.code(),
                    "source": "lace",
                    "message": e.to_string()
                }));
            }
        }

        let uri = uri.to_string();
        self.notify(
            "textDocument/publishDiagnostics",
            json!({ "uri": uri, "diagnostics": diagnostics }),
        );
    }

    // ── Hover ──────────────────────────────────────────────────────────────

    fn on_hover(&mut self, id: &Value, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let line = params["position"]["line"].as_u64().unwrap_or(0) as usize;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as usize;

        let source = match self.docs.get(uri) {
            Some(s) => s.to_string(),
            None => return Value::Null,
        };

        let word = word_at(&source, line, character);
        if word.is_empty() {
            return Value::Null;
        }

        let (program, _) = parse_program(&source);
        let prog = match program {
            Some(p) => p,
            None => return Value::Null,
        };

        for item in &prog.items {
            match item {
                TopLevelItem::Function(f) if f.name == word => {
                    let sig = fn_signature(f);
                    return json!({
                        "contents": { "kind": "markdown", "value": format!("```lace\n{sig}\n```") }
                    });
                }
                TopLevelItem::Tool(t) if t.name == word => {
                    let sig = tool_signature(t);
                    return json!({
                        "contents": { "kind": "markdown", "value": format!("```lace\n{sig}\n```") }
                    });
                }
                _ => {}
            }
        }

        // Fallback: keyword doc
        if let Some(doc) = keyword_doc(&word) {
            return json!({
                "contents": { "kind": "markdown", "value": doc }
            });
        }

        Value::Null
    }

    // ── Completion ─────────────────────────────────────────────────────────

    fn on_completion(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let source = self.docs.get(uri).map(|s| s.to_string());

        let mut items: Vec<Value> = Vec::new();

        // User-defined fns/tools from document
        if let Some(src) = &source {
            let (program, _) = parse_program(src);
            if let Some(prog) = program {
                for item in &prog.items {
                    match item {
                        TopLevelItem::Function(f) => {
                            items.push(json!({
                                "label": f.name,
                                "kind": 3, // Function
                                "detail": fn_signature(f),
                                "insertText": f.name
                            }));
                        }
                        TopLevelItem::Tool(t) => {
                            items.push(json!({
                                "label": t.name,
                                "kind": 3,
                                "detail": tool_signature(t),
                                "insertText": t.name
                            }));
                        }
                        _ => {}
                    }
                }
            }
        }

        // Static stdlib + keyword completions
        for (label, detail) in stdlib_completions() {
            items.push(json!({
                "label": label,
                "kind": 3,
                "detail": detail,
                "insertText": label
            }));
        }

        for kw in KEYWORDS {
            items.push(json!({
                "label": kw,
                "kind": 14, // Keyword
                "insertText": kw
            }));
        }

        json!({ "isIncomplete": false, "items": items })
    }

    // ── Go-to-definition ───────────────────────────────────────────────────

    fn on_definition(&mut self, params: &Value) -> Value {
        let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
        let line = params["position"]["line"].as_u64().unwrap_or(0) as usize;
        let character = params["position"]["character"].as_u64().unwrap_or(0) as usize;

        let source = match self.docs.get(uri) {
            Some(s) => s.to_string(),
            None => return Value::Null,
        };

        let word = word_at(&source, line, character);
        if word.is_empty() {
            return Value::Null;
        }

        let (program, _) = parse_program(&source);
        let prog = match program {
            Some(p) => p,
            None => return Value::Null,
        };

        for item in &prog.items {
            let span = match item {
                TopLevelItem::Function(f) if f.name == word => Some(f.span),
                TopLevelItem::Tool(t) if t.name == word => Some(t.span),
                _ => None,
            };
            if let Some(span) = span {
                let (sl, sc) = offset_to_line_col(&source, span.start);
                let (el, ec) = offset_to_line_col(&source, span.end);
                return json!({
                    "uri": uri,
                    "range": lsp_range(sl, sc, el, ec)
                });
            }
        }

        // Fallback: line scan for `fn word` / `tool word`
        for (i, src_line) in source.lines().enumerate() {
            let trimmed = src_line.trim_start();
            if trimmed.starts_with(&format!("fn {word}"))
                || trimmed.starts_with(&format!("pub fn {word}"))
                || trimmed.starts_with(&format!("tool {word}"))
                || trimmed.starts_with(&format!("pub tool {word}"))
            {
                return json!({
                    "uri": uri,
                    "range": lsp_range(i + 1, 1, i + 1, src_line.len() + 1)
                });
            }
        }

        Value::Null
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn lsp_range(sl: usize, sc: usize, el: usize, ec: usize) -> Value {
    // LSP lines/chars are 0-based; our offset_to_line_col returns 1-based
    json!({
        "start": { "line": sl.saturating_sub(1), "character": sc.saturating_sub(1) },
        "end":   { "line": el.saturating_sub(1), "character": ec.saturating_sub(1) }
    })
}

/// Extract the identifier word at (line, character) in source (0-based line).
fn word_at(source: &str, line: usize, character: usize) -> String {
    let src_line = source.lines().nth(line).unwrap_or("");
    let chars: Vec<char> = src_line.chars().collect();
    let ch = character.min(chars.len().saturating_sub(1));

    // Expand left
    let mut start = ch;
    while start > 0 && is_ident(chars[start - 1]) {
        start -= 1;
    }
    // Expand right
    let mut end = ch;
    while end < chars.len() && is_ident(chars[end]) {
        end += 1;
    }
    chars[start..end].iter().collect()
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn type_expr_str(te: &TypeExpr) -> String {
    match te {
        TypeExpr::Primitive(p, _) => match p {
            PrimitiveType::Int => "Int".to_string(),
            PrimitiveType::Float => "Float".to_string(),
            PrimitiveType::Bool => "Bool".to_string(),
            PrimitiveType::String => "String".to_string(),
            PrimitiveType::Bytes => "Bytes".to_string(),
            PrimitiveType::Unit => "()".to_string(),
        },
        TypeExpr::Named { name, .. } => name.clone(),
        TypeExpr::Generic { name, args, .. } => {
            let a: Vec<_> = args.iter().map(type_expr_str).collect();
            format!("{}<{}>", name, a.join(", "))
        }
        TypeExpr::Tuple { elems, .. } => {
            let inner: Vec<_> = elems.iter().map(type_expr_str).collect();
            format!("({})", inner.join(", "))
        }
        TypeExpr::Function { params, ret, .. } => {
            let ps: Vec<_> = params.iter().map(type_expr_str).collect();
            format!("fn({}) -> {}", ps.join(", "), type_expr_str(ret))
        }
        TypeExpr::Dynamic(_) => "Any".to_string(),
    }
}

fn param_str(p: &Param) -> String {
    format!("{}: {}", p.name, type_expr_str(&p.ty))
}

fn fn_signature(f: &FnDecl) -> String {
    let params: Vec<_> = f.params.iter().map(param_str).collect();
    let ret = f.ret_ty.as_ref().map(type_expr_str).unwrap_or_else(|| "()".to_string());
    format!("fn {}({}) -> {}", f.name, params.join(", "), ret)
}

fn tool_signature(t: &ToolDecl) -> String {
    let params: Vec<_> = t.params.iter().map(|p| {
        format!("{}: {}", p.name, type_expr_str(&p.ty))
    }).collect();
    let ret = type_expr_str(&t.ret_ty);
    format!("tool {}({}) -> {}", t.name, params.join(", "), ret)
}

fn type_error_span(e: &lace_types::TypeError) -> (usize, usize) {
    use lace_types::TypeError::*;
    match e {
        UnknownIdentifier { span_start, span_end, .. } => (*span_start, *span_end),
        Mismatch { span_start, span_end, .. } => (*span_start, *span_end),
        UnknownFunction { span_start, span_end, .. } => (*span_start, *span_end),
        InvalidPattern { span_start, span_end, .. } => (*span_start, *span_end),
        NonExhaustiveMatch { span_start, span_end, .. } => (*span_start, *span_end),
        UnknownRecordType { .. } | InvalidToolDecl { .. } => (0, 0),
    }
}

fn keyword_doc(word: &str) -> Option<&'static str> {
    match word {
        "fn" => Some("**fn** — declare a function\n\n```lace\nfn name(param: Type) -> RetType { ... }\n```"),
        "tool" => Some("**tool** — declare an LLM tool\n\n```lace\ntool name(param: Type) -> RetType { ... }\n```"),
        "let" => Some("**let** — bind a value\n\n```lace\nlet x = expr\n```"),
        "if" => Some("**if** — conditional expression"),
        "match" => Some("**match** — pattern matching"),
        "for" => Some("**for** — iterate over a list"),
        "return" => Some("**return** — return early from a function"),
        "true" | "false" => Some("Boolean literal"),
        _ => None,
    }
}

const KEYWORDS: &[&str] = &[
    "fn", "tool", "let", "if", "else", "match", "for", "in", "return",
    "true", "false", "pub", "use", "import", "module", "record", "enum",
    "type", "const", "extern", "as", "and", "or", "not",
];

/// Public helper for tests: parse + typecheck `source` and return diagnostics as JSON values.
pub fn compute_diagnostics(source: &str) -> Vec<serde_json::Value> {
    use lace_parser::{offset_to_line_col, parse_program};
    use serde_json::json;
    let mut diagnostics = Vec::new();
    let (program, parse_errors) = parse_program(source);
    for e in &parse_errors {
        use lace_parser::ParseError;
        let (span_start, span_end) = match e {
            ParseError::Message { span_start, span_end, .. } => (*span_start, *span_end),
        };
        let (sl, sc) = offset_to_line_col(source, span_start);
        let (el, ec) = offset_to_line_col(source, span_end.max(span_start));
        diagnostics.push(json!({
            "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
            "severity": 1,
            "message": format!("{e:?}")
        }));
    }
    if parse_errors.is_empty() {
        let type_errors = if let Some(ref prog) = program {
            lace_types::check_program(prog)
        } else {
            vec![]
        };
        #[allow(unused_variables)]
        let type_errors = type_errors;
        for e in &type_errors {
            let (span_start, span_end) = type_error_span(e);
            let (sl, sc) = offset_to_line_col(source, span_start);
            let (el, ec) = offset_to_line_col(source, span_end.max(span_start));
            diagnostics.push(json!({
                "range": { "start": { "line": sl, "character": sc }, "end": { "line": el, "character": ec } },
                "severity": 1,
                "message": format!("{e:?}")
            }));
        }
    }
    diagnostics
}

fn stdlib_completions() -> Vec<(&'static str, &'static str)> {
    vec![
        ("print",       "fn print(value: Any)"),
        ("println",     "fn println(value: Any)"),
        ("len",         "fn len(list: List<T>) -> Int"),
        ("map",         "fn map(list: List<T>, f: fn(T) -> U) -> List<U>"),
        ("filter",      "fn filter(list: List<T>, f: fn(T) -> Bool) -> List<T>"),
        ("reduce",      "fn reduce(list: List<T>, init: U, f: fn(U, T) -> U) -> U"),
        ("range",       "fn range(start: Int, end: Int) -> List<Int>"),
        ("toString",    "fn toString(value: Any) -> String"),
        ("parseInt",    "fn parseInt(s: String) -> Int"),
        ("parseFloat",  "fn parseFloat(s: String) -> Float"),
        ("concat",      "fn concat(a: String, b: String) -> String"),
        ("split",       "fn split(s: String, sep: String) -> List<String>"),
        ("trim",        "fn trim(s: String) -> String"),
        ("Http.get",    "tool Http.get(url: String) -> Result<String, String>"),
        ("Http.post",   "tool Http.post(url: String, body: String) -> Result<String, String>"),
        ("Fs.read",     "tool Fs.read(path: String) -> Result<String, String>"),
        ("Fs.write",    "tool Fs.write(path: String, content: String) -> Result<(), String>"),
    ]
}
