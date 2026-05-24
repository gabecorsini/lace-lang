//! Lace Language Server Protocol (LSP) implementation.
//!
//! Capabilities:
//! - textDocument/hover           — type + doc comment for symbol under cursor
//! - textDocument/definition      — go-to-definition for functions / variables
//! - textDocument/completion      — stdlib + user-defined symbols
//! - textDocument/publishDiagnostics — parse + type errors/warnings on open/change
//! - textDocument/formatting      — pretty-print via the built-in formatter

use std::collections::HashMap;
use std::sync::Arc;

use lace_ast::{
    BinaryOp, Block, EffectExpr, EffectTag, Expr, FnDecl, Literal, PrimitiveType, Stmt,
    TopLevelItem, TypeExpr, UnaryOp,
};
use lace_parser::{offset_to_line_col, parse_program, ParseError};
use lace_types::{check_program_full, TypeError, TypeWarning};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use tokio::sync::RwLock;

// ─── Document store ──────────────────────────────────────────────────────────

type DocumentStore = Arc<RwLock<HashMap<Url, String>>>;

// ─── Backend ─────────────────────────────────────────────────────────────────

pub struct LaceBackend {
    client: Client,
    documents: DocumentStore,
}

impl LaceBackend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn on_change(&self, uri: Url, text: String) {
        {
            let mut docs = self.documents.write().await;
            docs.insert(uri.clone(), text.clone());
        }
        let diagnostics = compute_diagnostics(&text);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

// ─── Diagnostic computation ──────────────────────────────────────────────────

pub fn compute_diagnostics(source: &str) -> Vec<Diagnostic> {
    let mut diags: Vec<Diagnostic> = Vec::new();

    let (program_opt, parse_errors) = parse_program(source);

    for err in &parse_errors {
        let ParseError::Message {
            message,
            span_start,
            span_end,
        } = err;
        let range = span_to_range(source, *span_start, *span_end);
        diags.push(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: None,
            code_description: None,
            source: Some("lace".to_string()),
            message: message.clone(),
            related_information: None,
            tags: None,
            data: None,
        });
    }

    if let Some(program) = program_opt {
        let (type_errors, type_warnings) = check_program_full(&program);

        for err in &type_errors {
            let (start, end) = type_error_span(err).unwrap_or((0, 0));
            let range = span_to_range(source, start, end);
            diags.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: Some(NumberOrString::String(err.code().to_string())),
                code_description: None,
                source: Some("lace".to_string()),
                message: err.to_string(),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        for warn in &type_warnings {
            match warn {
                TypeWarning::UnusedVariable { name, span_start, span_end } => {
                    let range = span_to_range(source, *span_start, *span_end);
                    diags.push(Diagnostic {
                        range,
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: Some(NumberOrString::String(warn.code().to_string())),
                        code_description: None,
                        source: Some("lace".to_string()),
                        message: format!("unused variable: `{name}`"),
                        related_information: None,
                        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
                        data: None,
                    });
                }
                TypeWarning::PureFnCallsEffectful { fn_name, callee } => {
                    // No span for W004; emit a file-level diagnostic at position 0
                    diags.push(Diagnostic {
                        range: tower_lsp::lsp_types::Range {
                            start: tower_lsp::lsp_types::Position { line: 0, character: 0 },
                            end: tower_lsp::lsp_types::Position { line: 0, character: 0 },
                        },
                        severity: Some(DiagnosticSeverity::WARNING),
                        code: Some(NumberOrString::String("W004".to_string())),
                        code_description: None,
                        source: Some("lace".to_string()),
                        message: format!(
                            "fn '{fn_name}' calls effectful '{callee}' — consider declaring '{fn_name}' as a tool"
                        ),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
        }
    }

    diags
}

fn type_error_span(err: &TypeError) -> Option<(usize, usize)> {
    match err {
        TypeError::UnknownIdentifier {
            span_start,
            span_end,
            ..
        }
        | TypeError::Mismatch {
            span_start,
            span_end,
            ..
        }
        | TypeError::UnknownFunction {
            span_start,
            span_end,
            ..
        }
        | TypeError::InvalidPattern {
            span_start,
            span_end,
            ..
        }
        | TypeError::NonExhaustiveMatch {
            span_start,
            span_end,
            ..
        } => Some((*span_start, *span_end)),
        TypeError::UnknownRecordType { .. } | TypeError::InvalidToolDecl { .. } => None,
    }
}

/// Convert a byte-offset span to an LSP Range (0-based lines and chars).
fn span_to_range(source: &str, start: usize, end: usize) -> Range {
    let (sl, sc) = offset_to_line_col(source, start);
    let (el, ec) = offset_to_line_col(source, end);
    Range {
        start: Position {
            line: (sl.saturating_sub(1)) as u32,
            character: (sc.saturating_sub(1)) as u32,
        },
        end: Position {
            line: (el.saturating_sub(1)) as u32,
            character: (ec.saturating_sub(1)) as u32,
        },
    }
}

/// Convert an LSP Position to a byte offset in source.
#[allow(dead_code)]
fn position_to_offset(source: &str, pos: Position) -> usize {
    let target_line = pos.line as usize;
    let target_char = pos.character as usize;
    let mut current_line = 0usize;
    let mut offset = 0usize;
    for (i, ch) in source.char_indices() {
        if current_line == target_line {
            if source[i..].chars().count() == 0 || (i - line_start_offset(source, current_line)) >= target_char {
                return i;
            }
        }
        if ch == '\n' {
            current_line += 1;
            if current_line > target_line {
                return offset;
            }
        }
        offset = i + ch.len_utf8();
    }
    offset
}

#[allow(dead_code)]
fn line_start_offset(source: &str, target_line: usize) -> usize {
    let mut line = 0;
    let mut offset = 0;
    for (i, ch) in source.char_indices() {
        if line == target_line {
            return i;
        }
        if ch == '\n' {
            line += 1;
            offset = i + 1;
        }
    }
    offset
}

/// Return the word (identifier) at the given byte offset.
fn word_at_offset(source: &str, offset: usize) -> &str {
    let bytes = source.as_bytes();
    let len = bytes.len();
    if offset >= len {
        return "";
    }
    // Walk backward to find start
    let mut start = offset;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    // Walk forward to find end
    let mut end = offset;
    while end < len && is_ident_byte(bytes[end]) {
        end += 1;
    }
    &source[start..end]
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ─── Symbol collection ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SymbolInfo {
    name: String,
    kind: SymbolKind,
    /// Byte offset of the definition
    #[allow(dead_code)]
    def_offset: usize,
    /// Hover text (signature + doc)
    hover_text: String,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum SymbolKind {
    Function,
    Variable,
    Record,
    Enum,
}

fn collect_symbols(source: &str) -> Vec<SymbolInfo> {
    let (program_opt, _) = parse_program(source);
    let program = match program_opt {
        Some(p) => p,
        None => return vec![],
    };

    let mut symbols = Vec::new();

    for item in &program.items {
        match item {
            TopLevelItem::Function(f) => {
                let sig = fmt_fn_signature(f);
                let doc = f
                    .doc_comment
                    .as_deref()
                    .map(|d| format!("\n\n{d}"))
                    .unwrap_or_default();
                let hover_text = format!("```lace\n{sig}\n```{doc}");
                symbols.push(SymbolInfo {
                    name: f.name.clone(),
                    kind: SymbolKind::Function,
                    def_offset: f.body.span.start.saturating_sub(1),
                    hover_text,
                });
            }
            TopLevelItem::Record(r) => {
                let sig = format!("record {}", r.name);
                let doc = r
                    .doc_comment
                    .as_deref()
                    .map(|d| format!("\n\n{d}"))
                    .unwrap_or_default();
                symbols.push(SymbolInfo {
                    name: r.name.clone(),
                    kind: SymbolKind::Record,
                    def_offset: 0,
                    hover_text: format!("```lace\n{sig}\n```{doc}"),
                });
            }
            TopLevelItem::Enum(e) => {
                let sig = format!("enum {}", e.name);
                symbols.push(SymbolInfo {
                    name: e.name.clone(),
                    kind: SymbolKind::Enum,
                    def_offset: 0,
                    hover_text: format!("```lace\n{sig}\n```"),
                });
            }
            _ => {}
        }
    }

    symbols
}

// ─── Completion items ─────────────────────────────────────────────────────────

/// Built-in stdlib completion labels.
static STDLIB_COMPLETIONS: &[(&str, &str)] = &[
    ("print", "fn print(value: Dynamic) [IO]"),
    ("println", "fn println(value: Dynamic) [IO]"),
    ("type_of", "fn type_of(value: Dynamic) -> String"),
    ("to_string", "fn to_string(value: Dynamic) -> String"),
    ("assert", "fn assert(cond: Bool)"),
    ("assert_eq", "fn assert_eq(a: Dynamic, b: Dynamic)"),
    ("List.length", "fn List.length(list: List<T>) -> Int"),
    ("List.range", "fn List.range(start: Int, end: Int) -> List<Int>"),
    ("List.map", "fn List.map(list: List<T>, f: Fn(T) -> U) -> List<U>"),
    ("List.filter", "fn List.filter(list: List<T>, f: Fn(T) -> Bool) -> List<T>"),
    ("List.fold", "fn List.fold(list: List<T>, init: U, f: Fn(U, T) -> U) -> U"),
    ("List.contains", "fn List.contains(list: List<T>, value: T) -> Bool"),
    ("List.sort", "fn List.sort(list: List<T>) -> List<T>"),
    ("List.sum", "fn List.sum(list: List<Int>) -> Int"),
    ("List.min", "fn List.min(list: List<Int>) -> Int"),
    ("List.max", "fn List.max(list: List<Int>) -> Int"),
    ("List.zip", "fn List.zip(a: List<T>, b: List<U>) -> List<(T, U)>"),
    ("List.flat_map", "fn List.flat_map(list: List<T>, f: Fn(T) -> List<U>) -> List<U>"),
    ("Map.new", "fn Map.new() -> Map<String, Dynamic>"),
    ("Map.insert", "fn Map.insert(map: Map<K, V>, key: K, value: V) -> Map<K, V>"),
    ("Map.get", "fn Map.get(map: Map<K, V>, key: K) -> Option<V>"),
    ("Map.remove", "fn Map.remove(map: Map<K, V>, key: K) -> Map<K, V>"),
    ("Map.contains_key", "fn Map.contains_key(map: Map<K, V>, key: K) -> Bool"),
    ("Map.keys", "fn Map.keys(map: Map<K, V>) -> List<K>"),
    ("Map.values", "fn Map.values(map: Map<K, V>) -> List<V>"),
    ("Map.entries", "fn Map.entries(map: Map<K, V>) -> List<(K, V)>"),
    ("Map.length", "fn Map.length(map: Map<K, V>) -> Int"),
    ("String.length", "fn String.length(s: String) -> Int"),
    ("String.contains", "fn String.contains(s: String, sub: String) -> Bool"),
    ("String.starts_with", "fn String.starts_with(s: String, prefix: String) -> Bool"),
    ("String.ends_with", "fn String.ends_with(s: String, suffix: String) -> Bool"),
    ("String.to_upper", "fn String.to_upper(s: String) -> String"),
    ("String.to_lower", "fn String.to_lower(s: String) -> String"),
    ("String.trim", "fn String.trim(s: String) -> String"),
    ("String.split", "fn String.split(s: String, sep: String) -> List<String>"),
    ("String.replace", "fn String.replace(s: String, from: String, to: String) -> String"),
    ("Http.get", "fn Http.get(url: String) -> Result<String, String> [IO]"),
    ("Http.post", "fn Http.post(url: String, body: String) -> Result<String, String> [IO]"),
    ("Http.post_json", "fn Http.post_json(url: String, body: Dynamic) -> Result<String, String> [IO]"),
    ("Json.parse", "fn Json.parse(s: String) -> Result<Dynamic, String>"),
    ("Json.stringify", "fn Json.stringify(value: Dynamic) -> String"),
    ("Json.get", "fn Json.get(value: Dynamic, key: String) -> Option<Dynamic>"),
    ("Json.keys", "fn Json.keys(value: Dynamic) -> List<String>"),
    ("Env.get", "fn Env.get(key: String) -> Option<String> [IO]"),
    ("Env.set", "fn Env.set(key: String, value: String) [IO]"),
    ("File.read", "fn File.read(path: String) -> Result<String, String> [IO]"),
    ("File.write", "fn File.write(path: String, content: String) -> Result<Unit, String> [IO]"),
    ("File.exists", "fn File.exists(path: String) -> Bool [IO]"),
];

fn stdlib_completion_items() -> Vec<CompletionItem> {
    STDLIB_COMPLETIONS
        .iter()
        .map(|(label, detail)| CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::FUNCTION),
            detail: Some(detail.to_string()),
            documentation: None,
            insert_text: Some(label.to_string()),
            ..Default::default()
        })
        .collect()
}

// ─── Formatter (re-implemented here; mirrors lace-cli's fmt_* functions) ─────

pub fn fmt_program(program: &lace_ast::Program) -> String {
    let mut out = String::new();
    let items = &program.items;
    for (i, item) in items.iter().enumerate() {
        out.push_str(&fmt_top_level_item(item));
        if i + 1 < items.len() {
            out.push('\n');
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn fmt_top_level_item(item: &lace_ast::TopLevelItem) -> String {
    match item {
        TopLevelItem::Function(f) => fmt_fn_decl(f),
        TopLevelItem::Const(c) => {
            format!("const {}: {} = {}\n", c.name, fmt_type_expr(&c.ty), fmt_expr(&c.expr))
        }
        _ => String::new(),
    }
}

fn fmt_fn_decl(f: &FnDecl) -> String {
    let params = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, fmt_type_expr(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = f
        .ret_ty
        .as_ref()
        .map(|t| format!(" -> {}", fmt_type_expr(t)))
        .unwrap_or_default();
    let effects = if f.effects.is_empty() {
        String::new()
    } else {
        let tags = f
            .effects
            .iter()
            .map(fmt_effect_expr)
            .collect::<Vec<_>>()
            .join(", ");
        format!(" [{tags}]")
    };
    let body = fmt_block(&f.body, 0);
    format!("fn {}({}){}{} {}\n", f.name, params, ret, effects, body)
}

pub fn fmt_fn_signature(f: &FnDecl) -> String {
    let params = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, fmt_type_expr(&p.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = f
        .ret_ty
        .as_ref()
        .map(|t| format!(" -> {}", fmt_type_expr(t)))
        .unwrap_or_default();
    let effects = if f.effects.is_empty() {
        String::new()
    } else {
        let tags = f
            .effects
            .iter()
            .map(fmt_effect_expr)
            .collect::<Vec<_>>()
            .join(", ");
        format!(" [{tags}]")
    };
    format!("fn {}({}){}{}", f.name, params, ret, effects)
}

fn fmt_block(block: &Block, indent: usize) -> String {
    let pad = "  ".repeat(indent + 1);
    let close_pad = "  ".repeat(indent);
    let mut lines = vec!["{".to_string()];
    for stmt in &block.stmts {
        lines.push(format!("{}{}", pad, fmt_stmt(stmt, indent + 1)));
    }
    if let Some(tail) = &block.tail_expr {
        lines.push(format!("{}{}", pad, fmt_expr(tail)));
    }
    lines.push(format!("{}}}", close_pad));
    lines.join("\n")
}

fn fmt_stmt(stmt: &Stmt, indent: usize) -> String {
    match stmt {
        Stmt::Let(s) => {
            if let Some(ty) = &s.ty {
                format!("let {}: {} = {}", s.name, fmt_type_expr(ty), fmt_expr(&s.expr))
            } else {
                format!("let {} = {}", s.name, fmt_expr(&s.expr))
            }
        }
        Stmt::MutLet(s) => {
            if let Some(ty) = &s.ty {
                format!("mut {}: {} = {}", s.name, fmt_type_expr(ty), fmt_expr(&s.expr))
            } else {
                format!("mut {} = {}", s.name, fmt_expr(&s.expr))
            }
        }
        Stmt::Assign(a) => format!("{} = {}", a.name, fmt_expr(&a.expr)),
        Stmt::Expr(e) => fmt_expr(e),
        Stmt::For(f) => {
            let body = fmt_block(&f.body, indent);
            format!("for {} in {} {}", f.name, fmt_expr(&f.iter), body)
        }
        Stmt::While(w) => {
            let body = fmt_block(&w.body, indent);
            format!("while {} {}", fmt_expr(&w.cond), body)
        }
        Stmt::PureBlock(b) => fmt_block(b, indent),
    }
}

fn fmt_expr(expr: &Expr) -> String {
    match expr {
        Expr::Literal(l, _) => match l {
            Literal::Int(n) => n.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::String(s) => format!("\"{s}\""),
            Literal::Bool(b) => b.to_string(),
        },
        Expr::Ident(name, _) => name.clone(),
        Expr::Binary {
            left, op, right, ..
        } => {
            let op_str = match op {
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::IntDiv => "//",
                BinaryOp::Rem => "%",
                BinaryOp::Eq => "==",
                BinaryOp::Ne => "!=",
                BinaryOp::Lt => "<",
                BinaryOp::Gt => ">",
                BinaryOp::Le => "<=",
                BinaryOp::Ge => ">=",
                BinaryOp::And => "and",
                BinaryOp::Or => "or",
                BinaryOp::Concat => "++",
            };
            format!("{} {} {}", fmt_expr(left), op_str, fmt_expr(right))
        }
        Expr::Unary { op, expr, .. } => {
            let op_str = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "not ",
            };
            format!("{}{}", op_str, fmt_expr(expr))
        }
        Expr::FnCall(call) => {
            let args = call.args.iter().map(fmt_expr).collect::<Vec<_>>().join(", ");
            format!("{}({})", call.name, args)
        }
        Expr::Block(b) => fmt_block(b, 0),
        Expr::If(i) => {
            let mut parts = Vec::new();
            for (j, (cond, blk)) in i.branches.iter().enumerate() {
                let kw = if j == 0 { "if" } else { "else if" };
                parts.push(format!("{} {} {}", kw, fmt_expr(cond), fmt_block(blk, 0)));
            }
            if let Some(else_blk) = &i.else_block {
                parts.push(format!("else {}", fmt_block(else_blk, 0)));
            }
            parts.join(" ")
        }
        Expr::Return { value, .. } => match value {
            Some(v) => format!("return {}", fmt_expr(v)),
            None => "return".to_string(),
        },
        Expr::ListLiteral { elems, .. } => {
            let items = elems.iter().map(fmt_expr).collect::<Vec<_>>().join(", ");
            format!("[{items}]")
        }
        Expr::TupleLiteral { elems, .. } => {
            let items = elems.iter().map(fmt_expr).collect::<Vec<_>>().join(", ");
            format!("({items})")
        }
        _ => "/* expr */".to_string(),
    }
}

fn fmt_type_expr(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Primitive(p, _) => match p {
            PrimitiveType::Int => "Int".to_string(),
            PrimitiveType::Float => "Float".to_string(),
            PrimitiveType::Bool => "Bool".to_string(),
            PrimitiveType::String => "String".to_string(),
            PrimitiveType::Bytes => "Bytes".to_string(),
            PrimitiveType::Unit => "Unit".to_string(),
        },
        TypeExpr::Dynamic(_) => "Dynamic".to_string(),
        TypeExpr::Named { name, .. } => name.clone(),
        TypeExpr::Generic { name, args, .. } => {
            let a = args.iter().map(fmt_type_expr).collect::<Vec<_>>().join(", ");
            format!("{name}<{a}>")
        }
        TypeExpr::Tuple { elems, .. } => {
            let e = elems.iter().map(fmt_type_expr).collect::<Vec<_>>().join(", ");
            format!("({e})")
        }
        TypeExpr::Function { params, ret, .. } => {
            let p = params.iter().map(fmt_type_expr).collect::<Vec<_>>().join(", ");
            format!("Fn({p}) -> {}", fmt_type_expr(ret))
        }
    }
}

fn fmt_effect_expr(e: &EffectExpr) -> String {
    match e {
        EffectExpr::Builtin(tag) => match tag {
            EffectTag::Pure => "Pure".to_string(),
            EffectTag::Io => "IO".to_string(),
            EffectTag::Mut => "Mut".to_string(),
            EffectTag::ToolCall => "ToolCall".to_string(),
            EffectTag::Time => "Time".to_string(),
            EffectTag::Rand => "Rand".to_string(),
        },
        EffectExpr::Variable(name) => name.clone(),
    }
}

// ─── LanguageServer impl ──────────────────────────────────────────────────────

#[tower_lsp::async_trait]
impl LanguageServer for LaceBackend {
    async fn initialize(&self, _params: InitializeParams) -> LspResult<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                document_formatting_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "lace-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "lace-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.on_change(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.on_change(params.text_document.uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let mut docs = self.documents.write().await;
        docs.remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let offset = pos_to_offset_simple(&source, pos);
        let word = word_at_offset(&source, offset);
        if word.is_empty() {
            return Ok(None);
        }

        // Check user symbols first
        let symbols = collect_symbols(&source);
        for sym in &symbols {
            if sym.name == word {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: sym.hover_text.clone(),
                    }),
                    range: None,
                }));
            }
        }

        // Check stdlib
        for (label, detail) in STDLIB_COMPLETIONS {
            if *label == word {
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```lace\n{detail}\n```"),
                    }),
                    range: None,
                }));
            }
        }

        Ok(None)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let offset = pos_to_offset_simple(&source, pos);
        let word = word_at_offset(&source, offset);
        if word.is_empty() {
            return Ok(None);
        }

        let (program_opt, _) = parse_program(&source);
        let program = match program_opt {
            Some(p) => p,
            None => return Ok(None),
        };

        // Search top-level functions and variables
        for item in &program.items {
            match item {
                TopLevelItem::Function(f) if f.name == word => {
                    let (line, col) = offset_to_line_col(&source, f.body.span.start);
                    let def_pos = Position {
                        line: (line.saturating_sub(1)) as u32,
                        character: (col.saturating_sub(1)) as u32,
                    };
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: uri.clone(),
                        range: Range {
                            start: def_pos,
                            end: def_pos,
                        },
                    })));
                }
                TopLevelItem::Const(c) if c.name == word => {
                    // Consts don't have a direct span, use offset 0
                    let def_pos = Position { line: 0, character: 0 };
                    return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                        uri: uri.clone(),
                        range: Range {
                            start: def_pos,
                            end: def_pos,
                        },
                    })));
                }
                _ => {}
            }
        }

        Ok(None)
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;

        let docs = self.documents.read().await;
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let offset = pos_to_offset_simple(&source, pos);

        // Check if we're in a method-call context (preceded by '.')
        let prefix_before = if offset > 0 { &source[..offset] } else { "" };
        let is_method = prefix_before.trim_end().ends_with('.');

        let mut items: Vec<CompletionItem> = stdlib_completion_items();

        // Keyword completions
        let keywords: &[(&str, &str)] = &[
            ("fn", "fn keyword — declare a function"),
            ("tool", "tool keyword — declare a tool (effectful external call)"),
            ("let", "let keyword — bind a value"),
            ("mut", "mut keyword — mutable binding"),
            ("if", "if keyword — conditional expression"),
            ("else", "else keyword — else branch"),
            ("match", "match keyword — pattern matching"),
            ("for", "for keyword — for loop"),
            ("while", "while keyword — while loop"),
            ("return", "return keyword — early return"),
            ("record", "record keyword — declare a record type"),
            ("enum", "enum keyword — declare an enum type"),
            ("import", "import keyword — import a module"),
            ("const", "const keyword — declare a constant"),
        ];
        for (kw, detail) in keywords {
            items.push(CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some(detail.to_string()),
                insert_text: Some(kw.to_string()),
                ..Default::default()
            });
        }

        // Add user-defined symbols
        let symbols = collect_symbols(&source);
        for sym in &symbols {
            items.push(CompletionItem {
                label: sym.name.clone(),
                kind: Some(match sym.kind {
                    SymbolKind::Function => CompletionItemKind::FUNCTION,
                    SymbolKind::Variable => CompletionItemKind::VARIABLE,
                    SymbolKind::Record => CompletionItemKind::STRUCT,
                    SymbolKind::Enum => CompletionItemKind::ENUM,
                }),
                detail: Some(sym.hover_text.lines().next().unwrap_or("").to_string()),
                ..Default::default()
            });
        }

        // If method context, filter to methods only
        let final_items = if is_method {
            items
                .into_iter()
                .filter(|i| i.label.contains('.'))
                .collect()
        } else {
            items
        };

        Ok(Some(CompletionResponse::Array(final_items)))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> LspResult<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;

        let docs = self.documents.read().await;
        let source = match docs.get(uri) {
            Some(s) => s.clone(),
            None => return Ok(None),
        };
        drop(docs);

        let (program_opt, parse_errors) = parse_program(&source);
        if !parse_errors.is_empty() {
            // Cannot format if there are parse errors
            return Ok(None);
        }
        let program = match program_opt {
            Some(p) => p,
            None => return Ok(None),
        };

        let formatted = fmt_program(&program);

        // Replace the entire document
        let line_count = source.lines().count();
        let last_line_len = source.lines().last().map(|l| l.len()).unwrap_or(0);

        let edit = TextEdit {
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position {
                    line: line_count as u32,
                    character: last_line_len as u32,
                },
            },
            new_text: formatted,
        };

        Ok(Some(vec![edit]))
    }
}

/// Simple position → byte offset (handles ASCII and basic Unicode).
fn pos_to_offset_simple(source: &str, pos: Position) -> usize {
    let mut line = 0u32;
    let mut col = 0u32;
    for (i, ch) in source.char_indices() {
        if line == pos.line && col == pos.character {
            return i;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    source.len()
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Run the LSP server over stdio. Call from `lace lsp` or the binary.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(LaceBackend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
