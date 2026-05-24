use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RuntimeError {
    pub message: String,
    pub span: Option<Span>,
    /// When Some, this error was produced by the `?` operator propagating an Err value.
    /// call_function catches this and returns Ok(Err(v)) instead of propagating the error.
    pub propagated_err: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub checkpoint_path: Option<String>,
    pub replay_mode: bool,
    pub source_path: Option<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            checkpoint_path: None,
            replay_mode: false,
            source_path: None,
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
    call_stack: Vec<CallFrame>,
    loop_signal: Option<LoopSignal>,
    return_value: Option<Value>,
    variant_constructors: HashSet<String>,
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
            call_stack: Vec::new(),
            loop_signal: None,
            return_value: None,
            variant_constructors: HashSet::new(),
        }
    }

    pub fn run_program(mut self, program: &Program) -> Result<Value, RuntimeError> {
        // Register module name bindings so Lace code can do List.range(...), etc.
        self.env.define("List".into(), Value::String("List".into()));
        self.env.define("File".into(), Value::String("File".into()));

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
        let out = if self.functions.contains_key("main") {
            self.call_function("main", vec![], Span::default())
        } else {
            Ok(Value::Unit)
        }?;

        if self.checkpoint_path.is_some() {
            self.save_checkpoint_state()?;
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
            // Bind the last path segment as a module-ref value so Lace code can do module.fn(...)
            if let Some(name) = import.path.last() {
                let module_name = import.path.join(".");
                self.env.define(name.clone(), Value::String(module_name));
            }
        }

        Ok(())
    }

    fn load_module_from_import(
        &mut self,
        base_dir: &Path,
        import: &ImportDecl,
    ) -> Result<(), RuntimeError> {
        let module_name = import.path.join(".");
        if self.loaded_modules.contains(&module_name) {
            return Ok(());
        }

        let mut module_path = base_dir.to_path_buf();
        for part in &import.path {
            module_path.push(part);
        }
        module_path.set_extension("lace");

        let source = fs::read_to_string(&module_path).map_err(|e| RuntimeError {
            message: format!(
                "failed to import module '{}' from '{}': {e}",
                module_name,
                module_path.display()
            ),
            span: Some(import.span),
            propagated_err: None,
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
                    "failed to parse imported module '{}': {}",
                    module_name, joined
                ),
                span: Some(import.span),
                propagated_err: None,
            });
        }

        let program = parsed.ok_or_else(|| RuntimeError {
            message: format!("failed to parse imported module '{}'", module_name),
            span: Some(import.span),
            propagated_err: None,
        })?;

        if self.loaded_modules.contains(&module_name) {
            return Ok(());
        }
        self.loaded_modules.insert(module_name.clone());

        let prev_module = self.module_name.clone();
        let prev_dir = self.current_dir.clone();
        self.module_name = module_name;
        self.current_dir = module_path.parent().map(Path::to_path_buf);

        self.load_imports(&program)?;
        self.register_items(&program);

        self.current_dir = prev_dir;
        self.module_name = prev_module;

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
                        && (call.name == "List.map" || call.name == "List.filter")
                    {
                        return self
                            .call_builtin(&call.name, &args)?
                            .ok_or_else(|| RuntimeError {
                                message: format!("unknown function '{}'", call.name),
                                span: Some(call.span),
                                propagated_err: None,
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
                self.call_method(target, &call.method, args, call.span)
            }
            Expr::FieldAccess {
                target,
                field,
                span,
            } => {
                let obj = self.eval_expr(target)?;
                match obj {
                    Value::Record { fields, .. } => {
                        fields.get(field).cloned().ok_or_else(|| RuntimeError {
                            message: format!("missing field '{}'", field),
                            span: Some(*span),
                            propagated_err: None,
                        })
                    }
                    Value::String(module_name) => {
                        let fn_name = format!("{}.{}", module_name, field);
                        if self.functions.contains_key(&fn_name) {
                            Ok(Value::String(fn_name))
                        } else {
                            Err(RuntimeError {
                                message: format!(
                                    "module '{}' has no exported function '{}'",
                                    module_name, field
                                ),
                                span: Some(*span),
                                propagated_err: None,
                            })
                        }
                    }
                    _ => Err(RuntimeError {
                        message: "field access on non-record value".into(),
                        span: Some(*span),
                        propagated_err: None,
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
                        }),
                    (Value::Tuple(items), Value::Int(idx)) => items
                        .get(idx as usize)
                        .cloned()
                        .ok_or_else(|| RuntimeError {
                            message: format!("tuple index {} out of bounds", idx),
                            span: Some(*span),
                            propagated_err: None,
                        }),
                    _ => Err(RuntimeError {
                        message: "indexing requires list/tuple and int index".into(),
                        span: Some(*span),
                        propagated_err: None,
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
                        }),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!as_bool(&v))),
                }
            }
            Expr::Closure(_) => Err(RuntimeError {
                message: "closure runtime values are not implemented in Phase 2".into(),
                span: Some(expr.span()),
                propagated_err: None,
            }),
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

        let resolved_name = self
            .resolve_function_name(name)
            .ok_or_else(|| RuntimeError {
                message: format!("unknown function '{}'", name),
                span: Some(span),
                propagated_err: None,
            })?;

        let f = self
            .functions
            .get(&resolved_name)
            .cloned()
            .ok_or_else(|| RuntimeError {
                message: format!("unknown function '{}'", name),
                span: Some(span),
                propagated_err: None,
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
            });
        }

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
                }),
                (Some(Value::Bool(false)), _) => Err(RuntimeError {
                    message: "assertion failed".into(),
                    span: None,
                    propagated_err: None,
                }),
                _ => Err(RuntimeError {
                    message: "assert expects (Bool, String)".into(),
                    span: None,
                    propagated_err: None,
                }),
            },
            "assert_eq" => {
                let Some(actual) = args.first() else {
                    return Err(RuntimeError {
                        message: "assert_eq expects (actual, expected, message)".into(),
                        span: None,
                        propagated_err: None,
                    });
                };
                let Some(expected) = args.get(1) else {
                    return Err(RuntimeError {
                        message: "assert_eq expects (actual, expected, message)".into(),
                        span: None,
                        propagated_err: None,
                    });
                };
                let message = match args.get(2) {
                    Some(Value::String(s)) => Some(s.as_str()),
                    Some(_) => {
                        return Err(RuntimeError {
                            message: "assert_eq expects third argument to be String".into(),
                            span: None,
                            propagated_err: None,
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
                    })
                }
            }
            "assert_err" => {
                let Some(value) = args.first() else {
                    return Err(RuntimeError {
                        message: "assert_err expects (result, message)".into(),
                        span: None,
                        propagated_err: None,
                    });
                };
                let message = match args.get(1) {
                    Some(Value::String(s)) => Some(s.as_str()),
                    Some(_) => {
                        return Err(RuntimeError {
                            message: "assert_err expects second argument to be String".into(),
                            span: None,
                            propagated_err: None,
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
                }),
            },
            "List.map" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::String(fn_name))) => {
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        let mapped =
                            self.call_function(fn_name, vec![item.clone()], Span::default())?;
                        out.push(mapped);
                    }
                    Ok(Some(Value::List(out)))
                }
                _ => Err(RuntimeError {
                    message: "List.map expects (List, fn_ref)".into(),
                    span: None,
                    propagated_err: None,
                }),
            },
            "List.filter" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::String(fn_name))) => {
                    let mut out = Vec::new();
                    for item in items {
                        let keep =
                            self.call_function(fn_name, vec![item.clone()], Span::default())?;
                        if as_bool(&keep) {
                            out.push(item.clone());
                        }
                    }
                    Ok(Some(Value::List(out)))
                }
                _ => Err(RuntimeError {
                    message: "List.filter expects (List, fn_ref)".into(),
                    span: None,
                    propagated_err: None,
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
                }),
            },
            "None" => Ok(Some(Value::Variant { name: "None".into(), payload: vec![] })),
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
            // Option
            "unwrap_or" => match target {
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                    Ok(args.first().cloned().unwrap_or(Value::Unit))
                }
                _ => Err(RuntimeError {
                    message: "unwrap_or expects Option value".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "map" => match target {
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    Ok(Value::Variant {
                        name: "Some".into(),
                        payload,
                    })
                }
                Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                    Ok(Value::Variant {
                        name: "None".into(),
                        payload: vec![],
                    })
                }
                Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                    Ok(Value::Variant {
                        name: "Ok".into(),
                        payload,
                    })
                }
                Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                    Ok(Value::Variant {
                        name: "Err".into(),
                        payload,
                    })
                }
                Value::List(items) => Ok(Value::List(items)),
                _ => Err(RuntimeError {
                    message: "map expects Option/Result/List value".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "and_then" => match target {
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    Ok(Value::Variant {
                        name: "Some".into(),
                        payload,
                    })
                }
                Value::Variant { name, payload } if name == "None" && payload.is_empty() => {
                    Ok(Value::Variant {
                        name: "None".into(),
                        payload: vec![],
                    })
                }
                Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                    Ok(Value::Variant {
                        name: "Ok".into(),
                        payload,
                    })
                }
                Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                    Ok(Value::Variant {
                        name: "Err".into(),
                        payload,
                    })
                }
                _ => Err(RuntimeError {
                    message: "and_then expects Option or Result".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "unwrap_or_else" => match target {
                Value::Variant { name, payload } if name == "Ok" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                Value::Variant { name, payload } if name == "Err" && payload.len() == 1 => {
                    if let Some(Value::String(_fn_name)) = args.first() {
                        Ok(payload[0].clone())
                    } else {
                        Ok(Value::Unit)
                    }
                }
                _ => Err(RuntimeError {
                    message: "unwrap_or_else expects Result".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },

            // String helpers
            "len" => match target {
                Value::String(s) => Ok(Value::Int(s.len() as i64)),
                _ => Err(RuntimeError {
                    message: "len expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "trim" => match target {                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                _ => Err(RuntimeError {
                    message: "trim expects String".into(),
                    span: Some(span),
                    propagated_err: None,
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
                }),
            },
            "to_upper" => match target {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(RuntimeError {
                    message: "to_upper expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "to_lower" => match target {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(RuntimeError {
                    message: "to_lower expects String".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },

            // List helpers (minimal builtins)
            "filter" => match target {
                Value::List(items) => Ok(Value::List(items)),
                _ => Err(RuntimeError {
                    message: "filter expects List".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "fold" => match target {
                Value::List(_items) => Ok(args.first().cloned().unwrap_or(Value::Unit)),
                _ => Err(RuntimeError {
                    message: "fold expects List".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            "collect" => match target {
                Value::List(items) => Ok(Value::List(items)),
                _ => Err(RuntimeError {
                    message: "collect expects List".into(),
                    span: Some(span),
                    propagated_err: None,
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
                }),
            },

            // Existing
            "unwrap" => match target {
                Value::Variant { name, payload } if name == "Confident" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                _ => Err(RuntimeError {
                    message: "unwrap expects Confident(value)".into(),
                    span: Some(span),
                    propagated_err: None,
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
                        })
                    }
                }
                _ => Err(RuntimeError {
                    message: "top expects Uncertain(list)".into(),
                    span: Some(span),
                    propagated_err: None,
                }),
            },
            _ => Err(RuntimeError {
                message: format!("unsupported method '{}'", method),
                span: Some(span),
                propagated_err: None,
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

        // mock option: call mock function if configured
        for option in &tool.decl.options {
            if let ToolOption::Mock(mock_name, _) = option {
                let out = self.call_function(mock_name, args.clone(), span)?;
                self.log_effect(
                    tool_name,
                    effect_tag,
                    value_to_json(&out),
                    json!(args),
                    started.elapsed().as_millis() as i64,
                )?;
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
            return Ok(parsed);
        }

        let placeholder = placeholder_for_type(&tool.decl.ret_ty);
        self.log_effect(
            tool_name,
            effect_tag,
            value_to_json(&placeholder),
            json!(args),
            started.elapsed().as_millis() as i64,
        )?;

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
        })?;

        fs::write(checkpoint_path, state_text).map_err(|e| RuntimeError {
            message: format!("failed to write checkpoint file '{}': {e}", checkpoint_path),
            span: None,
            propagated_err: None,
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
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .map_err(|e| RuntimeError {
                message: format!("failed to open journal file '{}': {e}", self.journal_path),
                span: None,
                propagated_err: None,
            })?;

        writeln!(file, "{line}").map_err(|e| RuntimeError {
            message: format!("failed to write journal entry: {e}"),
            span: None,
            propagated_err: None,
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
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + b as f64)),
                _ => type_error(span, "'+' expects numeric operands"),
            },
            BinaryOp::Sub => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 - b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - b as f64)),
                _ => type_error(span, "'-' expects numeric operands"),
            },
            BinaryOp::Mul => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 * b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * b as f64)),
                _ => type_error(span, "'*' expects numeric operands"),
            },
            BinaryOp::Div => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a / b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(a as f64 / b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / b as f64)),
                _ => type_error(span, "'/' expects numeric operands"),
            },
            BinaryOp::IntDiv => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.div_euclid(b))),
                _ => type_error(span, "'//' expects integer operands"),
            },
            BinaryOp::Rem => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a % b)),
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
    }
}

fn type_error<T>(span: Span, msg: &str) -> Result<T, RuntimeError> {
    Err(RuntimeError {
        message: msg.to_string(),
        span: Some(span),
        propagated_err: None,
    })
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
    }
}

fn display_value(v: &Value) -> String {
    match v {
        Value::Unit => "()".into(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::List(xs) => format!("{:?}", xs),
        Value::Tuple(xs) => format!("{:?}", xs),
        Value::Record { name, fields } => format!("{} {:?}", name, fields),
        Value::Variant { name, payload } => format!("{}{:?}", name, payload),
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
