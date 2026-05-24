use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
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

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub checkpoint_path: Option<String>,
    pub replay_mode: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            checkpoint_path: None,
            replay_mode: false,
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

pub struct Interpreter {
    run_id: String,
    seq: u64,
    module_name: String,
    journal_path: String,
    checkpoint_path: Option<String>,
    replay: Option<ReplayCursor>,
    env: Env,
    functions: HashMap<String, FunctionDef>,
    tools: HashMap<String, ToolDef>,
    call_stack: Vec<CallFrame>,
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
            call_stack: Vec::new(),
        }
    }

    pub fn run_program(mut self, program: &Program) -> Result<Value, RuntimeError> {
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

    fn register_items(&mut self, program: &Program) {
        if let Some(module) = &program.module {
            self.module_name = module.path.join(".");
        }

        for item in &program.items {
            match item {
                TopLevelItem::Function(f) => {
                    self.functions.insert(
                        f.name.clone(),
                        FunctionDef {
                            params: f.params.iter().map(|p| p.name.clone()).collect(),
                            effects: f.effects.clone(),
                            body: f.body.clone(),
                        },
                    );
                }
                TopLevelItem::Tool(t) => {
                    self.tools
                        .insert(t.name.clone(), ToolDef { decl: t.clone() });
                }
                _ => {}
            }
        }
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
                    });
                }
                Ok(EvalFlow::Value(Value::Unit))
            }
            Stmt::Expr(e) => self.eval_expr_flow(e),
            Stmt::For(f) => {
                let iter = self.eval_expr(&f.iter)?;
                if let Value::List(items) = iter {
                    for item in items {
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
                    }
                    Ok(EvalFlow::Value(Value::Unit))
                } else {
                    Err(RuntimeError {
                        message: "for-loop requires a list iterator".into(),
                        span: Some(f.span),
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
            Expr::Ident(name, span) => self.env.get(name).ok_or_else(|| RuntimeError {
                message: format!("unknown identifier '{}'", name),
                span: Some(*span),
            }),
            Expr::Block(b) => match self.eval_block(b)? {
                EvalFlow::Value(v) => Ok(v),
                EvalFlow::Return(v) => Ok(v),
            },
            Expr::If(i) => {
                for (cond, block) in &i.branches {
                    let cv = self.eval_expr(cond)?;
                    if as_bool(&cv) {
                        return match self.eval_block(block)? {
                            EvalFlow::Value(v) => Ok(v),
                            EvalFlow::Return(v) => Ok(v),
                        };
                    }
                }
                if let Some(else_block) = &i.else_block {
                    match self.eval_block(else_block)? {
                        EvalFlow::Value(v) => Ok(v),
                        EvalFlow::Return(v) => Ok(v),
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
                })
            }
            Expr::FnCall(call) => {
                let args = call
                    .args
                    .iter()
                    .map(|a| self.eval_expr(a))
                    .collect::<Result<Vec<_>, _>>()?;
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
                        })
                    }
                    _ => Err(RuntimeError {
                        message: "field access on non-record value".into(),
                        span: Some(*span),
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
                        }),
                    (Value::Tuple(items), Value::Int(idx)) => items
                        .get(idx as usize)
                        .cloned()
                        .ok_or_else(|| RuntimeError {
                            message: format!("tuple index {} out of bounds", idx),
                            span: Some(*span),
                        }),
                    _ => Err(RuntimeError {
                        message: "indexing requires list/tuple and int index".into(),
                        span: Some(*span),
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
                        }),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!as_bool(&v))),
                }
            }
            Expr::Closure(_) => Err(RuntimeError {
                message: "closure runtime values are not implemented in Phase 2".into(),
                span: Some(expr.span()),
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

        let Some(f) = self.functions.get(name).cloned() else {
            return Err(RuntimeError {
                message: format!("unknown function '{}'", name),
                span: Some(span),
            });
        };

        if f.params.len() != args.len() {
            return Err(RuntimeError {
                message: format!(
                    "function '{}' expected {} arguments, got {}",
                    name,
                    f.params.len(),
                    args.len()
                ),
                span: Some(span),
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

        let out = match eval_result? {
            EvalFlow::Value(v) => v,
            EvalFlow::Return(v) => v,
        };

        Ok(out)
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
                }),
            },

            // String helpers
            "trim" => match target {
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                _ => Err(RuntimeError {
                    message: "trim expects String".into(),
                    span: Some(span),
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
                }),
            },
            "to_upper" => match target {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(RuntimeError {
                    message: "to_upper expects String".into(),
                    span: Some(span),
                }),
            },
            "to_lower" => match target {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(RuntimeError {
                    message: "to_lower expects String".into(),
                    span: Some(span),
                }),
            },

            // List helpers (minimal builtins)
            "filter" => match target {
                Value::List(items) => Ok(Value::List(items)),
                _ => Err(RuntimeError {
                    message: "filter expects List".into(),
                    span: Some(span),
                }),
            },
            "fold" => match target {
                Value::List(_items) => Ok(args.first().cloned().unwrap_or(Value::Unit)),
                _ => Err(RuntimeError {
                    message: "fold expects List".into(),
                    span: Some(span),
                }),
            },
            "collect" => match target {
                Value::List(items) => Ok(Value::List(items)),
                _ => Err(RuntimeError {
                    message: "collect expects List".into(),
                    span: Some(span),
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
                }),
            },
            "candidates" => match target {
                Value::Variant { name, payload } if name == "Uncertain" && payload.len() == 1 => {
                    Ok(payload[0].clone())
                }
                _ => Err(RuntimeError {
                    message: "candidates expects Uncertain(list)".into(),
                    span: Some(span),
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
                        })
                    }
                }
                _ => Err(RuntimeError {
                    message: "top expects Uncertain(list)".into(),
                    span: Some(span),
                }),
            },
            _ => Err(RuntimeError {
                message: format!("unsupported method '{}'", method),
                span: Some(span),
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
        })?;

        fs::write(checkpoint_path, state_text).map_err(|e| RuntimeError {
            message: format!("failed to write checkpoint file '{}': {e}", checkpoint_path),
            span: None,
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
        })?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.journal_path)
            .map_err(|e| RuntimeError {
                message: format!("failed to open journal file '{}': {e}", self.journal_path),
                span: None,
            })?;

        writeln!(file, "{line}").map_err(|e| RuntimeError {
            message: format!("failed to write journal entry: {e}"),
            span: None,
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

fn load_replay_cursor(path: &str) -> Result<ReplayCursor, RuntimeError> {
    let content = fs::read_to_string(path).map_err(|e| RuntimeError {
        message: format!("failed to read replay source '{}': {e}", path),
        span: None,
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
