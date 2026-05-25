use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use regex::Regex as StdRegex;

pub mod tool_log;
pub use tool_log::ToolLogger;

use lace_ast::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Record {
        name: String,
        fields: HashMap<String, Value>,
    },
    Variant {
        name: String,
        payload: Vec<Value>,
    },
    Map(HashMap<String, Value>),
    Closure {
        params: Vec<String>,
        body: Block,
        captured_env: HashMap<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuntimeError {
    pub message: String,
    pub span: Option<Span>,
    /// When Some, this error was produced by the `?` operator propagating an Err value.
    /// call_function catches this and returns Ok(Err(v)) instead of propagating the error.
    pub propagated_err: Option<Value>,
    /// When true, this error was produced by the `?` operator propagating a None value.
    /// call_function catches this and returns Ok(None) instead of propagating the error.
    pub propagated_none: bool,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub checkpoint_path: Option<String>,
    pub replay_mode: bool,
    pub source_path: Option<String>,
    pub suppress_tool_log: bool,
    pub log_file: Option<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            checkpoint_path: None,
            replay_mode: false,
            source_path: None,
            suppress_tool_log: false,
            log_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointState {
    run_id: String,
    seq: u64,
    module_name: String,
    journal_path: String,
    checkpoint_path: String,
    env: JsonValue,
}

#[derive(Debug, Clone)]
struct ReplayCursor {
    entries: Vec<JournalEntry>,
    pos: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub run_id: String,
    pub seq: u64,
    pub timestamp: i64,
    pub effect: String,
    pub fn_name: String,
    pub module: String,
    pub inputs: JsonValue,
    pub output: JsonValue,
    pub duration_ms: i64,
}

#[derive(Debug, Clone)]
struct FunctionDef {
    params: Vec<String>,
    effects: Vec<EffectExpr>,
    body: Block,
    qualified_name: String,
    annotations: Vec<lace_ast::Annotation>,
}

#[derive(Debug, Clone)]
struct ToolDef {
    decl: ToolDecl,
}

#[derive(Debug, Clone)]
struct CallFrame {
    effects: Vec<EffectExpr>,
}

#[derive(Debug)]
struct Env {
    scopes: Vec<HashMap<String, Value>>,
}

#[derive(Debug)]
enum EvalFlow {
    Value(Value),
    Return(Value),
}

impl Env {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop(&mut self) {
        let _ = self.scopes.pop();
    }

    fn define(&mut self, name: String, value: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, value);
        }
    }

    fn assign(&mut self, name: &str, value: Value) -> bool {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                scope.insert(name.to_string(), value);
                return true;
            }
        }
        false
    }

    fn get(&self, name: &str) -> Option<Value> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
enum LoopSignal {
    Break,
    Continue,
}

pub struct Interpreter {
    run_id: String,
    seq: u64,
    module_name: String,
    current_dir: Option<PathBuf>,
    journal_path: String,
    checkpoint_path: Option<String>,
    replay: Option<ReplayCursor>,
    env: Env,
    functions: HashMap<String, FunctionDef>,
    tools: HashMap<String, ToolDef>,
    module_members: HashMap<String, HashSet<String>>,
    loaded_modules: HashSet<String>,
    /// Stack of canonical file paths currently being loaded — used for circular import detection.
    loading_stack: Vec<PathBuf>,
    call_stack: Vec<CallFrame>,
    loop_signal: Option<LoopSignal>,
    return_value: Option<Value>,
    variant_constructors: HashSet<String>,
    tool_logger: ToolLogger,
}

impl Interpreter {
    pub fn new(module_name: Option<String>) -> Self {
        Self::new_with_options(module_name, RunOptions::default())
    }

    pub fn new_with_options(module_name: Option<String>, options: RunOptions) -> Self {
        let run_id = format!(
            "run-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::from_secs(0))
                .as_millis()
        );

        let replay = if options.replay_mode {
            if let Some(path) = &options.checkpoint_path {
                Some(load_replay_cursor(path).unwrap_or(ReplayCursor {
                    entries: Vec::new(),
                    pos: 0,
                }))
            } else {
                Some(ReplayCursor {
                    entries: Vec::new(),
                    pos: 0,
                })
            }
        } else {
            None
        };

        Self {
            run_id,
            seq: 0,
            module_name: module_name.unwrap_or_else(|| "main".into()),
            current_dir: options
                .source_path
                .as_ref()
                .map(PathBuf::from)
                .and_then(|p| p.parent().map(Path::to_path_buf)),
            journal_path: options
                .checkpoint_path
                .as_ref()
                .map(|p| format!("{p}.journal.ndjson"))
                .unwrap_or_else(|| ".lace-journal.ndjson".into()),
            checkpoint_path: options.checkpoint_path,
            replay,
            env: Env::new(),
            functions: HashMap::new(),
            tools: HashMap::new(),
            module_members: HashMap::new(),
            loaded_modules: HashSet::new(),
            loading_stack: Vec::new(),
            call_stack: Vec::new(),
            loop_signal: None,
            return_value: None,
            variant_constructors: HashSet::new(),
            tool_logger: ToolLogger::new(options.suppress_tool_log, options.log_file.as_deref()),
        }
    }

    pub fn run_program(mut self, program: &Program) -> Result<Value, RuntimeError> {
        // Register module name bindings so Lace code can do List.range(...), etc.
        self.env.define("List".into(), Value::String("List".into()));
        self.env.define("File".into(), Value::String("File".into()));
        self.env.define("Map".into(), Value::String("Map".into()));
        self.env.define("Http".into(), Value::String("Http".into()));
        self.env.define("Json".into(), Value::String("Json".into()));
        self.env.define("Env".into(), Value::String("Env".into()));
        self.env.define("Fs".into(), Value::String("Fs".into()));
        self.env.define("Time".into(), Value::String("Time".into()));
        self.env.define("Str".into(), Value::String("Str".into()));
        self.env.define("Regex".into(), Value::String("Regex".into()));

        self.load_imports(program)?;
        self.register_items(program);

        // execute top-level consts as bindings
        for item in &program.items {
            if let TopLevelItem::Const(c) = item {
                let value = self.eval_expr(&c.expr)?;
                self.env.define(c.name.clone(), value);
            }
        }

        // run main if present; otherwise Unit
        // run top-level statements
        for item in &program.items {
            if let TopLevelItem::Statement(stmt) = item {
                self.eval_stmt(stmt)?;
            }
        }

        let out = if self.functions.contains_key("main") {
            self.call_function("main", vec![], Span::default())
        } else {
            Ok(Value::Unit)
        }?;

        if self.checkpoint_path.is_some() {
            self.save_checkpoint_state()?;
        }

        if let Some(summary) = self.tool_logger.summary() {
            eprintln!("{summary}");
        }

        Ok(out)
    }

    pub fn run_named_function(
        mut self,
        program: &Program,
        function_name: &str,
    ) -> Result<Value, RuntimeError> {
        // Register module name bindings so Lace code can do List.range(...), etc.
        self.env.define("List".into(), Value::String("List".into()));
        self.env.define("File".into(), Value::String("File".into()));
        self.env.define("Map".into(), Value::String("Map".into()));
        self.env.define("Http".into(), Value::String("Http".into()));
        self.env.define("Json".into(), Value::String("Json".into()));
        self.env.define("Env".into(), Value::String("Env".into()));
        self.env.define("Fs".into(), Value::String("Fs".into()));
        self.env.define("Time".into(), Value::String("Time".into()));
        self.env.define("Str".into(), Value::String("Str".into()));
        self.env.define("Regex".into(), Value::String("Regex".into()));

        self.load_imports(program)?;
        self.register_items(program);

        // execute top-level consts as bindings
        for item in &program.items {
            if let TopLevelItem::Const(c) = item {
                let value = self.eval_expr(&c.expr)?;
                self.env.define(c.name.clone(), value);
            }
        }

        let out = self.call_function(function_name, vec![], Span::default())?;

        if self.checkpoint_path.is_some() {
            self.save_checkpoint_state()?;
        }

        if let Some(summary) = self.tool_logger.summary() {
            eprintln!("{summary}");
        }

        Ok(out)
    }

    fn register_items(&mut self, program: &Program) {
        if let Some(module) = &program.module {
            self.module_name = module.path.join(".");
        }

        let mut exported = self
            .module_members
            .remove(&self.module_name)
            .unwrap_or_default();

        for item in &program.items {
            match item {
                TopLevelItem::Function(f) => {
                    let qualified_name = format!("{}.{}", self.module_name, f.name);
                    let def = FunctionDef {
                        params: f.params.iter().map(|p| p.name.clone()).collect(),
                        effects: f.effects.clone(),
                        body: f.body.clone(),
                        qualified_name: qualified_name.clone(),
                        annotations: f.annotations.clone(),
                    };
                    self.functions.insert(qualified_name.clone(), def.clone());
                    if self.module_name == "main" {
                        self.functions.insert(f.name.clone(), def);
                    }
                    exported.insert(f.name.clone());
                }
                TopLevelItem::Tool(t) => {
                    self.tools
                        .insert(t.name.clone(), ToolDef { decl: t.clone() });
                    exported.insert(t.name.clone());
                }
                TopLevelItem::Enum(e) => {
                    for variant in &e.variants {
                        match &variant.body {
                            None => {
                                // Unit variant: register as a Value in global env
                                self.env.define(
                                    variant.name.clone(),
                                    Value::Variant {
                                        name: variant.name.clone(),
                                        payload: vec![],
                                    },
                                );
                            }
                            Some(lace_ast::EnumVariantBody::Tuple(_)) => {
                                // Tuple variant: register as a callable constructor
                                self.variant_constructors.insert(variant.name.clone());
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        self.module_members
            .insert(self.module_name.clone(), exported);
    }

    fn load_imports(&mut self, program: &Program) -> Result<(), RuntimeError> {
        let Some(base_dir) = self.current_dir.clone() else {
            return Ok(());
        };

        for import in &program.imports {
            self.load_module_from_import(&base_dir, import)?;
        }

        Ok(())
    }

    fn load_module_from_import(
        &mut self,
        base_dir: &Path,
        import: &ImportDecl,
    ) -> Result<(), RuntimeError> {
        // Resolve the file path relative to the importing file's directory.
        let module_path = base_dir.join(&import.file_path);
        let canonical = module_path.canonicalize().map_err(|e| RuntimeError {
            message: format!(
                "import '{}': file not found at '{}': {e}",
                import.file_path,
                module_path.display()
            ),
            span: Some(import.span),
            propagated_err: None,
                propagated_none: false,
        })?;

        // Dedup: skip if already loaded.
        let canon_str = canonical.to_string_lossy().to_string();
        if self.loaded_modules.contains(&canon_str) {
            // Module already loaded — just bind the alias as itself for dispatch.
            self.env
                .define(import.alias.clone(), Value::String(import.alias.clone()));
            return Ok(());
        }

        // Circular import detection.
        if self.loading_stack.contains(&canonical) {
            let cycle: Vec<String> = self
                .loading_stack
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            return Err(RuntimeError {
                message: format!(
                    "circular import detected: {} -> {}",
                    cycle.join(" -> "),
                    canonical.display()
                ),
                span: Some(import.span),
                propagated_err: None,
                propagated_none: false,
            });
        }

        let source = fs::read_to_string(&canonical).map_err(|e| RuntimeError {
            message: format!(
                "import '{}': cannot read file '{}': {e}",
                import.file_path,
                canonical.display()
            ),
            span: Some(import.span),
            propagated_err: None,
                propagated_none: false,
        })?;

        let (parsed, parse_errors) = lace_parser::parse_program(&source);
        if !parse_errors.is_empty() {
            let joined = parse_errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(RuntimeError {
                message: format!(
                    "parse error in imported file '{}': {}",
                    canonical.display(),
                    joined
                ),
                span: Some(import.span),
                propagated_err: None,
                propagated_none: false,
            });
        }

        let program = parsed.ok_or_else(|| RuntimeError {
            message: format!(
                "failed to parse imported file '{}'",
                canonical.display()
            ),
            span: Some(import.span),
            propagated_err: None,
                propagated_none: false,
        })?;

        self.loaded_modules.insert(canon_str.clone());
        self.loading_stack.push(canonical.clone());

        // Save interpreter context and switch into the imported module's scope.
        let prev_module = self.module_name.clone();
        let prev_dir = self.current_dir.clone();
        self.module_name = canon_str.clone();
        self.current_dir = canonical.parent().map(Path::to_path_buf);

        // Recursively load the imported module's own imports.
        self.load_imports(&program)?;

        // Register items under the module's own names (for recursive calls inside the module),
        // then register again prefixed with the alias for callers.
        self.register_items(&program);

        // Also register every public fn/tool/enum/record under "<alias>.<name>"
        // so callers can do alias.fn_name(...).
        let alias = import.alias.clone();
        for item in &program.items {
            match item {
                TopLevelItem::Function(f) => {
                    let qualified = format!("{}.{}", alias, f.name);
                    // Functions are stored under "<canon_path>.<name>" by register_items
                    let canon_key = format!("{}.{}", canon_str, f.name);
                    let def = self.functions.get(&canon_key)
                        .or_else(|| self.functions.get(&f.name))
                        .cloned();
                    if let Some(def) = def {
                        self.functions.insert(qualified, def);
                    }
                }
                TopLevelItem::Tool(t) => {
                    let qualified = format!("{}.{}", alias, t.name);
                    let def = self.tools.get(&t.name).cloned();
                    if let Some(def) = def {
                        self.tools.insert(qualified, def);
                    }
                }
                TopLevelItem::Enum(e) => {
                    // Register variant constructors under alias.VariantName as well
                    for variant in &e.variants {
                        let qualified_variant = format!("{}.{}", alias, variant.name);
                        self.variant_constructors.insert(qualified_variant.clone());
                        if let Some(v) = self.env.get(&variant.name) {
                            self.env.define(qualified_variant, v);
                        }
                    }
                    // Also expose the enum type name under alias.EnumName
                    let qualified_enum = format!("{}.{}", alias, e.name);
                    self.env
                        .define(qualified_enum, Value::String(format!("{}.{}", alias, e.name)));
                }
                TopLevelItem::Record(r) => {
                    // Expose record constructor alias.RecordName
                    let qualified = format!("{}.{}", alias, r.name);
                    if let Some(v) = self.env.get(&r.name) {
                        self.env.define(qualified, v);
                    }
                }
                TopLevelItem::Const(c) => {
                    let qualified = format!("{}.{}", alias, c.name);
                    // First try env (in case already evaluated), then eval inline
                    let v = if let Some(v) = self.env.get(&c.name) {
                        Some(v)
                    } else {
                        self.eval_expr(&c.expr).ok()
                    };
                    if let Some(v) = v {
                        self.env.define(qualified, v);
                    }
                }
                TopLevelItem::TypeAlias(_)
                | TopLevelItem::Extern(_)
                | TopLevelItem::Statement(_) => {}
            }
        }

        self.loading_stack.pop();
        self.current_dir = prev_dir;
        self.module_name = prev_module;

        // Bind the alias as a string so method dispatch constructs "alias.method_name"
        // and finds the qualified function registered above.
        self.env
            .define(import.alias.clone(), Value::String(import.alias.clone()));

        Ok(())
    }

    fn eval_block(&mut self, block: &Block) -> Result<EvalFlow, RuntimeError> {
        self.env.push();

        for stmt in &block.stmts {
            match self.eval_stmt(stmt)? {
                EvalFlow::Value(_) => {}
                EvalFlow::Return(v) => {
                    self.env.pop();
                    return Ok(EvalFlow::Return(v));
                }
            }
            // Short-circuit on return/break/continue signals from sub-expressions
            if self.return_value.is_some() || self.loop_signal.is_some() {
                self.env.pop();
                return Ok(EvalFlow::Value(Value::Unit));
            }
        }

        let out = if let Some(tail) = &block.tail_expr {
            match self.eval_expr_flow(tail)? {
                EvalFlow::Value(v) => EvalFlow::Value(v),
                EvalFlow::Return(v) => EvalFlow::Return(v),
            }
        } else {
            EvalFlow::Value(Value::Unit)
        };

        self.env.pop();
        Ok(out)
    }

    fn eval_stmt(&mut self, stmt: &Stmt) -> Result<EvalFlow, RuntimeError> {
        match stmt {
            Stmt::Let(s) | Stmt::MutLet(s) => {
                let value = self.eval_expr(&s.expr)?;
                self.env.define(s.name.clone(), value);
                Ok(EvalFlow::Value(Value::Unit))
            }
            Stmt::Assign(s) => {
                let value = self.eval_expr(&s.expr)?;
                if !self.env.assign(&s.name, value) {
                    return Err(RuntimeError {
                        message: format!("unknown variable '{}'", s.name),
                        span: Some(s.span),
                        propagated_err: None,
                propagated_none: false,
                    });
                }
                Ok(EvalFlow::Value(Value::Unit))
            }
            Stmt::Expr(e) => self.eval_expr_flow(e),
            Stmt::For(f) => {
                let iter = self.eval_expr(&f.iter)?;
                if let Value::List(items) = iter {
                    'for_loop: for item in items {
                        self.env.push();
                        self.env.define(f.name.clone(), item);
                        match self.eval_block(&f.body)? {
                            EvalFlow::Value(_) => {}
                            EvalFlow::Return(v) => {
                                self.env.pop();
                                return Ok(EvalFlow::Return(v));
                            }
                        }
                        self.env.pop();
                        if self.return_value.is_some() {
                            break 'for_loop;
                        }
                        match self.loop_signal.take() {
                            Some(LoopSignal::Break) => break 'for_loop,
                            Some(LoopSignal::Continue) => continue 'for_loop,
                            None => {}
                        }
                    }
                    Ok(EvalFlow::Value(Value::Unit))
                } else {
                    Err(RuntimeError {
                        message: "for-loop requires a list iterator".into(),
                        span: Some(f.span),
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            Stmt::While(w) => {
                loop {
                    let cond = self.eval_expr(&w.cond)?;
                    if !as_bool(&cond) {
                        break;
                    }
                    match self.eval_block(&w.body)? {
                        EvalFlow::Value(_) => {}
                        EvalFlow::Return(v) => return Ok(EvalFlow::Return(v)),
                    }
                    if self.return_value.is_some() {
                        break;
                    }
                    match self.loop_signal.take() {
                        Some(LoopSignal::Break) => break,
                        Some(LoopSignal::Continue) => {}
                        None => {}
                    }
                }
                Ok(EvalFlow::Value(Value::Unit))
            }
            Stmt::PureBlock(b) => self.eval_block(b),
        }
    }

    fn eval_expr_flow(&mut self, expr: &Expr) -> Result<EvalFlow, RuntimeError> {
        match expr {
            Expr::Return { value, .. } => {
                let v = if let Some(v) = value {
                    self.eval_expr(v)?
                } else {
                    Value::Unit
                };
                Ok(EvalFlow::Return(v))
            }
            Expr::Break { .. } => {
                self.loop_signal = Some(LoopSignal::Break);
                Ok(EvalFlow::Value(Value::Unit))
            }
            Expr::Continue { .. } => {
                self.loop_signal = Some(LoopSignal::Continue);
                Ok(EvalFlow::Value(Value::Unit))
            }
            _ => self.eval_expr(expr).map(EvalFlow::Value),
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
        match expr {
            Expr::Literal(lit, _) => Ok(match lit {
                Literal::Int(v) => Value::Int(*v),
                Literal::Float(v) => Value::Float(v.parse::<f64>().unwrap_or(0.0)),
                Literal::String(v) => Value::String(v.clone()),
                Literal::Bool(v) => Value::Bool(*v),
            }),
            Expr::Ident(name, span) => {
                if let Some(v) = self.env.get(name) {
                    Ok(v)
                } else if self.functions.contains_key(name.as_str()) {
                    // Allow bare function names to be used as first-class references
                    Ok(Value::String(name.clone()))
                } else {
                    Err(RuntimeError {
                        message: format!("unknown identifier '{}'", name),
                        span: Some(*span),
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            Expr::Block(b) => match self.eval_block(b)? {
                EvalFlow::Value(v) => Ok(v),
                EvalFlow::Return(v) => { self.return_value = Some(v); Ok(Value::Unit) }
            },
            Expr::If(i) => {
                for (cond, block) in &i.branches {
                    let cv = self.eval_expr(cond)?;
                    if as_bool(&cv) {
                        return match self.eval_block(block)? {
                            EvalFlow::Value(v) => Ok(v),
                            EvalFlow::Return(v) => { self.return_value = Some(v); Ok(Value::Unit) }
                        };
                    }
                }
                if let Some(else_block) = &i.else_block {
                    match self.eval_block(else_block)? {
                        EvalFlow::Value(v) => Ok(v),
                        EvalFlow::Return(v) => { self.return_value = Some(v); Ok(Value::Unit) }
                    }
                } else {
                    Ok(Value::Unit)
                }
            }
            Expr::Match(m) => {
                let value = self.eval_expr(&m.expr)?;
                for arm in &m.arms {
                    if let Some(bindings) = self.try_match(&arm.pattern, &value) {
                        self.env.push();
                        for (k, v) in bindings {
                            self.env.define(k, v);
                        }
                        let out = self.eval_expr(&arm.expr);
                        self.env.pop();
                        return out;
                    }
                }
                Err(RuntimeError {
                    message: "non-exhaustive match at runtime".into(),
                    span: Some(m.span),
                    propagated_err: None,
                propagated_none: false,
                })
            }
            Expr::FnCall(call) => {
                let args = call
                    .args
                    .iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<Vec<_>, _>>()?;

                if let Some(Value::String(fn_name)) = args.first() {
                    if self.functions.contains_key(fn_name)
                        && (call.name == "List.map" || call.name == "List.filter"
                            || call.name == "List.reduce" || call.name == "List.sort_by"
                            || call.name == "List.for_each" || call.name == "List.find"
                            || call.name == "List.any" || call.name == "List.all"
                            || call.name == "List.flat_map")
                    {
                        return self
                            .call_builtin(&call.name, &args)?
                            .ok_or_else(|| RuntimeError {
                                message: format!("unknown function '{}'", call.name),
                                span: Some(call.span),
                                propagated_err: None,
                propagated_none: false,
                            });
                    }
                }

                self.call_function(&call.name, args, call.span)
            }
            Expr::MethodCall(call) => {
                let target = self.eval_expr(&call.target)?;
                let args = call
                    .args
                    .iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<Vec<_>, _>>()?;
                // If target is a module ref (String), resolve as a qualified function call
                if let Value::String(module_name) = &target {
                    let fn_name = format!("{}.{}", module_name, call.method);
                    if self.functions.contains_key(&fn_name) {
                        return self.call_function(&fn_name.clone(), args, call.span);
                    }
                }
                self.call_method(target, &call.method, args, call.span)
            }
            Expr::FieldAccess {
                target,
                field,
                span,
            } => {
                let obj = self.eval_expr(target)?;
                match obj {
                    // Module ref: resolve const as qualified env lookup (e.g. math.pi)
                    // or return a function ref for method calls (e.g. math.add)
                    Value::String(ref module_name) => {
                        let qualified = format!("{}.{}", module_name, field);
                        // Check env first (covers consts)
                        if let Some(v) = self.env.get(&qualified) {
                            return Ok(v);
                        }
                        // Then check functions
                        if self.functions.contains_key(&qualified) {
                            return Ok(Value::String(qualified));
                        }
                        Err(RuntimeError {
                            message: format!(
                                "module '{}' has no exported symbol '{}'",
                                module_name, field
                            ),
                            span: Some(*span),
                            propagated_err: None,
                propagated_none: false,
                        })
                    }
                    Value::Record { fields, .. } => {
                        fields.get(field).cloned().ok_or_else(|| RuntimeError {
                            message: format!("missing field '{}'", field),
                            span: Some(*span),
                            propagated_err: None,
                propagated_none: false,
                        })
                    }
                    _ => Err(RuntimeError {
                        message: "field access on non-record value".into(),
                        span: Some(*span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            Expr::Index {
                target,
                index,
                span,
            } => {
                let t = self.eval_expr(target)?;
                let i = self.eval_expr(index)?;
                match (t, i) {
                    (Value::List(items), Value::Int(idx)) => items
                        .get(idx as usize)
                        .cloned()
                        .ok_or_else(|| RuntimeError {
                            message: format!("list index {} out of bounds", idx),
                            span: Some(*span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    (Value::Tuple(items), Value::Int(idx)) => items
                        .get(idx as usize)
                        .cloned()
                        .ok_or_else(|| RuntimeError {
                            message: format!("tuple index {} out of bounds", idx),
                            span: Some(*span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    _ => Err(RuntimeError {
                        message: "indexing requires list/tuple and int index".into(),
                        span: Some(*span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            Expr::Pipeline { left, right, span } => {
                let left_v = self.eval_expr(left)?;
                match &**right {
                    Expr::FnCall(call) => {
                        let mut args = vec![left_v];
                        let mut rhs_args = call
                            .args
                            .iter()
                            .map(|a| self.eval_expr(a))
                            .collect::<Result<Vec<_>, _>>()?;
                        args.append(&mut rhs_args);
                        self.call_function(&call.name, args, call.span)
                    }
                    Expr::MethodCall(call) => {
                        let mut args = vec![left_v];
                        let mut rhs_args = call
                            .args
                            .iter()
                            .map(|a| self.eval_expr(a))
                            .collect::<Result<Vec<_>, _>>()?;
                        args.append(&mut rhs_args);
                        self.call_function(&call.method, args, call.span)
                    }
                    Expr::Ident(name, _) => self.call_function(name, vec![left_v], *span),
                    _ => Err(RuntimeError {
                        message: "pipeline RHS must be callable".into(),
                        span: Some(*span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_binary(l, *op, r, *span)
            }
            Expr::Unary { op, expr, span } => {
                let v = self.eval_expr(expr)?;
                match op {
                    UnaryOp::Neg => match v {
                        Value::Int(i) => Ok(Value::Int(-i)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err(RuntimeError {
                            message: "unary '-' requires int or float".into(),
                            span: Some(*span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!as_bool(&v))),
                }
            }
            Expr::Closure(c) => {
                // Capture the entire current environment for lexical scoping
                let captured_env: HashMap<String, Value> = self
                    .env
                    .scopes
                    .iter()
                    .flat_map(|scope| scope.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect();
                Ok(Value::Closure {
                    params: c.params.iter().map(|p| p.name.clone()).collect(),
                    body: c.body.clone(),
                    captured_env,
                })
            }
            Expr::RecordLiteral(r) => {
                let mut fields = HashMap::new();
                for (name, e, _) in &r.fields {
                    fields.insert(name.clone(), self.eval_expr(e)?);
                }
                Ok(Value::Record {
                    name: r.name.clone(),
                    fields,
                })
            }
            Expr::ListLiteral { elems, .. } => Ok(Value::List(
                elems
                    .iter()
                    .map(|e| self.eval_expr(e))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            Expr::TupleLiteral { elems, .. } => Ok(Value::Tuple(
                elems
                    .iter()
                    .map(|e| self.eval_expr(e))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            Expr::Return { value, .. } => {
                if let Some(v) = value {
                    self.eval_expr(v)
                } else {
                    Ok(Value::Unit)
                }
            }
            Expr::Break { .. } => {
                self.loop_signal = Some(LoopSignal::Break);
                Ok(Value::Unit)
            }
            Expr::Continue { .. } => {
                self.loop_signal = Some(LoopSignal::Continue);
                Ok(Value::Unit)
            }
            Expr::ErrorProp { expr, span } => {
                let v = self.eval_expr(expr)?;
                match v {
                    Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                        Ok(payload[0].clone())
                    }
                    Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                        Err(RuntimeError {
                            message: format!("error propagation: {:?}", payload[0]),
                            span: Some(*span),
                            propagated_err: Some(payload[0].clone()),
                            propagated_none: false,
                        })
                    }
                    Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                        Ok(payload[0].clone())
                    }
                    Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                        Err(RuntimeError {
                            message: "none propagation".into(),
                            span: Some(*span),
                            propagated_err: None,
                            propagated_none: true,
                        })
                    }
                    other => Ok(other),
                }
            }
        }
    }

    fn call_function(
        &mut self,
        name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // stdlib builtins
        if let Some(v) = self.call_builtin(name, &args)? {
            return Ok(v);
        }

        // tool declaration execution
        if self.tools.contains_key(name) {
            return self.call_tool(name, args, span);
        }

        // Enum tuple variant constructor
        if self.variant_constructors.contains(name) {
            return Ok(Value::Variant {
                name: name.to_string(),
                payload: args,
            });
        }

        // Check if the name resolves to a closure value in the environment
        if let Some(Value::Closure { params, body, captured_env }) = self.env.get(name) {
            return self.call_closure_value(params, body, captured_env, args, span);
        }

        let resolved_name = self
            .resolve_function_name(name)
            .ok_or_else(|| RuntimeError {
                message: format!("unknown function '{}'", name),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            })?;

        let f = self
            .functions
            .get(&resolved_name)
            .cloned()
            .ok_or_else(|| RuntimeError {
                message: format!("unknown function '{}'", name),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            })?;

        if f.params.len() != args.len() {
            return Err(RuntimeError {
                message: format!(
                    "function '{}' expected {} arguments, got {}",
                    name,
                    f.params.len(),
                    args.len()
                ),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            });
        }

        // Extract @retry and @timeout from annotations
        let retry_max: Option<i64> = f.annotations.iter()
            .find(|a| a.name == "retry")
            .and_then(|a| a.args.iter().find(|arg| arg.name == "max"))
            .and_then(|arg| if let lace_ast::AnnotationValue::Int(n) = &arg.value { Some(*n) } else { None });

        let timeout_ms: Option<i64> = f.annotations.iter()
            .find(|a| a.name == "timed")
            .and_then(|a| a.args.iter().find(|arg| arg.name == "ms"))
            .and_then(|arg| if let lace_ast::AnnotationValue::Int(n) = &arg.value { Some(*n) } else { None });

        let started = Instant::now();

        // Execute once
        let mut result = self.exec_fn_body(&f, args.clone(), span)?;

        // @retry: if result is Err, retry up to max times
        if let Some(max) = retry_max {
            for _ in 0..max {
                if !is_err_variant(&result) {
                    break;
                }
                result = self.exec_fn_body(&f, args.clone(), span)?;
            }
        }

        // @timeout: post-hoc check (best-effort for single-threaded interpreter)
        if let Some(ms) = timeout_ms {
            if started.elapsed().as_millis() as i64 > ms {
                return Ok(Value::Variant {
                    name: "Err".into(),
                    payload: vec![Value::String("timeout".into())],
                });
            }
        }

        Ok(result)
    }

    fn exec_fn_body(
        &mut self,
        f: &FunctionDef,
        args: Vec<Value>,
        _span: Span,
    ) -> Result<Value, RuntimeError> {
        self.call_stack.push(CallFrame {
            effects: f.effects.clone(),
        });
        self.env.push();
        for (p, a) in f.params.iter().zip(args.into_iter()) {
            self.env.define(p.clone(), a);
        }

        let eval_result = self.eval_block(&f.body);
        self.env.pop();
        self.call_stack.pop();

        // If a `?` propagated an Err value, catch it here and return Ok(Err(v))
        let eval_result = match eval_result {
            Err(RuntimeError { propagated_err: Some(err_val), .. }) => {
                return Ok(Value::Variant {
                    name: "Err".into(),
                    payload: vec![err_val],
                });
            }
            Err(RuntimeError { propagated_none: true, .. }) => {
                return Ok(Value::Variant {
                    name: "None".into(),
                    payload: vec![],
                });
            }
            other => other,
        };

        let out = match eval_result? {
            EvalFlow::Value(v) => self.return_value.take().unwrap_or(v),
            EvalFlow::Return(v) => v,
        };

        Ok(out)
    }

    /// Call a Value::Closure or Value::String (fn ref) with the given args.
    fn call_callable(&mut self, callable: Value, args: Vec<Value>, span: Span) -> Result<Value, RuntimeError> {
        match callable {
            Value::String(fn_name) => self.call_function(&fn_name.clone(), args, span),
            Value::Closure { params, body, captured_env } => {
                self.call_closure_value(params, body, captured_env, args, span)
            }
            other => Err(RuntimeError {
                message: format!("value is not callable: {}", display_value(&other)),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            }),
        }
    }

    fn call_closure_value(
        &mut self,
        params: Vec<String>,
        body: Block,
        captured_env: HashMap<String, Value>,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        if params.len() != args.len() {
            return Err(RuntimeError {
                message: format!(
                    "closure expected {} argument(s), got {}",
                    params.len(),
                    args.len()
                ),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            });
        }
        self.env.push();
        // Inject captured variables
        for (k, v) in &captured_env {
            self.env.define(k.clone(), v.clone());
        }
        // Bind parameters (overrides captured vars with same name)
        for (p, a) in params.iter().zip(args.into_iter()) {
            self.env.define(p.clone(), a);
        }
        let eval_result = self.eval_block(&body);
        self.env.pop();

        let eval_result = match eval_result {
            Err(RuntimeError { propagated_err: Some(err_val), .. }) => {
                return Ok(Value::Variant {
                    name: "Err".into(),
                    payload: vec![err_val],
                });
            }
            Err(RuntimeError { propagated_none: true, .. }) => {
                return Ok(Value::Variant {
                    name: "None".into(),
                    payload: vec![],
                });
            }
            other => other,
        };

        let out = match eval_result? {
            EvalFlow::Value(v) => self.return_value.take().unwrap_or(v),
            EvalFlow::Return(v) => v,
        };
        Ok(out)
    }

    fn resolve_function_name(&self, name: &str) -> Option<String> {
        if self.functions.contains_key(name) {
            return Some(name.to_string());
        }

        let qualified = format!("{}.{}", self.module_name, name);
        if self.functions.contains_key(&qualified) {
            return Some(qualified);
        }

        for def in self.functions.values() {
            if def.qualified_name == name {
                return Some(name.to_string());
            }
        }

        None
    }

    fn call_builtin(&mut self, name: &str, args: &[Value]) -> Result<Option<Value>, RuntimeError> {
        match name {
            "print" | "println" => {
                if let Some(entry) = self.replay_entry_for(name, "IO") {
                    let msg = args.get(0).map(display_value).unwrap_or_else(String::new);
                    if name == "println" {
                        println!("[REPLAYED] {msg}");
                    } else {
                        print!("[REPLAYED] {msg}");
                    }
                    return Ok(Some(self.parse_replay_output(&entry.output)));
                }
                let msg = args.get(0).map(display_value).unwrap_or_else(String::new);
                if name == "println" {
                    println!("{msg}");
                } else {
                    print!("{msg}");
                }
                self.log_effect(name, "IO", json!(args), JsonValue::Null, 0)?;
                Ok(Some(Value::Unit))
            }
            "type_of" => {
                let ty = if let Some(v) = args.first() {
                    value_type_name(v)
                } else {
                    "Unit".into()
                };
                Ok(Some(Value::String(ty)))
            }
            "now_unix" => {
                if let Some(entry) = self.replay_entry_for(name, "Time") {
                    eprintln!("[REPLAYED] now_unix");
                    return Ok(Some(self.parse_replay_output(&entry.output)));
                }
                let val = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::from_secs(0))
                    .as_secs() as i64;
                self.log_effect(name, "Time", json!(args), json!(val), 0)?;
                Ok(Some(Value::Int(val)))
            }
            "now_millis" => {
                if let Some(entry) = self.replay_entry_for(name, "Time") {
                    eprintln!("[REPLAYED] now_millis");
                    return Ok(Some(self.parse_replay_output(&entry.output)));
                }
                let val = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::from_secs(0))
                    .as_millis() as i64;
                self.log_effect(name, "Time", json!(args), json!(val), 0)?;
                Ok(Some(Value::Int(val)))
            }
            "to_string" => {
                let out = args.first().map(display_value).unwrap_or_default();
                Ok(Some(Value::String(out)))
            }
            "assert" => match (args.first(), args.get(1)) {
                (Some(Value::Bool(true)), _) => Ok(Some(Value::Unit)),
                (Some(Value::Bool(false)), Some(Value::String(message))) => Err(RuntimeError {
                    message: format!("assertion failed: {message}"),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
                (Some(Value::Bool(false)), _) => Err(RuntimeError {
                    message: "assertion failed".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
                _ => Err(RuntimeError {
                    message: "assert expects (Bool, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "assert_eq" => {
                let Some(actual) = args.first() else {
                    return Err(RuntimeError {
                        message: "assert_eq expects (actual, expected, message)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    });
                };
                let Some(expected) = args.get(1) else {
                    return Err(RuntimeError {
                        message: "assert_eq expects (actual, expected, message)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    });
                };
                let message = match args.get(2) {
                    Some(Value::String(s)) => Some(s.as_str()),
                    Some(_) => {
                        return Err(RuntimeError {
                            message: "assert_eq expects third argument to be String".into(),
                            span: None,
                            propagated_err: None,
                propagated_none: false,
                        });
                    }
                    None => None,
                };

                if actual == expected {
                    Ok(Some(Value::Unit))
                } else {
                    let mut msg = format!(
                        "assertion failed: expected values to be equal (left: {}, right: {})",
                        display_value(actual),
                        display_value(expected)
                    );
                    if let Some(extra) = message {
                        msg.push_str(&format!(": {extra}"));
                    }
                    Err(RuntimeError {
                        message: msg,
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "assert_err" => {
                let Some(value) = args.first() else {
                    return Err(RuntimeError {
                        message: "assert_err expects (result, message)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    });
                };
                let message = match args.get(1) {
                    Some(Value::String(s)) => Some(s.as_str()),
                    Some(_) => {
                        return Err(RuntimeError {
                            message: "assert_err expects second argument to be String".into(),
                            span: None,
                            propagated_err: None,
                propagated_none: false,
                        });
                    }
                    None => None,
                };

                match value {
                    Value::Variant { name, .. } if name == "Err" => Ok(Some(Value::Unit)),
                    _ => {
                        let mut msg = "assertion failed: expected result to be Err(_)".to_string();
                        if let Some(extra) = message {
                            msg.push_str(&format!(": {extra}"));
                        }
                        Err(RuntimeError {
                            message: msg,
                            span: None,
                            propagated_err: None,
                propagated_none: false,
                        })
                    }
                }
            }
            "List.length" => match args.first() {
                Some(Value::List(items)) => Ok(Some(Value::Int(items.len() as i64))),
                _ => Err(RuntimeError {
                    message: "List.length expects a list".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.range" => match (args.first(), args.get(1)) {
                (Some(Value::Int(start)), Some(Value::Int(end))) => {
                    let mut out = Vec::new();
                    if start <= end {
                        for i in *start..*end {
                            out.push(Value::Int(i));
                        }
                    }
                    Ok(Some(Value::List(out)))
                }
                _ => Err(RuntimeError {
                    message: "List.range expects (Int, Int)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.map" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.map expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        let mapped = self.call_callable(callable.clone(), vec![item], Span::default())?;
                        out.push(mapped);
                    }
                    Ok(Some(Value::List(out)))
                } else {
                    Err(RuntimeError {
                        message: "List.map expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.filter" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.filter expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    let mut out = Vec::new();
                    for item in items {
                        let keep = self.call_callable(callable.clone(), vec![item.clone()], Span::default())?;
                        if as_bool(&keep) {
                            out.push(item);
                        }
                    }
                    Ok(Some(Value::List(out)))
                } else {
                    Err(RuntimeError {
                        message: "List.filter expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.fold" => {
                let (list, init, callable) = match (args.first(), args.get(1), args.get(2)) {
                    (Some(l), Some(i), Some(c)) => (l.clone(), i.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.fold expects (List, init, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    let mut acc = init;
                    for item in items {
                        acc = self.call_callable(callable.clone(), vec![acc, item], Span::default())?;
                    }
                    Ok(Some(acc))
                } else {
                    Err(RuntimeError {
                        message: "List.fold expects (List, init, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.reduce" => {
                let (list, init, callable) = match (args.first(), args.get(1), args.get(2)) {
                    (Some(l), Some(i), Some(c)) => (l.clone(), i.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.reduce expects (List, init, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    let mut acc = init;
                    for item in items {
                        acc = self.call_callable(callable.clone(), vec![acc, item], Span::default())?;
                    }
                    Ok(Some(acc))
                } else {
                    Err(RuntimeError {
                        message: "List.reduce expects (List, init, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.sort_by" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.sort_by expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    let mut sorted = items.clone();
                    let mut sort_err: Option<RuntimeError> = None;
                    sorted.sort_by(|a, b| {
                        if sort_err.is_some() { return std::cmp::Ordering::Equal; }
                        match self.call_callable(callable.clone(), vec![a.clone(), b.clone()], Span::default()) {
                            Ok(Value::Int(n)) => {
                                if n < 0 { std::cmp::Ordering::Less }
                                else if n > 0 { std::cmp::Ordering::Greater }
                                else { std::cmp::Ordering::Equal }
                            }
                            Ok(_) => std::cmp::Ordering::Equal,
                            Err(e) => { sort_err = Some(e); std::cmp::Ordering::Equal }
                        }
                    });
                    if let Some(e) = sort_err { return Err(e); }
                    Ok(Some(Value::List(sorted)))
                } else {
                    Err(RuntimeError {
                        message: "List.sort_by expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.for_each" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.for_each expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    for item in items {
                        self.call_callable(callable.clone(), vec![item], Span::default())?;
                    }
                    Ok(Some(Value::Unit))
                } else {
                    Err(RuntimeError {
                        message: "List.for_each expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.find" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.find expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    for item in items {
                        let keep = self.call_callable(callable.clone(), vec![item.clone()], Span::default())?;
                        if as_bool(&keep) {
                            return Ok(Some(Value::Variant { name: "Some".into(), payload: vec![item] }));
                        }
                    }
                    Ok(Some(Value::Variant { name: "None".into(), payload: vec![] }))
                } else {
                    Err(RuntimeError {
                        message: "List.find expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.any" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.any expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    for item in items {
                        let r = self.call_callable(callable.clone(), vec![item], Span::default())?;
                        if as_bool(&r) { return Ok(Some(Value::Bool(true))); }
                    }
                    Ok(Some(Value::Bool(false)))
                } else {
                    Err(RuntimeError {
                        message: "List.any expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.all" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.all expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    for item in items {
                        let r = self.call_callable(callable.clone(), vec![item], Span::default())?;
                        if !as_bool(&r) { return Ok(Some(Value::Bool(false))); }
                    }
                    Ok(Some(Value::Bool(true)))
                } else {
                    Err(RuntimeError {
                        message: "List.all expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.flat_map" => {
                let (list, callable) = match (args.first(), args.get(1)) {
                    (Some(l), Some(c)) => (l.clone(), c.clone()),
                    _ => return Err(RuntimeError {
                        message: "List.flat_map expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    }),
                };
                if let Value::List(items) = list {
                    let mut out = Vec::new();
                    for item in items {
                        let mapped = self.call_callable(callable.clone(), vec![item], Span::default())?;
                        if let Value::List(inner) = mapped {
                            out.extend(inner);
                        } else {
                            out.push(mapped);
                        }
                    }
                    Ok(Some(Value::List(out)))
                } else {
                    Err(RuntimeError {
                        message: "List.flat_map expects (List, fn_ref)".into(),
                        span: None,
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            }
            "List.zip" => match (args.first(), args.get(1)) {
                (Some(Value::List(left)), Some(Value::List(right))) => {
                    let pairs = left.iter().zip(right.iter())
                        .map(|(a, b)| Value::Tuple(vec![a.clone(), b.clone()]))
                        .collect();
                    Ok(Some(Value::List(pairs)))
                }
                _ => Err(RuntimeError {
                    message: "List.zip expects (List, List)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.sort" => match args.first() {
                Some(Value::List(items)) => {
                    let mut sorted = items.clone();
                    sorted.sort_by(|a, b| cmp_values(a, b));
                    Ok(Some(Value::List(sorted)))
                }
                _ => Err(RuntimeError {
                    message: "List.sort expects a List".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.contains" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(val)) => {
                    Ok(Some(Value::Bool(items.contains(val))))
                }
                _ => Err(RuntimeError {
                    message: "List.contains expects (List, value)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.sum" => match args.first() {
                Some(Value::List(items)) => {
                    let mut sum_i = 0i64;
                    let mut sum_f = 0f64;
                    let mut is_float = false;
                    for v in items {
                        match v {
                            Value::Int(i) => sum_i += i,
                            Value::Float(f) => { is_float = true; sum_f += f; }
                            _ => return Err(RuntimeError {
                                message: "List.sum requires a list of numbers".into(),
                                span: None,
                                propagated_err: None,
                propagated_none: false,
                            }),
                        }
                    }
                    if is_float {
                        Ok(Some(Value::Float(sum_f + sum_i as f64)))
                    } else {
                        Ok(Some(Value::Int(sum_i)))
                    }
                }
                _ => Err(RuntimeError {
                    message: "List.sum expects a List".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.min" => match args.first() {
                Some(Value::List(items)) if !items.is_empty() => {
                    let mut m = items[0].clone();
                    for v in &items[1..] {
                        if cmp_values(v, &m) == std::cmp::Ordering::Less {
                            m = v.clone();
                        }
                    }
                    Ok(Some(Value::Variant { name: "Some".into(), payload: vec![m] }))
                }
                Some(Value::List(_)) => {
                    Ok(Some(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(RuntimeError {
                    message: "List.min expects a List".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.max" => match args.first() {
                Some(Value::List(items)) if !items.is_empty() => {
                    let mut m = items[0].clone();
                    for v in &items[1..] {
                        if cmp_values(v, &m) == std::cmp::Ordering::Greater {
                            m = v.clone();
                        }
                    }
                    Ok(Some(Value::Variant { name: "Some".into(), payload: vec![m] }))
                }
                Some(Value::List(_)) => {
                    Ok(Some(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(RuntimeError {
                    message: "List.max expects a List".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "List.get" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::Int(idx))) => {
                    let i = *idx as usize;
                    if i < items.len() {
                        Ok(Some(Value::Variant {
                            name: "Some".into(),
                            payload: vec![items[i].clone()],
                        }))
                    } else {
                        Ok(Some(Value::Variant {
                            name: "None".into(),
                            payload: vec![],
                        }))
                    }
                }
                _ => Err(RuntimeError {
                    message: "List.get expects (List, Int)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            // Map stdlib
            "Map.new" => Ok(Some(Value::Map(HashMap::new()))),
            "Map.insert" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::Map(m)), Some(Value::String(key)), Some(val)) => {
                    let mut new_map = m.clone();
                    new_map.insert(key.clone(), val.clone());
                    Ok(Some(Value::Map(new_map)))
                }
                _ => Err(RuntimeError {
                    message: "Map.insert expects (Map, String, value)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.get" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(key))) => {
                    Ok(Some(match m.get(key) {
                        Some(v) => Value::Variant { name: "Some".into(), payload: vec![v.clone()] },
                        None => Value::Variant { name: "None".into(), payload: vec![] },
                    }))
                }
                _ => Err(RuntimeError {
                    message: "Map.get expects (Map, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.remove" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(key))) => {
                    let mut new_map = m.clone();
                    new_map.remove(key);
                    Ok(Some(Value::Map(new_map)))
                }
                _ => Err(RuntimeError {
                    message: "Map.remove expects (Map, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.contains_key" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(key))) => {
                    Ok(Some(Value::Bool(m.contains_key(key))))
                }
                _ => Err(RuntimeError {
                    message: "Map.contains_key expects (Map, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.keys" => match args.first() {
                Some(Value::Map(m)) => {
                    let mut keys: Vec<Value> = m.keys().map(|k| Value::String(k.clone())).collect();
                    keys.sort_by(|a, b| cmp_values(a, b));
                    Ok(Some(Value::List(keys)))
                }
                _ => Err(RuntimeError {
                    message: "Map.keys expects a Map".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.values" => match args.first() {
                Some(Value::Map(m)) => {
                    let mut pairs: Vec<(&String, &Value)> = m.iter().collect();
                    pairs.sort_by_key(|(k, _)| k.as_str());
                    let vals: Vec<Value> = pairs.into_iter().map(|(_, v)| v.clone()).collect();
                    Ok(Some(Value::List(vals)))
                }
                _ => Err(RuntimeError {
                    message: "Map.values expects a Map".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.entries" => match args.first() {
                Some(Value::Map(m)) => {
                    let mut pairs: Vec<(&String, &Value)> = m.iter().collect();
                    pairs.sort_by_key(|(k, _)| k.as_str());
                    let entries: Vec<Value> = pairs.into_iter()
                        .map(|(k, v)| Value::Tuple(vec![Value::String(k.clone()), v.clone()]))
                        .collect();
                    Ok(Some(Value::List(entries)))
                }
                _ => Err(RuntimeError {
                    message: "Map.entries expects a Map".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Map.length" => match args.first() {
                Some(Value::Map(m)) => Ok(Some(Value::Int(m.len() as i64))),
                _ => Err(RuntimeError {
                    message: "Map.length expects a Map".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            // Result / Option variant constructors
            "Ok" => {
                let v = args.first().cloned().unwrap_or(Value::Unit);
                Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![v] }))
            }
            "Err" => {
                let v = args.first().cloned().unwrap_or(Value::Unit);
                Ok(Some(Value::Variant { name: "Err".into(), payload: vec![v] }))
            }
            "Some" => {
                let v = args.first().cloned().unwrap_or(Value::Unit);
                Ok(Some(Value::Variant { name: "Some".into(), payload: vec![v] }))
            }
            // File I/O stdlib
            "File.read" => match args.first() {
                Some(Value::String(path)) => {
                    match fs::read_to_string(path) {
                        Ok(content) => {
                            self.log_effect("File.read", "IO", json!([path]), json!(content), 0)?;
                            Ok(Some(Value::Variant {
                                name: "Ok".into(),
                                payload: vec![Value::String(content)],
                            }))
                        }
                        Err(e) => Ok(Some(Value::Variant {
                            name: "Err".into(),
                            payload: vec![Value::String(e.to_string())],
                        })),
                    }
                }
                _ => Err(RuntimeError {
                    message: "File.read expects (String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "File.write" => match (args.first(), args.get(1)) {
                (Some(Value::String(path)), Some(Value::String(content))) => {
                    match fs::write(path, content) {
                        Ok(()) => {
                            self.log_effect("File.write", "IO", json!([path]), json!(null), 0)?;
                            Ok(Some(Value::Variant {
                                name: "Ok".into(),
                                payload: vec![Value::Unit],
                            }))
                        }
                        Err(e) => Ok(Some(Value::Variant {
                            name: "Err".into(),
                            payload: vec![Value::String(e.to_string())],
                        })),
                    }
                }
                _ => Err(RuntimeError {
                    message: "File.write expects (String, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "File.exists" => match args.first() {
                Some(Value::String(path)) => {
                    let exists = Path::new(path).exists();
                    self.log_effect("File.exists", "IO", json!([path]), json!(exists), 0)?;
                    Ok(Some(Value::Bool(exists)))
                }
                _ => Err(RuntimeError {
                    message: "File.exists expects (String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "None" => Ok(Some(Value::Variant { name: "None".into(), payload: vec![] })),
            // Http stdlib
            "Http.get" => match args.first() {
                Some(Value::String(url)) => {
                    let started = Instant::now();
                    let result = ureq::get(url).call();
                    let duration_ms = started.elapsed().as_millis() as i64;
                    match result {
                        Ok(resp) => {
                            let body = resp.into_string().unwrap_or_default();
                            self.log_effect("Http.get", "Http", json!([url]), json!(body), duration_ms)?;
                            Ok(Some(Value::Variant {
                                name: "Ok".into(),
                                payload: vec![Value::String(body)],
                            }))
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            self.log_effect("Http.get", "Http", json!([url]), json!(msg), duration_ms)?;
                            Ok(Some(Value::Variant {
                                name: "Err".into(),
                                payload: vec![Value::String(msg)],
                            }))
                        }
                    }
                }
                _ => Err(RuntimeError {
                    message: "Http.get expects (String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Http.post" => match (args.first(), args.get(1)) {
                (Some(Value::String(url)), Some(Value::String(body))) => {
                    let started = Instant::now();
                    let result = ureq::post(url).send_string(body);
                    let duration_ms = started.elapsed().as_millis() as i64;
                    match result {
                        Ok(resp) => {
                            let resp_body = resp.into_string().unwrap_or_default();
                            self.log_effect("Http.post", "Http", json!([url, body]), json!(resp_body), duration_ms)?;
                            Ok(Some(Value::Variant {
                                name: "Ok".into(),
                                payload: vec![Value::String(resp_body)],
                            }))
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            self.log_effect("Http.post", "Http", json!([url, body]), json!(msg), duration_ms)?;
                            Ok(Some(Value::Variant {
                                name: "Err".into(),
                                payload: vec![Value::String(msg)],
                            }))
                        }
                    }
                }
                _ => Err(RuntimeError {
                    message: "Http.post expects (String, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Http.post_json" => match (args.first(), args.get(1)) {
                (Some(Value::String(url)), Some(body_val)) => {
                    let started = Instant::now();
                    let json_body = value_to_json(body_val).to_string();
                    let result = ureq::post(url)
                        .set("Content-Type", "application/json")
                        .send_string(&json_body);
                    let duration_ms = started.elapsed().as_millis() as i64;
                    match result {
                        Ok(resp) => {
                            let resp_body = resp.into_string().unwrap_or_default();
                            self.log_effect("Http.post_json", "Http", json!([url, json_body]), json!(resp_body), duration_ms)?;
                            Ok(Some(Value::Variant {
                                name: "Ok".into(),
                                payload: vec![Value::String(resp_body)],
                            }))
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            self.log_effect("Http.post_json", "Http", json!([url, json_body]), json!(msg), duration_ms)?;
                            Ok(Some(Value::Variant {
                                name: "Err".into(),
                                payload: vec![Value::String(msg)],
                            }))
                        }
                    }
                }
                _ => Err(RuntimeError {
                    message: "Http.post_json expects (String, value)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            // Json stdlib
            "Json.parse" => match args.first() {
                Some(Value::String(s)) => {
                    match serde_json::from_str::<JsonValue>(s) {
                        Ok(jv) => Ok(Some(Value::Variant {
                            name: "Ok".into(),
                            payload: vec![json_to_lace_value(jv)],
                        })),
                        Err(e) => Ok(Some(Value::Variant {
                            name: "Err".into(),
                            payload: vec![Value::String(e.to_string())],
                        })),
                    }
                }
                _ => Err(RuntimeError {
                    message: "Json.parse expects (String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Json.stringify" => {
                let val = args.first().cloned().unwrap_or(Value::Unit);
                let json_str = value_to_json(&val).to_string();
                Ok(Some(Value::String(json_str)))
            }
            "Json.get" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(key))) => {
                    Ok(Some(match m.get(key) {
                        Some(v) => Value::Variant { name: "Some".into(), payload: vec![v.clone()] },
                        None => Value::Variant { name: "None".into(), payload: vec![] },
                    }))
                }
                _ => Err(RuntimeError {
                    message: "Json.get expects (Map, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Json.keys" => match args.first() {
                Some(Value::Map(m)) => {
                    let mut keys: Vec<Value> = m.keys().map(|k| Value::String(k.clone())).collect();
                    keys.sort_by(|a, b| cmp_values(a, b));
                    Ok(Some(Value::List(keys)))
                }
                _ => Err(RuntimeError {
                    message: "Json.keys expects (Map)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Json.index" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::Int(i))) => {
                    let idx = *i as usize;
                    Ok(Some(if idx < items.len() {
                        Value::Variant { name: "Some".into(), payload: vec![items[idx].clone()] }
                    } else {
                        Value::Variant { name: "None".into(), payload: vec![] }
                    }))
                },
                _ => Err(RuntimeError { message: "Json.index expects (List, Int)".into(), span: None, propagated_err: None, propagated_none: false })
            },
            // ── Regex stdlib ─────────────────────────────────────────────────
            "Regex.is_match" => match (args.first(), args.get(1)) {
                (Some(Value::String(pattern)), Some(Value::String(text))) => {
                    match StdRegex::new(pattern) {
                        Ok(re) => Ok(Some(Value::Bool(re.is_match(text)))),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("invalid regex: {}", e))] })),
                    }
                }
                _ => Err(RuntimeError { message: "Regex.is_match expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Regex.find" => match (args.first(), args.get(1)) {
                (Some(Value::String(pattern)), Some(Value::String(text))) => {
                    match StdRegex::new(pattern) {
                        Ok(re) => match re.find(text) {
                            Some(m) => Ok(Some(Value::Variant { name: "Some".into(), payload: vec![Value::String(m.as_str().to_string())] })),
                            None => Ok(Some(Value::Variant { name: "None".into(), payload: vec![] })),
                        },
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("invalid regex: {}", e))] })),
                    }
                }
                _ => Err(RuntimeError { message: "Regex.find expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Regex.find_all" => match (args.first(), args.get(1)) {
                (Some(Value::String(pattern)), Some(Value::String(text))) => {
                    match StdRegex::new(pattern) {
                        Ok(re) => {
                            let matches: Vec<Value> = re.find_iter(text).map(|m| Value::String(m.as_str().to_string())).collect();
                            Ok(Some(Value::List(matches)))
                        }
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("invalid regex: {}", e))] })),
                    }
                }
                _ => Err(RuntimeError { message: "Regex.find_all expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Regex.replace" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(pattern)), Some(Value::String(text)), Some(Value::String(replacement))) => {
                    match StdRegex::new(pattern) {
                        Ok(re) => Ok(Some(Value::String(re.replacen(text, 1, replacement.as_str()).to_string()))),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("invalid regex: {}", e))] })),
                    }
                }
                _ => Err(RuntimeError { message: "Regex.replace expects (String, String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Regex.replace_all" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(pattern)), Some(Value::String(text)), Some(Value::String(replacement))) => {
                    match StdRegex::new(pattern) {
                        Ok(re) => Ok(Some(Value::String(re.replace_all(text, replacement.as_str()).to_string()))),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("invalid regex: {}", e))] })),
                    }
                }
                _ => Err(RuntimeError { message: "Regex.replace_all expects (String, String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Regex.captures" => match (args.first(), args.get(1)) {
                (Some(Value::String(pattern)), Some(Value::String(text))) => {
                    match StdRegex::new(pattern) {
                        Ok(re) => match re.captures(text) {
                            Some(caps) => {
                                let groups: Vec<Value> = caps.iter().map(|m| match m {
                                    Some(m) => Value::String(m.as_str().to_string()),
                                    None => Value::String("".to_string()),
                                }).collect();
                                Ok(Some(Value::List(groups)))
                            }
                            None => Ok(Some(Value::List(vec![]))),
                        },
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("invalid regex: {}", e))] })),
                    }
                }
                _ => Err(RuntimeError { message: "Regex.captures expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            // ── Json.validate ────────────────────────────────────────────────
            "Json.validate" => match (args.first(), args.get(1)) {
                (Some(Value::Map(data)), Some(Value::Map(schema))) => {
                    let mut result: Result<Option<Value>, RuntimeError> = Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::Unit] }));
                    for (key, expected_type) in schema {
                        if let Value::String(type_str) = expected_type {
                            match data.get(key) {
                                None => {
                                    result = Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("field {}: missing", key))] }));
                                    break;
                                }
                                Some(val) => {
                                    let actual_type = match val {
                                        Value::String(_) => "string",
                                        Value::Int(_) => "int",
                                        Value::Float(_) => "float",
                                        Value::Bool(_) => "bool",
                                        Value::List(_) => "list",
                                        Value::Map(_) => "map",
                                        _ => "unknown",
                                    };
                                    let matches = match type_str.as_str() {
                                        "number" => matches!(val, Value::Int(_) | Value::Float(_)),
                                        t => actual_type == t,
                                    };
                                    if !matches {
                                        result = Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("field {}: expected {} got {}", key, type_str, actual_type))] }));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    result
                }
                _ => Err(RuntimeError { message: "Json.validate expects (Map, Map)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            // Env stdlib
            "Env.get" => match args.first() {
                Some(Value::String(key)) => {
                    match std::env::var(key) {
                        Ok(val) => Ok(Some(Value::Variant {
                            name: "Some".into(),
                            payload: vec![Value::String(val)],
                        })),
                        Err(_) => Ok(Some(Value::Variant {
                            name: "None".into(),
                            payload: vec![],
                        })),
                    }
                }
                _ => Err(RuntimeError {
                    message: "Env.get expects (String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "Env.set" => match (args.first(), args.get(1)) {
                (Some(Value::String(key)), Some(Value::String(val))) => {
                    std::env::set_var(key, val);
                    Ok(Some(Value::Unit))
                }
                _ => Err(RuntimeError {
                    message: "Env.set expects (String, String)".into(),
                    span: None,
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            // ── Fs stdlib ────────────────────────────────────────────────────
            "Fs.read" => match args.first() {
                Some(Value::String(path)) => {
                    match fs::read_to_string(path) {
                        Ok(content) => Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::String(content)] })),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                    }
                }
                _ => Err(RuntimeError { message: "Fs.read expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Fs.write" => match (args.first(), args.get(1)) {
                (Some(Value::String(path)), Some(Value::String(content))) => {
                    let p = std::path::Path::new(path);
                    if let Some(parent) = p.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    match fs::write(p, content) {
                        Ok(()) => Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::Unit] })),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                    }
                }
                _ => Err(RuntimeError { message: "Fs.write expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Fs.append" => match (args.first(), args.get(1)) {
                (Some(Value::String(path)), Some(Value::String(content))) => {
                    let p = std::path::Path::new(path);
                    if let Some(parent) = p.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    match OpenOptions::new().create(true).append(true).open(p) {
                        Ok(mut file) => match file.write_all(content.as_bytes()) {
                            Ok(()) => Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::Unit] })),
                            Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                        },
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                    }
                }
                _ => Err(RuntimeError { message: "Fs.append expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Fs.exists" => match args.first() {
                Some(Value::String(path)) => Ok(Some(Value::Bool(std::path::Path::new(path).exists()))),
                _ => Err(RuntimeError { message: "Fs.exists expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Fs.delete" => match args.first() {
                Some(Value::String(path)) => {
                    match fs::remove_file(path) {
                        Ok(()) => Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::Unit] })),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                    }
                }
                _ => Err(RuntimeError { message: "Fs.delete expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Fs.list_dir" => match args.first() {
                Some(Value::String(path)) => {
                    match fs::read_dir(path) {
                        Ok(entries) => {
                            let mut names = Vec::new();
                            for entry in entries.flatten() {
                                names.push(Value::String(entry.file_name().to_string_lossy().to_string()));
                            }
                            names.sort_by(|a, b| cmp_values(a, b));
                            Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::List(names)] }))
                        }
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                    }
                }
                _ => Err(RuntimeError { message: "Fs.list_dir expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            // ── Time stdlib ──────────────────────────────────────────────────
            "Time.now" => {
                let val = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                Ok(Some(Value::Int(val)))
            }
            "Time.now_ms" => {
                let val = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64;
                Ok(Some(Value::Int(val)))
            }
            "Time.format" => match (args.first(), args.get(1)) {
                (Some(Value::Int(ts)), Some(Value::String(fmt))) => {
                    let dt: DateTime<Utc> = Utc.timestamp_opt(*ts, 0).single().unwrap_or_else(Utc::now);
                    Ok(Some(Value::String(dt.format(fmt).to_string())))
                }
                _ => Err(RuntimeError { message: "Time.format expects (Int, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Time.parse" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(fmt))) => {
                    match NaiveDateTime::parse_from_str(s, fmt) {
                        Ok(ndt) => Ok(Some(Value::Variant { name: "Ok".into(), payload: vec![Value::Int(ndt.and_utc().timestamp())] })),
                        Err(e) => Ok(Some(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] })),
                    }
                }
                _ => Err(RuntimeError { message: "Time.parse expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Time.since" => match args.first() {
                Some(Value::Int(ts)) => {
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                    Ok(Some(Value::Int(now - ts)))
                }
                _ => Err(RuntimeError { message: "Time.since expects (Int)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            // ── Str stdlib ───────────────────────────────────────────────────
            "Str.split" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(delim))) => {
                    let parts: Vec<Value> = s.split(delim.as_str()).map(|p| Value::String(p.to_string())).collect();
                    Ok(Some(Value::List(parts)))
                }
                _ => Err(RuntimeError { message: "Str.split expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.join" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::String(sep))) => {
                    let parts: Vec<String> = items.iter().map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => display_value(other),
                    }).collect();
                    Ok(Some(Value::String(parts.join(sep))))
                }
                _ => Err(RuntimeError { message: "Str.join expects (List, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.trim" => match args.first() {
                Some(Value::String(s)) => Ok(Some(Value::String(s.trim().to_string()))),
                _ => Err(RuntimeError { message: "Str.trim expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.replace" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(s)), Some(Value::String(from)), Some(Value::String(to))) => {
                    Ok(Some(Value::String(s.replace(from.as_str(), to.as_str()))))
                }
                _ => Err(RuntimeError { message: "Str.replace expects (String, String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.contains" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(sub))) => Ok(Some(Value::Bool(s.contains(sub.as_str())))),
                _ => Err(RuntimeError { message: "Str.contains expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.starts_with" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(prefix))) => Ok(Some(Value::Bool(s.starts_with(prefix.as_str())))),
                _ => Err(RuntimeError { message: "Str.starts_with expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.ends_with" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(suffix))) => Ok(Some(Value::Bool(s.ends_with(suffix.as_str())))),
                _ => Err(RuntimeError { message: "Str.ends_with expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.to_upper" => match args.first() {
                Some(Value::String(s)) => Ok(Some(Value::String(s.to_uppercase()))),
                _ => Err(RuntimeError { message: "Str.to_upper expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.to_lower" => match args.first() {
                Some(Value::String(s)) => Ok(Some(Value::String(s.to_lowercase()))),
                _ => Err(RuntimeError { message: "Str.to_lower expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.len" => match args.first() {
                Some(Value::String(s)) => Ok(Some(Value::Int(s.chars().count() as i64))),
                _ => Err(RuntimeError { message: "Str.len expects (String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.slice" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(s)), Some(Value::Int(start)), Some(Value::Int(end))) => {
                    let chars: Vec<char> = s.chars().collect();
                    let len = chars.len() as i64;
                    let st = (*start).max(0).min(len) as usize;
                    let en = (*end).max(0).min(len) as usize;
                    let slice: String = chars[st.min(en)..en.max(st)].iter().collect();
                    Ok(Some(Value::String(slice)))
                }
                _ => Err(RuntimeError { message: "Str.slice expects (String, Int, Int)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.index_of" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(sub))) => {
                    let idx = s.find(sub.as_str()).map(|byte_idx| s[..byte_idx].chars().count() as i64).unwrap_or(-1);
                    Ok(Some(Value::Int(idx)))
                }
                _ => Err(RuntimeError { message: "Str.index_of expects (String, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.pad_left" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(s)), Some(Value::Int(width)), Some(Value::String(ch))) => {
                    let pad_char = ch.chars().next().unwrap_or(' ');
                    let len = s.chars().count();
                    let w = *width as usize;
                    if len >= w {
                        Ok(Some(Value::String(s.clone())))
                    } else {
                        let padding: String = std::iter::repeat(pad_char).take(w - len).collect();
                        Ok(Some(Value::String(format!("{}{}", padding, s))))
                    }
                }
                _ => Err(RuntimeError { message: "Str.pad_left expects (String, Int, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.pad_right" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(s)), Some(Value::Int(width)), Some(Value::String(ch))) => {
                    let pad_char = ch.chars().next().unwrap_or(' ');
                    let len = s.chars().count();
                    let w = *width as usize;
                    if len >= w {
                        Ok(Some(Value::String(s.clone())))
                    } else {
                        let padding: String = std::iter::repeat(pad_char).take(w - len).collect();
                        Ok(Some(Value::String(format!("{}{}", s, padding))))
                    }
                }
                _ => Err(RuntimeError { message: "Str.pad_right expects (String, Int, String)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.repeat" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::Int(n))) => {
                    Ok(Some(Value::String(s.repeat((*n).max(0) as usize))))
                }
                _ => Err(RuntimeError { message: "Str.repeat expects (String, Int)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            "Str.char_at" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::Int(i))) => {
                    let chars: Vec<char> = s.chars().collect();
                    let idx = *i as usize;
                    match chars.get(idx) {
                        Some(c) => Ok(Some(Value::String(c.to_string()))),
                        None => Ok(Some(Value::String(String::new()))),
                    }
                }
                _ => Err(RuntimeError { message: "Str.char_at expects (String, Int)".into(), span: None, propagated_err: None, propagated_none: false }),
            },
            _ => Ok(None),
        }
    }

    fn call_method(
        &mut self,
        target: Value,
        method: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        // Module method dispatch: e.g. List.range(0, 10) where target is Value::String("List")
        if let Value::String(module_name) = &target {
            let qualified = format!("{}.{}", module_name, method);
            if let Some(v) = self.call_builtin(&qualified, &args)? {
                return Ok(v);
            }
            if self.functions.contains_key(&qualified) {
                return self.call_function(&qualified, args, span);
            }
        }

        match method {
            // Option methods
            "is_some" => match target {
                Value::Variant { ref name, .. } => Ok(Value::Bool(name == "Some")),
                _ => Ok(Value::Bool(false)),
            },
            "is_none" => match target {
                Value::Variant { ref name, .. } => Ok(Value::Bool(name == "None")),
                _ => Ok(Value::Bool(false)),
            },
            "unwrap_or" => match target {
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                    Ok(args.first().cloned().unwrap_or(Value::Unit))
                }
                Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, .. } if name == "Err" => {
                    Ok(args.first().cloned().unwrap_or(Value::Unit))
                }
                _ => Err(RuntimeError {
                    message: "unwrap_or expects Option or Result value".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "unwrap" => match target {
                Value::Variant { name, payload } if (name == "Some" || name == "Ok" || name == "Confident") && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, payload } if name == "Ok" && payload.is_empty() => {
                    Ok(Value::Unit)
                }
                Value::Variant { name, .. } if name == "None" => Err(RuntimeError {
                    message: "unwrap called on None".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
                Value::Variant { name, payload } if name == "Err" => Err(RuntimeError {
                    message: format!("unwrap called on Err({})", payload.first().map(display_value).unwrap_or_default()),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
                _ => Err(RuntimeError {
                    message: "unwrap expects Some(_), Ok(_), or Confident(_)".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "map" => {
                let callable = args.first().cloned();
                match target {
                    Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                        if let Some(f) = callable {
                            let mapped = self.call_callable(f, vec![payload[0].clone()], span)?;
                            Ok(Value::Variant { name: "Some".into(), payload: vec![mapped] })
                        } else {
                            Ok(Value::Variant { name: "Some".into(), payload })
                        }
                    }
                    Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                        Ok(Value::Variant { name: "None".into(), payload: vec![] })
                    }
                    Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                        if let Some(f) = callable {
                            let mapped = self.call_callable(f, vec![payload[0].clone()], span)?;
                            Ok(Value::Variant { name: "Ok".into(), payload: vec![mapped] })
                        } else {
                            Ok(Value::Variant { name: "Ok".into(), payload })
                        }
                    }
                    Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                        Ok(Value::Variant { name: "Err".into(), payload })
                    }
                    Value::List(items) => {
                        if let Some(f) = callable {
                            let mut out = Vec::with_capacity(items.len());
                            for item in items {
                                let mapped = self.call_callable(f.clone(), vec![item], span)?;
                                out.push(mapped);
                            }
                            Ok(Value::List(out))
                        } else {
                            Ok(Value::List(items))
                        }
                    }
                    _ => Err(RuntimeError {
                        message: "map expects Option/Result/List value".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "and_then" => {
                let callable = args.first().cloned();
                match target {
                    Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                        if let Some(f) = callable {
                            self.call_callable(f, vec![payload[0].clone()], span)
                        } else {
                            Ok(Value::Variant { name: "Some".into(), payload })
                        }
                    }
                    Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                        Ok(Value::Variant { name: "None".into(), payload: vec![] })
                    }
                    Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                        if let Some(f) = callable {
                            self.call_callable(f, vec![payload[0].clone()], span)
                        } else {
                            Ok(Value::Variant { name: "Ok".into(), payload })
                        }
                    }
                    Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                        Ok(Value::Variant { name: "Err".into(), payload })
                    }
                    _ => Err(RuntimeError {
                        message: "and_then expects Option or Result".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "unwrap_or_else" => match target {
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                    if let Some(callable) = args.first().cloned() {
                        self.call_callable(callable, vec![], span)
                    } else {
                        Ok(Value::Unit)
                    }
                }
                Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                    if let Some(callable) = args.first().cloned() {
                        self.call_callable(callable, vec![payload[0].clone()], span)
                    } else {
                        Ok(Value::Unit)
                    }
                }
                _ => Err(RuntimeError {
                    message: "unwrap_or_else expects Option or Result".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "filter" if matches!(&target, Value::Variant { .. }) => {
                let callable = args.first().cloned();
                match target {
                    Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                        if let Some(f) = callable {
                            let keep = self.call_callable(f, vec![payload[0].clone()], span)?;
                            if as_bool(&keep) {
                                Ok(Value::Variant { name: "Some".into(), payload })
                            } else {
                                Ok(Value::Variant { name: "None".into(), payload: vec![] })
                            }
                        } else {
                            Ok(Value::Variant { name: "Some".into(), payload })
                        }
                    }
                    Value::Variant { name, payload: _ } if name == "None" => {
                        Ok(Value::Variant { name: "None".into(), payload: vec![] })
                    }
                    _ => Err(RuntimeError {
                        message: "filter on Option expects Some or None".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "ok_or" => match target {
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    Ok(Value::Variant { name: "Ok".into(), payload })
                }
                Value::Variant { name, .. } if name == "None" => {
                    let err = args.first().cloned().unwrap_or(Value::Unit);
                    Ok(Value::Variant { name: "Err".into(), payload: vec![err] })
                }
                _ => Err(RuntimeError {
                    message: "ok_or expects Option value".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "map_err" => {
                let callable = args.first().cloned();
                match target {
                    Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                        Ok(Value::Variant { name: "Ok".into(), payload })
                    }
                    Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                        if let Some(f) = callable {
                            let mapped = self.call_callable(f, vec![payload[0].clone()], span)?;
                            Ok(Value::Variant { name: "Err".into(), payload: vec![mapped] })
                        } else {
                            Ok(Value::Variant { name: "Err".into(), payload })
                        }
                    }
                    _ => Err(RuntimeError {
                        message: "map_err expects Result value".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "is_ok" => match target {
                Value::Variant { ref name, .. } => Ok(Value::Bool(name == "Ok")),
                _ => Ok(Value::Bool(false)),
            },
            "is_err" => match target {
                Value::Variant { ref name, .. } => Ok(Value::Bool(name == "Err")),
                _ => Ok(Value::Bool(false)),
            },
            "ok" if matches!(&target, Value::Variant { name, .. } if name == "Ok" || name == "Err") => {
                match target {
                    Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                        Ok(Value::Variant { name: "Some".into(), payload })
                    }
                    Value::Variant { name, .. } if name == "Err" => {
                        Ok(Value::Variant { name: "None".into(), payload: vec![] })
                    }
                    _ => Err(RuntimeError {
                        message: "ok() expects Result value".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "err" if matches!(&target, Value::Variant { name, .. } if name == "Ok" || name == "Err") => {
                match target {
                    Value::Variant { name, .. } if name == "Ok" => {
                        Ok(Value::Variant { name: "None".into(), payload: vec![] })
                    }
                    Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                        Ok(Value::Variant { name: "Some".into(), payload })
                    }
                    _ => Err(RuntimeError {
                        message: "err() expects Result value".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }

            // String helpers
            "len" => match target {
                Value::String(s) => Ok(Value::Int(s.len() as i64)),
                _ => Err(RuntimeError {
                    message: "len expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "trim" => match target {                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                _ => Err(RuntimeError {
                    message: "trim expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "split" => match target {
                Value::String(s) => {
                    let delim = args.first().map(display_value).unwrap_or_default();
                    Ok(Value::List(
                        s.split(&delim)
                            .map(|x| Value::String(x.to_string()))
                            .collect(),
                    ))
                }
                _ => Err(RuntimeError {
                    message: "split expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "contains" => match target {
                Value::String(s) => {
                    let needle = args.first().map(display_value).unwrap_or_default();
                    Ok(Value::Bool(s.contains(&needle)))
                }
                _ => Err(RuntimeError {
                    message: "contains expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "starts_with" => match target {
                Value::String(s) => {
                    let needle = args.first().map(display_value).unwrap_or_default();
                    Ok(Value::Bool(s.starts_with(&needle)))
                }
                _ => Err(RuntimeError {
                    message: "starts_with expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "ends_with" => match target {
                Value::String(s) => {
                    let needle = args.first().map(display_value).unwrap_or_default();
                    Ok(Value::Bool(s.ends_with(&needle)))
                }
                _ => Err(RuntimeError {
                    message: "ends_with expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "to_upper" => match target {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(RuntimeError {
                    message: "to_upper expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "to_lower" => match target {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(RuntimeError {
                    message: "to_lower expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },

            // Additional string methods
            "replace" => match target {
                Value::String(s) => {
                    let from = args.first().map(display_value).unwrap_or_default();
                    let to = args.get(1).map(display_value).unwrap_or_default();
                    Ok(Value::String(s.replace(&from, &to)))
                }
                _ => Err(RuntimeError {
                    message: "replace expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "is_empty" => match target {
                Value::String(s) => Ok(Value::Bool(s.is_empty())),
                Value::List(items) => Ok(Value::Bool(items.is_empty())),
                _ => Err(RuntimeError {
                    message: "is_empty expects String or List".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "char_at" => match target {
                Value::String(s) => {
                    let idx = match args.first() {
                        Some(Value::Int(i)) => *i,
                        _ => return Err(RuntimeError {
                            message: "char_at expects an Int index".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    };
                    let chars: Vec<char> = s.chars().collect();
                    if idx < 0 || idx as usize >= chars.len() {
                        Ok(Value::Variant { name: "None".into(), payload: vec![] })
                    } else {
                        Ok(Value::Variant {
                            name: "Some".into(),
                            payload: vec![Value::String(chars[idx as usize].to_string())],
                        })
                    }
                }
                _ => Err(RuntimeError {
                    message: "char_at expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "parse_int" => match target {
                Value::String(s) => {
                    match s.trim().parse::<i64>() {
                        Ok(n) => Ok(Value::Variant { name: "Ok".into(), payload: vec![Value::Int(n)] }),
                        Err(e) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] }),
                    }
                }
                _ => Err(RuntimeError {
                    message: "parse_int expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "parse_float" => match target {
                Value::String(s) => {
                    match s.trim().parse::<f64>() {
                        Ok(f) => Ok(Value::Variant { name: "Ok".into(), payload: vec![Value::Float(f)] }),
                        Err(e) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] }),
                    }
                }
                _ => Err(RuntimeError {
                    message: "parse_float expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "to_int" => match target {
                Value::Int(n) => Ok(Value::Int(n)),
                Value::Float(f) => Ok(Value::Int(f as i64)),
                Value::String(s) => {
                    match s.trim().parse::<i64>() {
                        Ok(n) => Ok(Value::Variant { name: "Ok".into(), payload: vec![Value::Int(n)] }),
                        Err(e) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] }),
                    }
                }
                _ => Err(RuntimeError {
                    message: "to_int expects Int, Float, or String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "to_float" => match target {
                Value::Float(f) => Ok(Value::Float(f)),
                Value::Int(n) => Ok(Value::Float(n as f64)),
                Value::String(s) => {
                    match s.trim().parse::<f64>() {
                        Ok(f) => Ok(Value::Variant { name: "Ok".into(), payload: vec![Value::Float(f)] }),
                        Err(e) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] }),
                    }
                }
                _ => Err(RuntimeError {
                    message: "to_float expects Int, Float, or String".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            // Numeric methods
            "abs" => match target {
                Value::Int(n) => Ok(Value::Int(n.abs())),
                Value::Float(f) => Ok(Value::Float(f.abs())),
                _ => Err(RuntimeError {
                    message: "abs expects Int or Float".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "floor" => match target {
                Value::Float(f) => Ok(Value::Float(f.floor())),
                Value::Int(n) => Ok(Value::Int(n)),
                _ => Err(RuntimeError {
                    message: "floor expects Float or Int".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "ceil" => match target {
                Value::Float(f) => Ok(Value::Float(f.ceil())),
                Value::Int(n) => Ok(Value::Int(n)),
                _ => Err(RuntimeError {
                    message: "ceil expects Float or Int".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "round" => match target {
                Value::Float(f) => Ok(Value::Float(f.round())),
                Value::Int(n) => Ok(Value::Int(n)),
                _ => Err(RuntimeError {
                    message: "round expects Float or Int".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "sqrt" => match target {
                Value::Float(f) => Ok(Value::Float(f.sqrt())),
                Value::Int(n) => Ok(Value::Float((n as f64).sqrt())),
                _ => Err(RuntimeError {
                    message: "sqrt expects Float or Int".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "pow" => match target {
                Value::Float(f) => {
                    let exp = match args.first() {
                        Some(Value::Float(e)) => *e,
                        Some(Value::Int(e)) => *e as f64,
                        _ => return Err(RuntimeError {
                            message: "pow expects a numeric exponent".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    };
                    Ok(Value::Float(f.powf(exp)))
                }
                Value::Int(n) => {
                    match args.first() {
                        Some(Value::Int(e)) if *e >= 0 => {
                            Ok(Value::Int(n.pow(*e as u32)))
                        }
                        Some(Value::Int(e)) => Ok(Value::Float((n as f64).powi(*e as i32))),
                        Some(Value::Float(e)) => Ok(Value::Float((n as f64).powf(*e))),
                        _ => Err(RuntimeError {
                            message: "pow expects a numeric exponent".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    }
                }
                _ => Err(RuntimeError {
                    message: "pow expects Float or Int".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "log" => match target {
                Value::Float(f) => {
                    let base = match args.first() {
                        Some(Value::Float(b)) => *b,
                        Some(Value::Int(b)) => *b as f64,
                        None => std::f64::consts::E,
                        _ => return Err(RuntimeError {
                            message: "log expects an optional numeric base".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    };
                    Ok(Value::Float(f.log(base)))
                }
                Value::Int(n) => {
                    let f = n as f64;
                    let base = match args.first() {
                        Some(Value::Float(b)) => *b,
                        Some(Value::Int(b)) => *b as f64,
                        None => std::f64::consts::E,
                        _ => return Err(RuntimeError {
                            message: "log expects an optional numeric base".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        }),
                    };
                    Ok(Value::Float(f.log(base)))
                }
                _ => Err(RuntimeError {
                    message: "log expects Float or Int".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            // List method-style helpers
            "filter" => {
                let callable = args.first().cloned();
                match target {
                    Value::List(items) => {
                        if let Some(f) = callable {
                            let mut out = Vec::new();
                            for item in items {
                                let keep = self.call_callable(f.clone(), vec![item.clone()], span)?;
                                if as_bool(&keep) {
                                    out.push(item);
                                }
                            }
                            Ok(Value::List(out))
                        } else {
                            Ok(Value::List(items))
                        }
                    }
                    _ => Err(RuntimeError {
                        message: "filter expects List".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "fold" => {
                let init = args.first().cloned().unwrap_or(Value::Unit);
                let callable = args.get(1).cloned();
                match target {
                    Value::List(items) => {
                        if let Some(f) = callable {
                            let mut acc = init;
                            for item in items {
                                acc = self.call_callable(f.clone(), vec![acc, item], span)?;
                            }
                            Ok(acc)
                        } else {
                            Ok(init)
                        }
                    }
                    _ => Err(RuntimeError {
                        message: "fold expects List".into(),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    }),
                }
            }
            "collect" => match target {
                Value::List(items) => Ok(Value::List(items)),
                _ => Err(RuntimeError {
                    message: "collect expects List".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "zip" => match (target, args.first().cloned()) {
                (Value::List(left), Some(Value::List(right))) => {
                    let pairs = left
                        .into_iter()
                        .zip(right)
                        .map(|(a, b)| Value::Tuple(vec![a, b]))
                        .collect();
                    Ok(Value::List(pairs))
                }
                _ => Err(RuntimeError {
                    message: "zip expects two lists".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "enumerate" => match target {
                Value::List(items) => {
                    let out = items
                        .into_iter()
                        .enumerate()
                        .map(|(idx, v)| Value::Tuple(vec![Value::Int(idx as i64), v]))
                        .collect();
                    Ok(Value::List(out))
                }
                _ => Err(RuntimeError {
                    message: "enumerate expects List".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },

            "candidates" => match target {
                Value::Variant { name, payload } if name == "Uncertain" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                _ => Err(RuntimeError {
                    message: "candidates expects Uncertain(list)".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "top" => match target {
                Value::Variant { name, payload } if name == "Uncertain" && payload.len() == 1 => {
                    if let Value::List(items) = &payload[0] {
                        if let Some(first) = items.first() {
                            Ok(Value::Variant {
                                name: "Some".into(),
                                payload: vec![first.clone()],
                            })
                        } else {
                            Ok(Value::Variant {
                                name: "None".into(),
                                payload: vec![],
                            })
                        }
                    } else {
                        Err(RuntimeError {
                            message: "Uncertain payload must be list".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        })
                    }
                }
                _ => Err(RuntimeError {
                    message: "top expects Uncertain(list)".into(),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
            },
            "to_string" => Ok(Value::String(display_value(&target))),
            _ => Err(RuntimeError {
                message: format!("unsupported method '{}'", method),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            }),
        }
    }

    fn call_tool(
        &mut self,
        tool_name: &str,
        args: Vec<Value>,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        let Some(tool) = self.tools.get(tool_name).cloned() else {
            return Err(RuntimeError {
                message: format!("unknown tool '{}'", tool_name),
                span: Some(span),
                propagated_err: None,
                propagated_none: false,
            });
        };

        if let Some(entry) = self.replay_tool_entry_for(tool_name) {
            eprintln!("[REPLAYED] tool {tool_name}");
            return Ok(self.parse_replay_output(&entry.inputs));
        }

        let mut shell_cmd: Option<String> = None;
        let mut http_method: Option<String> = None;
        let mut http_url: Option<String> = None;
        for ann in &tool.decl.annotations {
            match ann.name.as_str() {
                "shell" => {
                    if let Some(arg) = ann.args.first() {
                        if let AnnotationValue::String(cmd) = &arg.value {
                            shell_cmd = Some(cmd.clone());
                        }
                    }
                }
                "http" => {
                    if let Some(arg0) = ann.args.first() {
                        if let AnnotationValue::String(method) = &arg0.value {
                            http_method = Some(method.clone());
                        }
                    }
                    if let Some(arg1) = ann.args.get(1) {
                        if let AnnotationValue::String(url) = &arg1.value {
                            http_url = Some(url.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        let effect_tag = if shell_cmd.is_some() {
            "Shell"
        } else if http_method.is_some() && http_url.is_some() {
            "Http"
        } else {
            "ToolCall"
        };

        if let Some(frame) = self.call_stack.last() {
            if is_pure_only(&frame.effects) {
                panic!(
                    "Effect violation: pure fn called tool with [{}] effect",
                    effect_tag
                );
            }
        }

        let started = Instant::now();

        // Log the tool call
        {
            let arg_strs: Vec<String> = args.iter().map(|v| format!("{v:?}")).collect();
            self.tool_logger.log_call(tool_name, &arg_strs);
        }

        // mock option: call mock function if configured
        for option in &tool.decl.options {
            if let ToolOption::Mock(mock_name, _) = option {
                let out = self.call_function(mock_name, args.clone(), span)?;
                let duration_ms = started.elapsed().as_millis() as u64;
                self.log_effect(
                    tool_name,
                    effect_tag,
                    value_to_json(&out),
                    json!(args),
                    started.elapsed().as_millis() as i64,
                )?;
                let is_err = matches!(&out, Value::Variant { name, .. } if name == "Err");
                if is_err {
                    self.tool_logger.log_err(tool_name, "mock returned Err", duration_ms);
                } else {
                    self.tool_logger.log_ok(tool_name, duration_ms);
                }
                return Ok(out);
            }
        }

        let timeout = tool_timeout_duration(&tool.decl.options);

        if let Some(cmd_template) = shell_cmd {
            let cmd = interpolate_template(&cmd_template, &tool.decl.params, &args);
            let output = match run_shell_with_timeout(&cmd, timeout) {
                Ok(output) => output,
                Err(ShellExecError::Timeout) => {
                    let out = Value::Variant {
                        name: "Err".into(),
                        payload: vec![Value::Record {
                            name: "ToolError".into(),
                            fields: {
                                let mut m = HashMap::new();
                                m.insert("kind".into(), Value::String("Timeout".into()));
                                m.insert(
                                    "message".into(),
                                    Value::String(format!(
                                        "shell command timed out after {} ms",
                                        timeout.as_millis()
                                    )),
                                );
                                m
                            },
                        }],
                    };
                    self.log_effect(
                        tool_name,
                        effect_tag,
                        value_to_json(&out),
                        json!({"args": args, "command": cmd_template}),
                        started.elapsed().as_millis() as i64,
                    )?;
                    self.tool_logger.log_err(tool_name, "shell timeout", started.elapsed().as_millis() as u64);
                    return Ok(out);
                }
                Err(ShellExecError::Io(e)) => {
                    let out = Value::Variant {
                        name: "Err".into(),
                        payload: vec![Value::Record {
                            name: "ToolError".into(),
                            fields: {
                                let mut m = HashMap::new();
                                m.insert("kind".into(), Value::String("IoError".into()));
                                m.insert("message".into(), Value::String(e.to_string()));
                                m
                            },
                        }],
                    };
                    self.log_effect(
                        tool_name,
                        effect_tag,
                        value_to_json(&out),
                        json!({"args": args, "command": cmd_template}),
                        started.elapsed().as_millis() as i64,
                    )?;
                    self.tool_logger.log_err(tool_name, "shell io error", started.elapsed().as_millis() as u64);
                    return Ok(out);
                }
            };
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if !output.status.success() {
                let out = Value::Variant {
                    name: "Err".into(),
                    payload: vec![Value::Record {
                        name: "ToolError".into(),
                        fields: {
                            let mut m = HashMap::new();
                            m.insert("kind".into(), Value::String("ExitFailure".into()));
                            m.insert(
                                "message".into(),
                                Value::String(format!(
                                    "shell exited with status {:?}: {}",
                                    output.status.code(),
                                    stderr.trim()
                                )),
                            );
                            m.insert(
                                "code".into(),
                                Value::Int(output.status.code().unwrap_or(-1) as i64),
                            );
                            m.insert("stderr".into(), Value::String(stderr.clone()));
                            m
                        },
                    }],
                };
                self.log_effect(
                    tool_name,
                    effect_tag,
                    value_to_json(&out),
                    json!({"args": args, "command": cmd_template}),
                    started.elapsed().as_millis() as i64,
                )?;
                self.tool_logger.log_err(tool_name, "shell exit failure", started.elapsed().as_millis() as u64);
                return Ok(out);
            }

            let parsed = parse_tool_success_output(&tool.decl.ret_ty, &stdout);
            self.log_effect(
                tool_name,
                effect_tag,
                value_to_json(&parsed),
                json!({"args": args, "command": cmd_template}),
                started.elapsed().as_millis() as i64,
            )?;
            self.tool_logger.log_ok(tool_name, started.elapsed().as_millis() as u64);
            return Ok(parsed);
        }

        if let (Some(method), Some(url_template)) = (http_method, http_url) {
            let url = interpolate_template(&url_template, &tool.decl.params, &args);
            let method_upper = method.to_uppercase();
            let mut req = match method_upper.as_str() {
                "GET" => ureq::get(&url),
                "POST" => {
                    let mut b = ureq::post(&url);
                    b = b.set("content-type", "application/json");
                    b
                }
                _ => {
                    return Err(RuntimeError {
                        message: format!("unsupported @http method '{}'", method),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    })
                }
            };

            let timeout_ms = timeout.as_millis() as u64;
            req = req.timeout(timeout);

            let response_result = if method_upper == "POST" {
                let body = args
                    .first()
                    .map(value_to_json)
                    .unwrap_or_else(|| JsonValue::String(String::new()));
                req.send_string(&body.to_string())
            } else {
                req.call()
            };

            let response = match response_result {
                Ok(resp) => resp,
                Err(ureq::Error::Status(status, resp)) => {
                    let body = resp.into_string().unwrap_or_default();
                    let out = Value::Variant {
                        name: "Err".into(),
                        payload: vec![Value::Record {
                            name: "ToolError".into(),
                            fields: {
                                let mut m = HashMap::new();
                                m.insert("kind".into(), Value::String("HttpError".into()));
                                m.insert("status".into(), Value::Int(status as i64));
                                m.insert(
                                    "message".into(),
                                    Value::String(format!("http {} {} failed", method_upper, url)),
                                );
                                m.insert("body".into(), Value::String(body.clone()));
                                m
                            },
                        }],
                    };
                    self.log_effect(
                        tool_name,
                        effect_tag,
                        value_to_json(&out),
                        json!({"args": args, "url": url, "method": method_upper, "status": status as i64, "timeout_ms": timeout_ms}),
                        started.elapsed().as_millis() as i64,
                    )?;
                    self.tool_logger.log_err(tool_name, &format!("http status {status}"), started.elapsed().as_millis() as u64);
                    return Ok(out);
                }
                Err(ureq::Error::Transport(t)) => {
                    let kind = if t.kind() == ureq::ErrorKind::Io {
                        "Timeout"
                    } else {
                        "NetworkError"
                    };
                    let message = if kind == "Timeout" {
                        format!("http request timed out after {} ms", timeout_ms)
                    } else {
                        t.message().unwrap_or("network transport error").to_string()
                    };
                    let out = Value::Variant {
                        name: "Err".into(),
                        payload: vec![Value::Record {
                            name: "ToolError".into(),
                            fields: {
                                let mut m = HashMap::new();
                                m.insert("kind".into(), Value::String(kind.into()));
                                m.insert("message".into(), Value::String(message));
                                m
                            },
                        }],
                    };
                    self.log_effect(
                        tool_name,
                        effect_tag,
                        value_to_json(&out),
                        json!({"args": args, "url": url, "method": method_upper, "timeout_ms": timeout_ms}),
                        started.elapsed().as_millis() as i64,
                    )?;
                    self.tool_logger.log_err(tool_name, kind, started.elapsed().as_millis() as u64);
                    return Ok(out);
                }
            };

            let status = response.status() as i64;
            let body = response.into_string().unwrap_or_default();
            let parsed = parse_tool_success_output(&tool.decl.ret_ty, &body);
            self.log_effect(
                tool_name,
                effect_tag,
                value_to_json(&parsed),
                json!({"args": args, "url": url, "method": method_upper, "status": status, "timeout_ms": timeout_ms}),
                started.elapsed().as_millis() as i64,
            )?;
            self.tool_logger.log_ok(tool_name, started.elapsed().as_millis() as u64);
            return Ok(parsed);
        }

        let placeholder = placeholder_for_type(&tool.decl.ret_ty);
        eprintln!("[lace] W: tool '{}' has no dispatch annotation (@http/@shell/mock) — returning stub", tool_name);
        self.log_effect(
            tool_name,
            effect_tag,
            value_to_json(&placeholder),
            json!(args),
            started.elapsed().as_millis() as i64,
        )?;
        self.tool_logger.log_ok(tool_name, started.elapsed().as_millis() as u64);

        Ok(placeholder)
    }

    fn save_checkpoint_state(&self) -> Result<(), RuntimeError> {
        let Some(checkpoint_path) = &self.checkpoint_path else {
            return Ok(());
        };

        let scopes_json = self
            .env
            .scopes
            .iter()
            .map(|scope| {
                let mut m = serde_json::Map::new();
                for (k, v) in scope {
                    m.insert(k.clone(), value_to_json(v));
                }
                JsonValue::Object(m)
            })
            .collect::<Vec<_>>();

        let state = CheckpointState {
            run_id: self.run_id.clone(),
            seq: self.seq,
            module_name: self.module_name.clone(),
            journal_path: self.journal_path.clone(),
            checkpoint_path: checkpoint_path.clone(),
            env: JsonValue::Array(scopes_json),
        };

        let state_text = serde_json::to_string_pretty(&state).map_err(|e| RuntimeError {
            message: format!("failed to serialize checkpoint state: {e}"),
            span: None,
            propagated_err: None,
                propagated_none: false,
        })?;

        fs::write(checkpoint_path, state_text).map_err(|e| RuntimeError {
            message: format!("failed to write checkpoint file '{}': {e}", checkpoint_path),
            span: None,
            propagated_err: None,
                propagated_none: false,
        })
    }

    fn replay_entry_for(&mut self, fn_name: &str, effect: &str) -> Option<JournalEntry> {
        let replay = self.replay.as_mut()?;
        while replay.pos < replay.entries.len() {
            let entry = replay.entries[replay.pos].clone();
            replay.pos += 1;
            if entry.fn_name == fn_name && entry.effect == effect {
                self.seq = self.seq.max(entry.seq);
                return Some(entry);
            }
        }
        None
    }

    fn replay_tool_entry_for(&mut self, fn_name: &str) -> Option<JournalEntry> {
        let replay = self.replay.as_mut()?;
        while replay.pos < replay.entries.len() {
            let entry = replay.entries[replay.pos].clone();
            replay.pos += 1;
            if entry.fn_name == fn_name
                && (entry.effect == "ToolCall" || entry.effect == "Shell" || entry.effect == "Http")
            {
                self.seq = self.seq.max(entry.seq);
                return Some(entry);
            }
        }
        None
    }

    fn parse_replay_output(&self, output: &JsonValue) -> Value {
        json_to_value(output.clone())
    }

    fn log_effect(
        &mut self,
        fn_name: &str,
        effect: &str,
        inputs: JsonValue,
        output: JsonValue,
        duration_ms: i64,
    ) -> Result<(), RuntimeError> {
        self.seq += 1;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_millis() as i64;

        let entry = JournalEntry {
            id: format!("{}:{}", self.run_id, self.seq),
            run_id: self.run_id.clone(),
            seq: self.seq,
            timestamp,
            effect: effect.to_string(),
            fn_name: fn_name.to_string(),
            module: self.module_name.clone(),
            inputs,
            output,
            duration_ms,
        };

        let line = serde_json::to_string(&entry).map_err(|e| RuntimeError {
            message: format!("failed to serialize journal entry: {e}"),
            span: None,
            propagated_err: None,
                propagated_none: false,
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .map_err(|e| RuntimeError {
                message: format!("failed to open journal file '{}': {e}", self.journal_path),
                span: None,
                propagated_err: None,
                propagated_none: false,
            })?;

        writeln!(file, "{line}").map_err(|e| RuntimeError {
            message: format!("failed to write journal entry: {e}"),
            span: None,
            propagated_err: None,
                propagated_none: false,
        })
    }

    fn eval_binary(
        &self,
        left: Value,
        op: BinaryOp,
        right: Value,
        span: Span,
    ) -> Result<Value, RuntimeError> {
        match op {
            BinaryOp::Add => match (left, right) {
                (Value::Int(a), Value::Int(b)) => a.checked_add(b).map(Value::Int).ok_or_else(|| RuntimeError {
                    message: format!("integer overflow: {} + {}", a, b),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + b as f64)),
                _ => type_error(span, "'+' expects numeric operands"),
            },
            BinaryOp::Sub => match (left, right) {
                (Value::Int(a), Value::Int(b)) => a.checked_sub(b).map(Value::Int).ok_or_else(|| RuntimeError {
                    message: format!("integer overflow: {} - {}", a, b),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 - b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - b as f64)),
                _ => type_error(span, "'-' expects numeric operands"),
            },
            BinaryOp::Mul => match (left, right) {
                (Value::Int(a), Value::Int(b)) => a.checked_mul(b).map(Value::Int).ok_or_else(|| RuntimeError {
                    message: format!("integer overflow: {} * {}", a, b),
                    span: Some(span),
                    propagated_err: None,
                propagated_none: false,
                }),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 * b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * b as f64)),
                _ => type_error(span, "'*' expects numeric operands"),
            },
            BinaryOp::Div => match (left, right) {
                (Value::Int(a), Value::Int(b)) => {
                    if b == 0 {
                        return Err(RuntimeError {
                            message: "division by zero".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        });
                    }
                    a.checked_div(b).map(Value::Int).ok_or_else(|| RuntimeError {
                        message: format!("integer overflow: {} / {}", a, b),
                        span: Some(span),
                        propagated_err: None,
                propagated_none: false,
                    })
                }
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 / b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / b as f64)),
                _ => type_error(span, "'/' expects numeric operands"),
            },
            BinaryOp::IntDiv => match (left, right) {
                (Value::Int(a), Value::Int(b)) => {
                    if b == 0 {
                        return Err(RuntimeError {
                            message: "division by zero".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        });
                    }
                    Ok(Value::Int(a.div_euclid(b)))
                }
                _ => type_error(span, "'//' expects integer operands"),
            },
            BinaryOp::Rem => match (left, right) {
                (Value::Int(a), Value::Int(b)) => {
                    if b == 0 {
                        return Err(RuntimeError {
                            message: "remainder by zero".into(),
                            span: Some(span),
                            propagated_err: None,
                propagated_none: false,
                        });
                    }
                    Ok(Value::Int(a % b))
                }
                _ => type_error(span, "'%' expects integer operands"),
            },
            BinaryOp::Eq => Ok(Value::Bool(left == right)),
            BinaryOp::Ne => Ok(Value::Bool(left != right)),
            BinaryOp::Lt => cmp_bool(left, right, span, |a, b| a < b),
            BinaryOp::Gt => cmp_bool(left, right, span, |a, b| a > b),
            BinaryOp::Le => cmp_bool(left, right, span, |a, b| a <= b),
            BinaryOp::Ge => cmp_bool(left, right, span, |a, b| a >= b),
            BinaryOp::And => Ok(Value::Bool(as_bool(&left) && as_bool(&right))),
            BinaryOp::Or => Ok(Value::Bool(as_bool(&left) || as_bool(&right))),
            BinaryOp::Concat => Ok(Value::String(format!(
                "{}{}",
                display_value(&left),
                display_value(&right)
            ))),
        }
    }

    fn try_match(&self, pattern: &Pattern, value: &Value) -> Option<HashMap<String, Value>> {
        let mut bindings = HashMap::new();
        if self.try_match_into(pattern, value, &mut bindings) {
            Some(bindings)
        } else {
            None
        }
    }

    fn try_match_into(
        &self,
        pattern: &Pattern,
        value: &Value,
        bindings: &mut HashMap<String, Value>,
    ) -> bool {
        match pattern {
            Pattern::Wildcard(_) => true,
            Pattern::Literal(l, _) => match (l, value) {
                (Literal::Int(a), Value::Int(b)) => a == b,
                (Literal::Float(a), Value::Float(b)) => a.parse::<f64>().ok() == Some(*b),
                (Literal::String(a), Value::String(b)) => a == b,
                (Literal::Bool(a), Value::Bool(b)) => a == b,
                _ => false,
            },
            Pattern::Ident(name, _) => {
                // If the name refers to a known unit variant (uppercase, in env as Variant),
                // treat it as a structural match rather than a wildcard binding.
                if let Some(Value::Variant { name: vname, payload }) = self.env.get(name) {
                    if payload.is_empty() {
                        return matches!(value, Value::Variant { name: vn, .. } if vn == &vname);
                    }
                }
                bindings.insert(name.clone(), value.clone());
                true
            }
            Pattern::Tuple(parts, _) => {
                if let Value::Tuple(values) = value {
                    if parts.len() != values.len() {
                        return false;
                    }
                    for (p, v) in parts.iter().zip(values.iter()) {
                        if !self.try_match_into(p, v, bindings) {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }
            Pattern::EnumTuple { name, elems, .. } => {
                if let Value::Variant { name: vn, payload } = value {
                    if name != vn || elems.len() != payload.len() {
                        return false;
                    }
                    for (p, v) in elems.iter().zip(payload.iter()) {
                        if !self.try_match_into(p, v, bindings) {
                            return false;
                        }
                    }
                    true
                } else {
                    false
                }
            }
            Pattern::EnumStruct { .. } | Pattern::Record { .. } => false,
            Pattern::Or(left, right, _) => {
                let mut left_bindings = bindings.clone();
                if self.try_match_into(left, value, &mut left_bindings) {
                    *bindings = left_bindings;
                    return true;
                }

                let mut right_bindings = bindings.clone();
                if self.try_match_into(right, value, &mut right_bindings) {
                    *bindings = right_bindings;
                    return true;
                }

                false
            }
        }
    }
}

pub fn run(program: &Program) -> Result<Value, RuntimeError> {
    run_with_options(program, RunOptions::default())
}

pub fn run_with_options(program: &Program, options: RunOptions) -> Result<Value, RuntimeError> {
    Interpreter::new_with_options(program.module.as_ref().map(|m| m.path.join(".")), options)
        .run_program(program)
}

pub fn run_function_with_options(
    program: &Program,
    function_name: &str,
    options: RunOptions,
) -> Result<Value, RuntimeError> {
    Interpreter::new_with_options(program.module.as_ref().map(|m| m.path.join(".")), options)
        .run_named_function(program, function_name)
}

fn load_replay_cursor(path: &str) -> Result<ReplayCursor, RuntimeError> {
    let content = fs::read_to_string(path).map_err(|e| RuntimeError {
        message: format!("failed to read replay source '{}': {e}", path),
        span: None,
        propagated_err: None,
                propagated_none: false,
    })?;

    // If the path points to a checkpoint state JSON, follow its journal_path.
    if let Ok(state) = serde_json::from_str::<CheckpointState>(&content) {
        let journal_content =
            fs::read_to_string(&state.journal_path).map_err(|e| RuntimeError {
                message: format!(
                    "failed to read checkpoint journal '{}': {e}",
                    state.journal_path
                ),
                span: None,
                propagated_err: None,
                propagated_none: false,
            })?;
        let mut entries = Vec::new();
        for line in journal_content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<JournalEntry>(line) {
                entries.push(entry);
            }
        }
        return Ok(ReplayCursor { entries, pos: 0 });
    }

    // Otherwise treat as raw NDJSON journal.
    let mut entries = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<JournalEntry>(line) {
            entries.push(entry);
        }
    }
    Ok(ReplayCursor { entries, pos: 0 })
}

enum ShellExecError {
    Timeout,
    Io(std::io::Error),
}

fn run_shell_with_timeout(cmd: &str, timeout: Duration) -> Result<Output, ShellExecError> {
    let mut child = Command::new("bash")
        .arg("-lc")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ShellExecError::Io)?;

    let started = Instant::now();
    loop {
        if let Some(_status) = child.try_wait().map_err(ShellExecError::Io)? {
            return child.wait_with_output().map_err(ShellExecError::Io);
        }

        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ShellExecError::Timeout);
        }

        thread::sleep(Duration::from_millis(10));
    }
}

fn tool_timeout_duration(options: &[ToolOption]) -> Duration {
    for option in options {
        if let ToolOption::Timeout(duration, _) = option {
            return duration_lit_to_std(duration.clone());
        }
    }
    Duration::from_secs(30)
}

fn duration_lit_to_std(duration: DurationLit) -> Duration {
    let value = duration.value.max(0) as u64;
    match duration.unit {
        DurationUnit::Ms => Duration::from_millis(value),
        DurationUnit::S => Duration::from_secs(value),
        DurationUnit::M => Duration::from_secs(value.saturating_mul(60)),
        DurationUnit::H => Duration::from_secs(value.saturating_mul(3600)),
    }
}

fn is_pure_only(effects: &[EffectExpr]) -> bool {
    if effects.is_empty() {
        return false;
    }
    effects
        .iter()
        .all(|e| matches!(e, EffectExpr::Builtin(EffectTag::Pure)))
}

fn interpolate_template(template: &str, params: &[ToolParam], args: &[Value]) -> String {
    let mut out = template.to_string();
    for (idx, param) in params.iter().enumerate() {
        if let Some(arg) = args.get(idx) {
            out = out.replace(&format!("{{{}}}", param.name), &display_value(arg));
            out = out.replace(&format!("${{{}}}", param.name), &display_value(arg));
        }
    }
    out
}

fn parse_tool_success_output(ret_ty: &TypeExpr, text: &str) -> Value {
    if let TypeExpr::Generic { name, args, .. } = ret_ty {
        if name == "Result" && args.len() == 2 {
            let ok_val = parse_tool_ok_payload(&args[0], text);
            return Value::Variant {
                name: "Ok".into(),
                payload: vec![ok_val],
            };
        }
    }

    parse_tool_ok_payload(ret_ty, text)
}

fn parse_tool_ok_payload(ty: &TypeExpr, text: &str) -> Value {
    match ty {
        TypeExpr::Primitive(PrimitiveType::String, _) => Value::String(text.to_string()),
        TypeExpr::Dynamic(_) => serde_json::from_str::<JsonValue>(text)
            .map(json_to_value)
            .unwrap_or_else(|_| Value::String(text.to_string())),
        TypeExpr::Named { name, .. } if name == "String" => Value::String(text.to_string()),
        TypeExpr::Generic { name, .. } if name == "Json" => serde_json::from_str::<JsonValue>(text)
            .map(json_to_value)
            .unwrap_or_else(|_| Value::String(text.to_string())),
        _ => serde_json::from_str::<JsonValue>(text)
            .map(json_to_value)
            .unwrap_or_else(|_| Value::String(text.to_string())),
    }
}

fn json_to_value(v: JsonValue) -> Value {
    match v {
        JsonValue::Null => Value::Unit,
        JsonValue::Bool(b) => Value::Bool(b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        JsonValue::String(s) => Value::String(s),
        JsonValue::Array(arr) => Value::List(arr.into_iter().map(json_to_value).collect()),
        JsonValue::Object(obj) => {
            if let Some(JsonValue::String(tag)) = obj.get("__variant") {
                let payload = obj
                    .get("payload")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().cloned().map(json_to_value).collect())
                    .unwrap_or_default();
                return Value::Variant {
                    name: tag.clone(),
                    payload,
                };
            }

            if let Some(JsonValue::String(tag)) = obj.get("__record") {
                let fields = obj
                    .get("fields")
                    .and_then(|v| v.as_object())
                    .map(|m| {
                        m.iter()
                            .map(|(k, v)| (k.clone(), json_to_value(v.clone())))
                            .collect()
                    })
                    .unwrap_or_default();
                return Value::Record {
                    name: tag.clone(),
                    fields,
                };
            }

            Value::Record {
                name: "Json".into(),
                fields: obj
                    .into_iter()
                    .map(|(k, v)| (k, json_to_value(v)))
                    .collect(),
            }
        }
    }
}

fn value_to_json(v: &Value) -> JsonValue {
    match v {
        Value::Unit => JsonValue::Null,
        Value::Int(i) => json!(i),
        Value::Float(f) => json!(f),
        Value::Bool(b) => json!(b),
        Value::String(s) => json!(s),
        Value::List(xs) => JsonValue::Array(xs.iter().map(value_to_json).collect()),
        Value::Tuple(xs) => json!({
            "__tuple": xs.iter().map(value_to_json).collect::<Vec<_>>()
        }),
        Value::Record { name, fields } => {
            let mut map = serde_json::Map::new();
            map.insert("__record".into(), JsonValue::String(name.clone()));
            let f = fields
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect::<serde_json::Map<_, _>>();
            map.insert("fields".into(), JsonValue::Object(f));
            JsonValue::Object(map)
        }
        Value::Variant { name, payload } => json!({
            "__variant": name,
            "payload": payload.iter().map(value_to_json).collect::<Vec<_>>()
        }),
        Value::Map(m) => {
            let obj: serde_json::Map<String, JsonValue> = m.iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            JsonValue::Object(obj)
        }
        Value::Closure { params, .. } => json!({ "__closure": params }),
    }
}

fn type_error<T>(span: Span, msg: &str) -> Result<T, RuntimeError> {
    Err(RuntimeError {
        message: msg.to_string(),
        span: Some(span),
        propagated_err: None,
                propagated_none: false,
    })
}

fn cmp_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

fn as_bool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Float(f) => *f != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Unit => false,
        Value::List(xs) => !xs.is_empty(),
        Value::Tuple(xs) => !xs.is_empty(),
        Value::Record { .. } => true,
        Value::Variant { .. } => true,
        Value::Map(m) => !m.is_empty(),
        Value::Closure { .. } => true,
    }
}

fn display_value(v: &Value) -> String {
    match v {
        Value::Unit => "()".into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::List(xs) => {
            let inner: Vec<String> = xs.iter().map(display_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Tuple(xs) => {
            let inner: Vec<String> = xs.iter().map(display_value).collect();
            format!("({})", inner.join(", "))
        }
        Value::Record { name, fields } => {
            let mut pairs: Vec<_> = fields.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            let inner: Vec<String> = pairs.iter().map(|(k, v)| format!("{}: {}", k, display_value(v))).collect();
            format!("{} {{ {} }}", name, inner.join(", "))
        }
        Value::Variant { name, payload } if payload.is_empty() => name.clone(),
        Value::Variant { name, payload } if payload.len() == 1 => {
            format!("{}({})", name, display_value(&payload[0]))
        }
        Value::Variant { name, payload } => {
            let inner: Vec<String> = payload.iter().map(display_value).collect();
            format!("{}({})", name, inner.join(", "))
        }
        Value::Map(m) => {
            let mut pairs: Vec<_> = m.iter().collect();
            pairs.sort_by_key(|(k, _)| k.as_str());
            let inner: Vec<String> = pairs.iter().map(|(k, v)| format!("{}: {}", k, display_value(v))).collect();
            format!("{{{}}}", inner.join(", "))
        }
        Value::Closure { params, .. } => format!("<fn({})>", params.join(", ")),
    }
}

fn value_type_name(v: &Value) -> String {
    match v {
        Value::Unit => "Unit".into(),
        Value::Int(_) => "Int".into(),
        Value::Float(_) => "Float".into(),
        Value::Bool(_) => "Bool".into(),
        Value::String(_) => "String".into(),
        Value::List(_) => "List".into(),
        Value::Tuple(_) => "Tuple".into(),
        Value::Record { name, .. } => name.clone(),
        Value::Variant { name, .. } => name.clone(),
        Value::Map(_) => "Map".into(),
        Value::Closure { .. } => "Fn".into(),
    }
}

fn cmp_bool<F>(left: Value, right: Value, span: Span, f: F) -> Result<Value, RuntimeError>
where
    F: Fn(f64, f64) -> bool,
{
    let Some(l) = as_number(&left) else {
        return type_error(span, "comparison requires numeric operands");
    };
    let Some(r) = as_number(&right) else {
        return type_error(span, "comparison requires numeric operands");
    };
    Ok(Value::Bool(f(l, r)))
}

fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

fn placeholder_for_type(ty: &TypeExpr) -> Value {
    match ty {
        TypeExpr::Primitive(p, _) => match p {
            PrimitiveType::Int => Value::Int(0),
            PrimitiveType::Float => Value::Float(0.0),
            PrimitiveType::Bool => Value::Bool(false),
            PrimitiveType::String => Value::String("<stub>".into()),
            PrimitiveType::Bytes => Value::String("<bytes-stub>".into()),
            PrimitiveType::Unit => Value::Unit,
        },
        TypeExpr::Dynamic(_) => Value::String("<dynamic-stub>".into()),
        TypeExpr::Tuple { elems, .. } => {
            Value::Tuple(elems.iter().map(placeholder_for_type).collect())
        }
        TypeExpr::Named { name, .. } => Value::Variant {
            name: name.clone(),
            payload: vec![],
        },
        TypeExpr::Function { .. } => Value::String("<function-stub>".into()),
        TypeExpr::Generic { name, args, .. } => match name.as_str() {
            "Result" if args.len() == 2 => Value::Variant {
                name: "Ok".into(),
                payload: vec![placeholder_for_type(&args[0])],
            },
            "Option" if args.len() == 1 => Value::Variant {
                name: "Some".into(),
                payload: vec![placeholder_for_type(&args[0])],
            },
            "List" if args.len() == 1 => Value::List(vec![placeholder_for_type(&args[0])]),
            "Confident" if args.len() == 1 => Value::Variant {
                name: "Confident".into(),
                payload: vec![placeholder_for_type(&args[0])],
            },
            "Uncertain" if args.len() == 1 => Value::Variant {
                name: "Uncertain".into(),
                payload: vec![Value::List(vec![placeholder_for_type(&args[0])])],
            },
            "Scored" if args.len() == 1 => {
                let mut fields = HashMap::new();
                fields.insert("value".into(), placeholder_for_type(&args[0]));
                fields.insert("score".into(), Value::Float(0.5));
                Value::Record {
                    name: "Scored".into(),
                    fields,
                }
            }
            _ => Value::Variant {
                name: name.clone(),
                payload: vec![],
            },
        },
    }
}

/// Check whether a Value is an Err variant.
fn is_err_variant(v: &Value) -> bool {
    matches!(v, Value::Variant { name, .. } if name == "Err")
}

/// Convert a serde_json::Value to a Lace Value, mapping JSON null to None, objects to Map, etc.
fn json_to_lace_value(v: JsonValue) -> Value {
    match v {
        JsonValue::Null => Value::Variant { name: "None".into(), payload: vec![] },
        JsonValue::Bool(b) => Value::Bool(b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        JsonValue::String(s) => Value::String(s),
        JsonValue::Array(arr) => Value::List(arr.into_iter().map(json_to_lace_value).collect()),
        JsonValue::Object(obj) => {
            let map: HashMap<String, Value> = obj.into_iter()
                .map(|(k, v)| (k, json_to_lace_value(v)))
                .collect();
            Value::Map(map)
        }
    }
}
