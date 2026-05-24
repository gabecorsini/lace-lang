use std::collections::HashMap;

use lace_interp::{ToolLogger, Value};

use crate::chunk::Chunk;
use crate::error::VmError;
use crate::opcode::OpCode;

pub struct CallFrame {
    pub chunk_idx: usize,
    pub ip: usize,
    pub locals: Vec<Value>,
    pub base: usize,
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<CallFrame>,
    globals: HashMap<String, Value>,
    chunks: Vec<Chunk>,
    tool_logger: ToolLogger,
}

impl Vm {
    pub fn new(chunks: Vec<Chunk>, tool_log: bool) -> Self {
        let mut globals = HashMap::new();
        // Register all fn/tool chunks as globals by name → chunk index
        for (i, chunk) in chunks.iter().enumerate() {
            if chunk.name != "main" {
                globals.insert(chunk.name.clone(), Value::Int(i as i64));
            }
        }
        let tool_logger = ToolLogger::new(!tool_log, None);
        Vm { stack: Vec::new(), frames: Vec::new(), globals, chunks, tool_logger }
    }

    pub fn new_with_logger(chunks: Vec<Chunk>, tool_logger: ToolLogger) -> Self {
        let mut globals = HashMap::new();
        for (i, chunk) in chunks.iter().enumerate() {
            if chunk.name != "main" {
                globals.insert(chunk.name.clone(), Value::Int(i as i64));
            }
        }
        Vm { stack: Vec::new(), frames: Vec::new(), globals, chunks, tool_logger }
    }

    fn chunk_idx_by_name(&self, name: &str) -> Option<usize> {
        self.chunks.iter().position(|c| c.name == name)
    }

    pub fn run(&mut self) -> Result<Value, VmError> {
        // First run bootstrap to set up globals
        let bootstrap_idx = self.chunk_idx_by_name("__bootstrap__")
            .ok_or_else(|| VmError::RuntimeError("no bootstrap chunk".into()))?;
        self.frames.push(CallFrame {
            chunk_idx: bootstrap_idx,
            ip: 0,
            locals: Vec::new(),
            base: 0,
        });
        self.execute()?;

        // Then run main if it exists
        if let Some(main_idx) = self.chunk_idx_by_name("main") {
            self.frames.push(CallFrame {
                chunk_idx: main_idx,
                ip: 0,
                locals: Vec::new(),
                base: 0,
            });
            let result = self.execute();
            if let Some(summary) = self.tool_logger.summary() {
                eprintln!("{summary}");
            }
            result
        } else {
            if let Some(summary) = self.tool_logger.summary() {
                eprintln!("{summary}");
            }
            Ok(Value::Unit)
        }
    }

    fn execute(&mut self) -> Result<Value, VmError> {
        self.execute_until(0)
    }

    fn execute_until(&mut self, stop_at_depth: usize) -> Result<Value, VmError> {
        loop {
            if self.frames.len() <= stop_at_depth {
                return Ok(self.stack.pop().unwrap_or(Value::Unit));
            }

            let (chunk_idx, ip) = {
                let frame = self.frames.last().unwrap();
                (frame.chunk_idx, frame.ip)
            };

            if ip >= self.chunks[chunk_idx].code.len() {
                // End of chunk — return Unit
                self.frames.pop();
                self.stack.push(Value::Unit);
                continue;
            }

            // Clone opcode to avoid borrow issues
            let op = self.chunks[chunk_idx].code[ip].clone();
            self.frames.last_mut().unwrap().ip += 1;

            match op {
                OpCode::LoadConst(idx) => {
                    let val = self.chunks[chunk_idx].constants[idx].clone();
                    self.stack.push(val);
                }
                OpCode::LoadLocal(slot) => {
                    let frame = self.frames.last().unwrap();
                    let val = frame.locals.get(slot)
                        .cloned()
                        .ok_or_else(|| VmError::RuntimeError(format!("local slot {} out of range", slot)))?;
                    self.stack.push(val);
                }
                OpCode::StoreLocal(slot) => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let frame = self.frames.last_mut().unwrap();
                    // Extend locals if needed
                    while frame.locals.len() <= slot {
                        frame.locals.push(Value::Unit);
                    }
                    frame.locals[slot] = val;
                }
                OpCode::LoadGlobal(ref name) => {
                    // First try globals map, then check if it's a chunk name
                    let val = if let Some(v) = self.globals.get(name) {
                        v.clone()
                    } else if let Some(idx) = self.chunk_idx_by_name(name) {
                        Value::Int(idx as i64)
                    } else {
                        return Err(VmError::UndefinedVariable(name.clone()));
                    };
                    self.stack.push(val);
                }
                OpCode::StoreGlobal(ref name) => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    self.globals.insert(name.clone(), val);
                }
                OpCode::Pop => {
                    self.stack.pop().ok_or(VmError::StackUnderflow)?;
                }
                OpCode::Add => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(x + y),
                        (Value::Float(x), Value::Float(y)) => Value::Float(x + y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(x as f64 + y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x + y as f64),
                        (a, b) => return Err(VmError::TypeError(format!("cannot add {:?} and {:?}", a, b))),
                    };
                    self.stack.push(res);
                }
                OpCode::Sub => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(x - y),
                        (Value::Float(x), Value::Float(y)) => Value::Float(x - y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(x as f64 - y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x - y as f64),
                        (a, b) => return Err(VmError::TypeError(format!("cannot sub {:?} and {:?}", a, b))),
                    };
                    self.stack.push(res);
                }
                OpCode::Mul => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(x * y),
                        (Value::Float(x), Value::Float(y)) => Value::Float(x * y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(x as f64 * y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x * y as f64),
                        (a, b) => return Err(VmError::TypeError(format!("cannot mul {:?} and {:?}", a, b))),
                    };
                    self.stack.push(res);
                }
                OpCode::Div => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(_), Value::Int(0)) => return Err(VmError::DivisionByZero),
                        (Value::Float(_), Value::Float(y)) if y == 0.0 => return Err(VmError::DivisionByZero),
                        (Value::Int(x), Value::Int(y)) => Value::Float(x as f64 / y as f64),
                        (Value::Float(x), Value::Float(y)) => Value::Float(x / y),
                        (Value::Int(x), Value::Float(y)) => Value::Float(x as f64 / y),
                        (Value::Float(x), Value::Int(y)) => Value::Float(x / y as f64),
                        (a, b) => return Err(VmError::TypeError(format!("cannot div {:?} and {:?}", a, b))),
                    };
                    self.stack.push(res);
                }
                OpCode::IntDiv => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(_), Value::Int(0)) => return Err(VmError::DivisionByZero),
                        (Value::Int(x), Value::Int(y)) => Value::Int(x / y),
                        (a, b) => return Err(VmError::TypeError(format!("cannot intdiv {:?} and {:?}", a, b))),
                    };
                    self.stack.push(res);
                }
                OpCode::Mod => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(_), Value::Int(0)) => return Err(VmError::DivisionByZero),
                        (Value::Int(x), Value::Int(y)) => Value::Int(x % y),
                        (a, b) => return Err(VmError::TypeError(format!("cannot mod {:?} and {:?}", a, b))),
                    };
                    self.stack.push(res);
                }
                OpCode::Neg => {
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match a {
                        Value::Int(x) => Value::Int(-x),
                        Value::Float(x) => Value::Float(-x),
                        a => return Err(VmError::TypeError(format!("cannot negate {:?}", a))),
                    };
                    self.stack.push(res);
                }
                OpCode::Eq => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    self.stack.push(Value::Bool(a == b));
                }
                OpCode::Ne => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    self.stack.push(Value::Bool(a != b));
                }
                OpCode::Lt => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => x < y,
                        (Value::Float(x), Value::Float(y)) => x < y,
                        (a, b) => return Err(VmError::TypeError(format!("cannot compare {:?} < {:?}", a, b))),
                    };
                    self.stack.push(Value::Bool(res));
                }
                OpCode::Le => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => x <= y,
                        (Value::Float(x), Value::Float(y)) => x <= y,
                        (a, b) => return Err(VmError::TypeError(format!("cannot compare {:?} <= {:?}", a, b))),
                    };
                    self.stack.push(Value::Bool(res));
                }
                OpCode::Gt => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => x > y,
                        (Value::Float(x), Value::Float(y)) => x > y,
                        (a, b) => return Err(VmError::TypeError(format!("cannot compare {:?} > {:?}", a, b))),
                    };
                    self.stack.push(Value::Bool(res));
                }
                OpCode::Ge => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let res = match (a, b) {
                        (Value::Int(x), Value::Int(y)) => x >= y,
                        (Value::Float(x), Value::Float(y)) => x >= y,
                        (a, b) => return Err(VmError::TypeError(format!("cannot compare {:?} >= {:?}", a, b))),
                    };
                    self.stack.push(Value::Bool(res));
                }
                OpCode::And => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    match (a, b) {
                        (Value::Bool(x), Value::Bool(y)) => self.stack.push(Value::Bool(x && y)),
                        (a, b) => return Err(VmError::TypeError(format!("cannot and {:?} and {:?}", a, b))),
                    }
                }
                OpCode::Or => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    match (a, b) {
                        (Value::Bool(x), Value::Bool(y)) => self.stack.push(Value::Bool(x || y)),
                        (a, b) => return Err(VmError::TypeError(format!("cannot or {:?} and {:?}", a, b))),
                    }
                }
                OpCode::Not => {
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    match a {
                        Value::Bool(x) => self.stack.push(Value::Bool(!x)),
                        a => return Err(VmError::TypeError(format!("cannot not {:?}", a))),
                    }
                }
                OpCode::Concat => {
                    let b = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let a = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    match (a, b) {
                        (Value::String(x), Value::String(y)) => {
                            self.stack.push(Value::String(x + &y));
                        }
                        (a, b) => return Err(VmError::TypeError(format!("concat requires strings, got {:?} and {:?}", a, b))),
                    }
                }
                OpCode::Jump(target) => {
                    self.frames.last_mut().unwrap().ip = target;
                }
                OpCode::JumpIfFalse(target) => {
                    let val = self.stack.last().ok_or(VmError::StackUnderflow)?.clone();
                    if !is_truthy(&val) {
                        self.frames.last_mut().unwrap().ip = target;
                    }
                }
                OpCode::JumpIfTrue(target) => {
                    let val = self.stack.last().ok_or(VmError::StackUnderflow)?.clone();
                    if is_truthy(&val) {
                        self.frames.last_mut().unwrap().ip = target;
                    }
                }
                OpCode::Call(arg_count) => {
                    // Stack: [arg0, arg1, ..., argN-1, fn_value]
                    let fn_val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let target_chunk_idx = self.resolve_fn_value(&fn_val)?;
                    let mut args: Vec<Value> = (0..arg_count)
                        .map(|_| self.stack.pop().ok_or(VmError::StackUnderflow))
                        .collect::<Result<Vec<_>, _>>()?;
                    args.reverse();

                    let arity = self.chunks[target_chunk_idx].arity;
                    if args.len() != arity {
                        return Err(VmError::RuntimeError(format!(
                            "function '{}' expects {} args, got {}",
                            self.chunks[target_chunk_idx].name, arity, args.len()
                        )));
                    }

                    self.frames.push(CallFrame {
                        chunk_idx: target_chunk_idx,
                        ip: 0,
                        locals: args,
                        base: self.stack.len(),
                    });
                }
                OpCode::CallTool(arg_count) => {
                    let fn_val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let target_chunk_idx = self.resolve_fn_value(&fn_val)?;
                    let mut args: Vec<Value> = (0..arg_count)
                        .map(|_| self.stack.pop().ok_or(VmError::StackUnderflow))
                        .collect::<Result<Vec<_>, _>>()?;
                    args.reverse();

                    let tool_name = self.chunks[target_chunk_idx].name.clone();
                    let arg_strs: Vec<String> = args.iter().map(|v| format!("{v:?}")).collect();
                    self.tool_logger.log_call(&tool_name, &arg_strs);
                    let start = std::time::Instant::now();

                    // Tool stubs just return Unit (no body to execute)
                    self.stack.push(Value::Unit);

                    let duration_ms = start.elapsed().as_millis() as u64;
                    self.tool_logger.log_ok(&tool_name, duration_ms);
                    let _ = args;
                }
                OpCode::CallBuiltin(ref name, arg_count) => {
                    let mut args: Vec<Value> = (0..arg_count)
                        .map(|_| self.stack.pop().ok_or(VmError::StackUnderflow))
                        .collect::<Result<Vec<_>, _>>()?;
                    args.reverse();
                    let name = name.clone();
                    let result = self.call_stdlib_builtin(&name, args)?;
                    self.stack.push(result);
                }
                OpCode::CallMethod(ref method, arg_count) => {
                    // Stack: args pushed first, then target on top
                    let target = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let mut args: Vec<Value> = (0..arg_count)
                        .map(|_| self.stack.pop().ok_or(VmError::StackUnderflow))
                        .collect::<Result<Vec<_>, _>>()?;
                    args.reverse();
                    let method = method.clone();
                    let result = self.call_stdlib_method(target, &method, args)?;
                    self.stack.push(result);
                }
                OpCode::Return => {
                    let ret_val = self.stack.pop().unwrap_or(Value::Unit);
                    self.frames.pop();
                    self.stack.push(ret_val);
                }
                OpCode::MakeList(n) => {
                    let mut elems: Vec<Value> = (0..n)
                        .map(|_| self.stack.pop().ok_or(VmError::StackUnderflow))
                        .collect::<Result<Vec<_>, _>>()?;
                    elems.reverse();
                    self.stack.push(Value::List(elems));
                }
                OpCode::MakeMap(_n) => {
                    // Simplified: just push empty map
                    self.stack.push(Value::Map(HashMap::new()));
                }
                OpCode::GetField(ref field) => {
                    let target = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let val = match &target {
                        Value::Record { fields, .. } => {
                            fields.get(field).cloned().unwrap_or(Value::Unit)
                        }
                        Value::Map(m) => m.get(field).cloned().unwrap_or(Value::Unit),
                        _ => return Err(VmError::TypeError(format!("cannot get field on {:?}", target))),
                    };
                    self.stack.push(val);
                }
                OpCode::MakeSome => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    self.stack.push(Value::Variant { name: "Some".into(), payload: vec![val] });
                }
                OpCode::MakeOk => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    self.stack.push(Value::Variant { name: "Ok".into(), payload: vec![val] });
                }
                OpCode::MakeErr => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    self.stack.push(Value::Variant { name: "Err".into(), payload: vec![val] });
                }
                OpCode::Unwrap => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let inner = match val {
                        Value::Variant { name, payload } if name == "Some" || name == "Ok" => {
                            payload.into_iter().next().unwrap_or(Value::Unit)
                        }
                        other => return Err(VmError::RuntimeError(format!("unwrap failed on {:?}", other))),
                    };
                    self.stack.push(inner);
                }
                OpCode::IsOk => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let ok = matches!(&val, Value::Variant { name, .. } if name == "Ok");
                    self.stack.push(Value::Bool(ok));
                }
                OpCode::IsSome => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    let some = matches!(&val, Value::Variant { name, .. } if name == "Some");
                    self.stack.push(Value::Bool(some));
                }
                OpCode::Print => {
                    let val = self.stack.pop().ok_or(VmError::StackUnderflow)?;
                    println!("{}", display_value(&val));
                    self.stack.push(Value::Unit);
                }
                OpCode::Halt => {
                    self.frames.pop();
                    return Ok(self.stack.pop().unwrap_or(Value::Unit));
                }
            }
        }
    }

    fn resolve_fn_value(&self, val: &Value) -> Result<usize, VmError> {
        match val {
            Value::Int(idx) => Ok(*idx as usize),
            other => Err(VmError::TypeError(format!("expected function, got {:?}", other))),
        }
    }

    /// Call a function value (chunk index) with given args, returning the result.
    /// Used by stdlib builtins like List.map that need to call back into user functions.
    fn call_fn_value(&mut self, fn_val: Value, args: Vec<Value>) -> Result<Value, VmError> {
        let chunk_idx = self.resolve_fn_value(&fn_val)?;
        let arity = self.chunks[chunk_idx].arity;
        if args.len() != arity {
            return Err(VmError::RuntimeError(format!(
                "function '{}' expects {} args, got {}",
                self.chunks[chunk_idx].name, arity, args.len()
            )));
        }
        let target_depth = self.frames.len();
        self.frames.push(CallFrame {
            chunk_idx,
            ip: 0,
            locals: args,
            base: self.stack.len(),
        });
        self.execute_until(target_depth)
    }

    fn call_stdlib_builtin(&mut self, name: &str, args: Vec<Value>) -> Result<Value, VmError> {
        match name {
            "to_string" => {
                let out = args.first().map(display_value).unwrap_or_default();
                Ok(Value::String(out))
            }
            "len" => match args.first() {
                Some(Value::String(s)) => Ok(Value::Int(s.len() as i64)),
                Some(Value::List(l)) => Ok(Value::Int(l.len() as i64)),
                _ => Err(VmError::RuntimeError("len expects String or List".into())),
            },
            "type_of" => {
                let ty = match args.first() {
                    Some(Value::Int(_)) => "Int",
                    Some(Value::Float(_)) => "Float",
                    Some(Value::Bool(_)) => "Bool",
                    Some(Value::String(_)) => "String",
                    Some(Value::List(_)) => "List",
                    Some(Value::Tuple(_)) => "Tuple",
                    Some(Value::Record { .. }) => "Record",
                    Some(Value::Variant { .. }) => "Variant",
                    Some(Value::Map(_)) => "Map",
                    Some(Value::Closure { .. }) => "Closure",
                    Some(Value::Unit) | None => "Unit",
                };
                Ok(Value::String(ty.into()))
            }
            "assert" => match (args.first(), args.get(1)) {
                (Some(Value::Bool(true)), _) => Ok(Value::Unit),
                (Some(Value::Bool(false)), Some(Value::String(msg))) => {
                    Err(VmError::RuntimeError(format!("assertion failed: {msg}")))
                }
                (Some(Value::Bool(false)), _) => {
                    Err(VmError::RuntimeError("assertion failed".into()))
                }
                _ => Err(VmError::RuntimeError("assert expects (Bool, String)".into())),
            },
            "assert_eq" => {
                let actual = args.first().ok_or_else(|| VmError::RuntimeError("assert_eq: missing actual".into()))?;
                let expected = args.get(1).ok_or_else(|| VmError::RuntimeError("assert_eq: missing expected".into()))?;
                if actual == expected {
                    Ok(Value::Unit)
                } else {
                    let msg = match args.get(2) {
                        Some(Value::String(s)) => format!("assertion failed: expected {}, got {}: {s}", display_value(expected), display_value(actual)),
                        _ => format!("assertion failed: expected {}, got {}", display_value(expected), display_value(actual)),
                    };
                    Err(VmError::RuntimeError(msg))
                }
            }
            "assert_err" => {
                let val = args.first().ok_or_else(|| VmError::RuntimeError("assert_err: missing value".into()))?;
                match val {
                    Value::Variant { name, .. } if name == "Err" => Ok(Value::Unit),
                    _ => {
                        let msg = match args.get(1) {
                            Some(Value::String(s)) => format!("assertion failed: expected Err(_): {s}"),
                            _ => "assertion failed: expected Err(_)".into(),
                        };
                        Err(VmError::RuntimeError(msg))
                    }
                }
            }
            "parse_int" => match args.first() {
                Some(Value::String(s)) => match s.parse::<i64>() {
                    Ok(n) => Ok(Value::Variant { name: "Ok".into(), payload: vec![Value::Int(n)] }),
                    Err(_) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("cannot parse {:?} as Int", s))] }),
                },
                _ => Err(VmError::RuntimeError("parse_int expects String".into())),
            },
            "parse_float" => match args.first() {
                Some(Value::String(s)) => match s.parse::<f64>() {
                    Ok(f) => Ok(Value::Variant { name: "Ok".into(), payload: vec![Value::Float(f)] }),
                    Err(_) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(format!("cannot parse {:?} as Float", s))] }),
                },
                _ => Err(VmError::RuntimeError("parse_float expects String".into())),
            },
            "int_to_float" => match args.first() {
                Some(Value::Int(n)) => Ok(Value::Float(*n as f64)),
                _ => Err(VmError::RuntimeError("int_to_float expects Int".into())),
            },
            "float_to_int" => match args.first() {
                Some(Value::Float(f)) => Ok(Value::Int(*f as i64)),
                _ => Err(VmError::RuntimeError("float_to_int expects Float".into())),
            },

            // List module
            "List.length" => match args.first() {
                Some(Value::List(items)) => Ok(Value::Int(items.len() as i64)),
                _ => Err(VmError::RuntimeError("List.length expects a List".into())),
            },
            "List.range" => match (args.first(), args.get(1)) {
                (Some(Value::Int(start)), Some(Value::Int(end))) => {
                    let mut out = Vec::new();
                    if start <= end {
                        for i in *start..*end {
                            out.push(Value::Int(i));
                        }
                    }
                    Ok(Value::List(out))
                }
                _ => Err(VmError::RuntimeError("List.range expects (Int, Int)".into())),
            },
            "List.map" => {
                let (list, fn_val) = match (args.first().cloned(), args.get(1).cloned()) {
                    (Some(l), Some(f)) => (l, f),
                    _ => return Err(VmError::RuntimeError("List.map expects (List, fn)".into())),
                };
                if let Value::List(items) = list {
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        let result = self.call_fn_value(fn_val.clone(), vec![item])?;
                        out.push(result);
                    }
                    Ok(Value::List(out))
                } else {
                    Err(VmError::RuntimeError("List.map: first arg must be List".into()))
                }
            }
            "List.filter" => {
                let (list, fn_val) = match (args.first().cloned(), args.get(1).cloned()) {
                    (Some(l), Some(f)) => (l, f),
                    _ => return Err(VmError::RuntimeError("List.filter expects (List, fn)".into())),
                };
                if let Value::List(items) = list {
                    let mut out = Vec::new();
                    for item in items {
                        let keep = self.call_fn_value(fn_val.clone(), vec![item.clone()])?;
                        if matches!(keep, Value::Bool(true)) {
                            out.push(item);
                        }
                    }
                    Ok(Value::List(out))
                } else {
                    Err(VmError::RuntimeError("List.filter: first arg must be List".into()))
                }
            }
            "List.fold" => {
                let (list, init, fn_val) = match (args.first().cloned(), args.get(1).cloned(), args.get(2).cloned()) {
                    (Some(l), Some(i), Some(f)) => (l, i, f),
                    _ => return Err(VmError::RuntimeError("List.fold expects (List, init, fn)".into())),
                };
                if let Value::List(items) = list {
                    let mut acc = init;
                    for item in items {
                        acc = self.call_fn_value(fn_val.clone(), vec![acc, item])?;
                    }
                    Ok(acc)
                } else {
                    Err(VmError::RuntimeError("List.fold: first arg must be List".into()))
                }
            }
            "List.flat_map" => {
                let (list, fn_val) = match (args.first().cloned(), args.get(1).cloned()) {
                    (Some(l), Some(f)) => (l, f),
                    _ => return Err(VmError::RuntimeError("List.flat_map expects (List, fn)".into())),
                };
                if let Value::List(items) = list {
                    let mut out = Vec::new();
                    for item in items {
                        match self.call_fn_value(fn_val.clone(), vec![item])? {
                            Value::List(inner) => out.extend(inner),
                            other => out.push(other),
                        }
                    }
                    Ok(Value::List(out))
                } else {
                    Err(VmError::RuntimeError("List.flat_map: first arg must be List".into()))
                }
            }
            "List.zip" => match (args.first(), args.get(1)) {
                (Some(Value::List(a)), Some(Value::List(b))) => {
                    let out = a.iter().zip(b.iter())
                        .map(|(x, y)| Value::Tuple(vec![x.clone(), y.clone()]))
                        .collect();
                    Ok(Value::List(out))
                }
                _ => Err(VmError::RuntimeError("List.zip expects (List, List)".into())),
            },
            "List.sort" => match args.first().cloned() {
                Some(Value::List(mut items)) => {
                    items.sort_by(|a, b| compare_values(a, b));
                    Ok(Value::List(items))
                }
                _ => Err(VmError::RuntimeError("List.sort expects a List".into())),
            },
            "List.contains" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(v)) => Ok(Value::Bool(items.contains(v))),
                _ => Err(VmError::RuntimeError("List.contains expects (List, value)".into())),
            },
            "List.sum" => match args.first() {
                Some(Value::List(items)) => {
                    let mut int_sum: i64 = 0;
                    let mut float_sum: f64 = 0.0;
                    let mut is_float = false;
                    for item in items {
                        match item {
                            Value::Int(n) => int_sum += n,
                            Value::Float(f) => { float_sum += f; is_float = true; }
                            _ => return Err(VmError::RuntimeError("List.sum: non-numeric element".into())),
                        }
                    }
                    if is_float {
                        Ok(Value::Float(int_sum as f64 + float_sum))
                    } else {
                        Ok(Value::Int(int_sum))
                    }
                }
                _ => Err(VmError::RuntimeError("List.sum expects a List".into())),
            },
            "List.min" => match args.first() {
                Some(Value::List(items)) if !items.is_empty() => {
                    let mut min = items[0].clone();
                    for item in &items[1..] {
                        if compare_values(item, &min) == std::cmp::Ordering::Less {
                            min = item.clone();
                        }
                    }
                    Ok(Value::Variant { name: "Some".into(), payload: vec![min] })
                }
                Some(Value::List(_)) => Ok(Value::Variant { name: "None".into(), payload: vec![] }),
                _ => Err(VmError::RuntimeError("List.min expects a List".into())),
            },
            "List.max" => match args.first() {
                Some(Value::List(items)) if !items.is_empty() => {
                    let mut max = items[0].clone();
                    for item in &items[1..] {
                        if compare_values(item, &max) == std::cmp::Ordering::Greater {
                            max = item.clone();
                        }
                    }
                    Ok(Value::Variant { name: "Some".into(), payload: vec![max] })
                }
                Some(Value::List(_)) => Ok(Value::Variant { name: "None".into(), payload: vec![] }),
                _ => Err(VmError::RuntimeError("List.max expects a List".into())),
            },
            "List.push" => match (args.first().cloned(), args.get(1).cloned()) {
                (Some(Value::List(mut items)), Some(v)) => { items.push(v); Ok(Value::List(items)) }
                _ => Err(VmError::RuntimeError("List.push expects (List, value)".into())),
            },
            "List.pop" => match args.first().cloned() {
                Some(Value::List(mut items)) => {
                    let last = items.pop().map(|v| Value::Variant { name: "Some".into(), payload: vec![v] })
                        .unwrap_or(Value::Variant { name: "None".into(), payload: vec![] });
                    Ok(Value::Tuple(vec![Value::List(items), last]))
                }
                _ => Err(VmError::RuntimeError("List.pop expects a List".into())),
            },
            "List.head" => match args.first() {
                Some(Value::List(items)) => {
                    Ok(items.first().map(|v| Value::Variant { name: "Some".into(), payload: vec![v.clone()] })
                        .unwrap_or(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(VmError::RuntimeError("List.head expects a List".into())),
            },
            "List.tail" => match args.first() {
                Some(Value::List(items)) if !items.is_empty() => {
                    Ok(Value::List(items[1..].to_vec()))
                }
                Some(Value::List(_)) => Ok(Value::List(vec![])),
                _ => Err(VmError::RuntimeError("List.tail expects a List".into())),
            },
            "List.concat" => match (args.first(), args.get(1)) {
                (Some(Value::List(a)), Some(Value::List(b))) => {
                    let mut out = a.clone();
                    out.extend(b.iter().cloned());
                    Ok(Value::List(out))
                }
                _ => Err(VmError::RuntimeError("List.concat expects (List, List)".into())),
            },
            "List.reverse" => match args.first().cloned() {
                Some(Value::List(mut items)) => { items.reverse(); Ok(Value::List(items)) }
                _ => Err(VmError::RuntimeError("List.reverse expects a List".into())),
            },
            "List.unique" => match args.first() {
                Some(Value::List(items)) => {
                    let mut seen = Vec::new();
                    for item in items {
                        if !seen.contains(item) {
                            seen.push(item.clone());
                        }
                    }
                    Ok(Value::List(seen))
                }
                _ => Err(VmError::RuntimeError("List.unique expects a List".into())),
            },
            "List.flatten" => match args.first() {
                Some(Value::List(items)) => {
                    let mut out = Vec::new();
                    for item in items {
                        match item {
                            Value::List(inner) => out.extend(inner.iter().cloned()),
                            other => out.push(other.clone()),
                        }
                    }
                    Ok(Value::List(out))
                }
                _ => Err(VmError::RuntimeError("List.flatten expects a List".into())),
            },
            "List.enumerate" => match args.first() {
                Some(Value::List(items)) => {
                    let out = items.iter().enumerate()
                        .map(|(i, v)| Value::Tuple(vec![Value::Int(i as i64), v.clone()]))
                        .collect();
                    Ok(Value::List(out))
                }
                _ => Err(VmError::RuntimeError("List.enumerate expects a List".into())),
            },
            "List.slice" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::List(items)), Some(Value::Int(from)), Some(Value::Int(to))) => {
                    let from = (*from as usize).min(items.len());
                    let to = (*to as usize).min(items.len());
                    Ok(Value::List(items[from..to].to_vec()))
                }
                _ => Err(VmError::RuntimeError("List.slice expects (List, Int, Int)".into())),
            },
            "List.index" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::Int(idx))) => {
                    Ok(items.get(*idx as usize).cloned()
                        .map(|v| Value::Variant { name: "Some".into(), payload: vec![v] })
                        .unwrap_or(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(VmError::RuntimeError("List.index expects (List, Int)".into())),
            },

            // Json module
            "Json.parse" => match args.first() {
                Some(Value::String(s)) => {
                    match serde_json::from_str::<serde_json::Value>(s) {
                        Ok(jv) => Ok(Value::Variant { name: "Ok".into(), payload: vec![json_to_value(jv)] }),
                        Err(e) => Ok(Value::Variant { name: "Err".into(), payload: vec![Value::String(e.to_string())] }),
                    }
                }
                _ => Err(VmError::RuntimeError("Json.parse expects String".into())),
            },
            "Json.stringify" => {
                let v = args.first().cloned().unwrap_or(Value::Unit);
                Ok(Value::String(value_to_json_string(&v)))
            }
            "Json.get" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(key))) => {
                    Ok(m.get(key).cloned()
                        .map(|v| Value::Variant { name: "Some".into(), payload: vec![v] })
                        .unwrap_or(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(VmError::RuntimeError("Json.get expects (Map, String)".into())),
            },
            "Json.keys" => match args.first() {
                Some(Value::Map(m)) => {
                    Ok(Value::List(m.keys().map(|k| Value::String(k.clone())).collect()))
                }
                _ => Err(VmError::RuntimeError("Json.keys expects Map".into())),
            },
            "Json.index" => match (args.first(), args.get(1)) {
                (Some(Value::List(items)), Some(Value::Int(idx))) => {
                    Ok(items.get(*idx as usize).cloned()
                        .map(|v| Value::Variant { name: "Some".into(), payload: vec![v] })
                        .unwrap_or(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(VmError::RuntimeError("Json.index expects (List, Int)".into())),
            },

            // String module
            "String.len" => match args.first() {
                Some(Value::String(s)) => Ok(Value::Int(s.len() as i64)),
                _ => Err(VmError::RuntimeError("String.len expects String".into())),
            },
            "String.trim" => match args.first() {
                Some(Value::String(s)) => Ok(Value::String(s.trim().to_string())),
                _ => Err(VmError::RuntimeError("String.trim expects String".into())),
            },
            "String.split" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(delim))) => {
                    Ok(Value::List(s.split(delim.as_str()).map(|x| Value::String(x.into())).collect()))
                }
                _ => Err(VmError::RuntimeError("String.split expects (String, String)".into())),
            },
            "String.contains" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(needle))) => Ok(Value::Bool(s.contains(needle.as_str()))),
                _ => Err(VmError::RuntimeError("String.contains expects (String, String)".into())),
            },
            "String.starts_with" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(prefix))) => Ok(Value::Bool(s.starts_with(prefix.as_str()))),
                _ => Err(VmError::RuntimeError("String.starts_with expects (String, String)".into())),
            },
            "String.ends_with" => match (args.first(), args.get(1)) {
                (Some(Value::String(s)), Some(Value::String(suffix))) => Ok(Value::Bool(s.ends_with(suffix.as_str()))),
                _ => Err(VmError::RuntimeError("String.ends_with expects (String, String)".into())),
            },
            "String.to_upper" => match args.first() {
                Some(Value::String(s)) => Ok(Value::String(s.to_uppercase())),
                _ => Err(VmError::RuntimeError("String.to_upper expects String".into())),
            },
            "String.to_lower" => match args.first() {
                Some(Value::String(s)) => Ok(Value::String(s.to_lowercase())),
                _ => Err(VmError::RuntimeError("String.to_lower expects String".into())),
            },
            "String.replace" => match (args.first(), args.get(1), args.get(2)) {
                (Some(Value::String(s)), Some(Value::String(from)), Some(Value::String(to))) => {
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err(VmError::RuntimeError("String.replace expects (String, String, String)".into())),
            },

            // Math module
            "Math.abs" => match args.first() {
                Some(Value::Int(n)) => Ok(Value::Int(n.abs())),
                Some(Value::Float(f)) => Ok(Value::Float(f.abs())),
                _ => Err(VmError::RuntimeError("Math.abs expects Int or Float".into())),
            },
            "Math.sqrt" => match args.first() {
                Some(Value::Float(f)) => Ok(Value::Float(f.sqrt())),
                Some(Value::Int(n)) => Ok(Value::Float((*n as f64).sqrt())),
                _ => Err(VmError::RuntimeError("Math.sqrt expects number".into())),
            },
            "Math.floor" => match args.first() {
                Some(Value::Float(f)) => Ok(Value::Int(f.floor() as i64)),
                Some(Value::Int(n)) => Ok(Value::Int(*n)),
                _ => Err(VmError::RuntimeError("Math.floor expects number".into())),
            },
            "Math.ceil" => match args.first() {
                Some(Value::Float(f)) => Ok(Value::Int(f.ceil() as i64)),
                Some(Value::Int(n)) => Ok(Value::Int(*n)),
                _ => Err(VmError::RuntimeError("Math.ceil expects number".into())),
            },
            "Math.round" => match args.first() {
                Some(Value::Float(f)) => Ok(Value::Int(f.round() as i64)),
                Some(Value::Int(n)) => Ok(Value::Int(*n)),
                _ => Err(VmError::RuntimeError("Math.round expects number".into())),
            },
            "Math.pow" => match (args.first(), args.get(1)) {
                (Some(Value::Float(base)), Some(Value::Float(exp))) => Ok(Value::Float(base.powf(*exp))),
                (Some(Value::Int(base)), Some(Value::Int(exp))) => Ok(Value::Int(base.pow(*exp as u32))),
                (Some(Value::Float(base)), Some(Value::Int(exp))) => Ok(Value::Float(base.powi(*exp as i32))),
                (Some(Value::Int(base)), Some(Value::Float(exp))) => Ok(Value::Float((*base as f64).powf(*exp))),
                _ => Err(VmError::RuntimeError("Math.pow expects two numbers".into())),
            },
            "Math.min" => match (args.first(), args.get(1)) {
                (Some(Value::Int(a)), Some(Value::Int(b))) => Ok(Value::Int(*a.min(b))),
                (Some(Value::Float(a)), Some(Value::Float(b))) => Ok(Value::Float(a.min(*b))),
                _ => Err(VmError::RuntimeError("Math.min expects two numbers".into())),
            },
            "Math.max" => match (args.first(), args.get(1)) {
                (Some(Value::Int(a)), Some(Value::Int(b))) => Ok(Value::Int(*a.max(b))),
                (Some(Value::Float(a)), Some(Value::Float(b))) => Ok(Value::Float(a.max(*b))),
                _ => Err(VmError::RuntimeError("Math.max expects two numbers".into())),
            },

            // Map module
            "Map.new" => Ok(Value::Map(HashMap::new())),
            "Map.insert" => match (args.first().cloned(), args.get(1).cloned(), args.get(2).cloned()) {
                (Some(Value::Map(mut m)), Some(Value::String(k)), Some(v)) => { m.insert(k, v); Ok(Value::Map(m)) }
                _ => Err(VmError::RuntimeError("Map.insert expects (Map, String, value)".into())),
            },
            "Map.get" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(k))) => {
                    Ok(m.get(k).cloned()
                        .map(|v| Value::Variant { name: "Some".into(), payload: vec![v] })
                        .unwrap_or(Value::Variant { name: "None".into(), payload: vec![] }))
                }
                _ => Err(VmError::RuntimeError("Map.get expects (Map, String)".into())),
            },
            "Map.keys" => match args.first() {
                Some(Value::Map(m)) => Ok(Value::List(m.keys().map(|k| Value::String(k.clone())).collect())),
                _ => Err(VmError::RuntimeError("Map.keys expects Map".into())),
            },
            "Map.values" => match args.first() {
                Some(Value::Map(m)) => Ok(Value::List(m.values().cloned().collect())),
                _ => Err(VmError::RuntimeError("Map.values expects Map".into())),
            },
            "Map.contains_key" => match (args.first(), args.get(1)) {
                (Some(Value::Map(m)), Some(Value::String(k))) => Ok(Value::Bool(m.contains_key(k))),
                _ => Err(VmError::RuntimeError("Map.contains_key expects (Map, String)".into())),
            },
            "Map.remove" => match (args.first().cloned(), args.get(1)) {
                (Some(Value::Map(mut m)), Some(Value::String(k))) => { m.remove(k); Ok(Value::Map(m)) }
                _ => Err(VmError::RuntimeError("Map.remove expects (Map, String)".into())),
            },

            _ => Err(VmError::RuntimeError(format!("unknown stdlib builtin: {name}"))),
        }
    }

    fn call_stdlib_method(&mut self, target: Value, method: &str, args: Vec<Value>) -> Result<Value, VmError> {
        match method {
            "to_string" => Ok(Value::String(display_value(&target))),
            "len" => match target {
                Value::String(s) => Ok(Value::Int(s.len() as i64)),
                Value::List(l) => Ok(Value::Int(l.len() as i64)),
                _ => Err(VmError::RuntimeError(format!("len not supported on {:?}", target))),
            },
            "trim" => match target {
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                _ => Err(VmError::RuntimeError("trim expects String".into())),
            },
            "split" => match target {
                Value::String(s) => {
                    let delim = match args.first() {
                        Some(Value::String(d)) => d.clone(),
                        _ => return Err(VmError::RuntimeError("split expects String delimiter".into())),
                    };
                    Ok(Value::List(s.split(delim.as_str()).map(|x| Value::String(x.into())).collect()))
                }
                _ => Err(VmError::RuntimeError("split expects String".into())),
            },
            "contains" => match target {
                Value::String(s) => {
                    let needle = match args.first() {
                        Some(Value::String(n)) => n.clone(),
                        _ => return Err(VmError::RuntimeError("contains expects String needle".into())),
                    };
                    Ok(Value::Bool(s.contains(needle.as_str())))
                }
                Value::List(items) => {
                    let needle = args.first().ok_or_else(|| VmError::RuntimeError("contains expects a value".into()))?;
                    Ok(Value::Bool(items.contains(needle)))
                }
                _ => Err(VmError::RuntimeError("contains not supported on this type".into())),
            },
            "starts_with" => match target {
                Value::String(s) => {
                    let prefix = match args.first() { Some(Value::String(p)) => p.clone(), _ => return Err(VmError::RuntimeError("starts_with expects String".into())) };
                    Ok(Value::Bool(s.starts_with(prefix.as_str())))
                }
                _ => Err(VmError::RuntimeError("starts_with expects String".into())),
            },
            "ends_with" => match target {
                Value::String(s) => {
                    let suffix = match args.first() { Some(Value::String(p)) => p.clone(), _ => return Err(VmError::RuntimeError("ends_with expects String".into())) };
                    Ok(Value::Bool(s.ends_with(suffix.as_str())))
                }
                _ => Err(VmError::RuntimeError("ends_with expects String".into())),
            },
            "to_upper" => match target {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(VmError::RuntimeError("to_upper expects String".into())),
            },
            "to_lower" => match target {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(VmError::RuntimeError("to_lower expects String".into())),
            },
            "replace" => match target {
                Value::String(s) => {
                    let from = match args.first() { Some(Value::String(f)) => f.clone(), _ => return Err(VmError::RuntimeError("replace expects (String, String)".into())) };
                    let to = match args.get(1) { Some(Value::String(t)) => t.clone(), _ => return Err(VmError::RuntimeError("replace expects (String, String)".into())) };
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err(VmError::RuntimeError("replace expects String".into())),
            },
            "push" => match target {
                Value::List(mut items) => {
                    let v = args.into_iter().next().unwrap_or(Value::Unit);
                    items.push(v);
                    Ok(Value::List(items))
                }
                _ => Err(VmError::RuntimeError("push expects List".into())),
            },
            "map" => match target {
                Value::List(items) => {
                    let fn_val = args.into_iter().next().ok_or_else(|| VmError::RuntimeError("map expects a function".into()))?;
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        out.push(self.call_fn_value(fn_val.clone(), vec![item])?);
                    }
                    Ok(Value::List(out))
                }
                Value::Variant { name, payload } if name == "Some" && payload.len() == 1 => {
                    let fn_val = args.into_iter().next().ok_or_else(|| VmError::RuntimeError("map expects a function".into()))?;
                    let mapped = self.call_fn_value(fn_val, vec![payload[0].clone()])?;
                    Ok(Value::Variant { name: "Some".into(), payload: vec![mapped] })
                }
                other @ Value::Variant { .. } => Ok(other),
                _ => Err(VmError::RuntimeError("map expects List or Option/Result".into())),
            },
            "filter" => match target {
                Value::List(items) => {
                    let fn_val = args.into_iter().next().ok_or_else(|| VmError::RuntimeError("filter expects a function".into()))?;
                    let mut out = Vec::new();
                    for item in items {
                        if matches!(self.call_fn_value(fn_val.clone(), vec![item.clone()])?, Value::Bool(true)) {
                            out.push(item);
                        }
                    }
                    Ok(Value::List(out))
                }
                _ => Err(VmError::RuntimeError("filter expects List".into())),
            },
            "is_some" => Ok(Value::Bool(matches!(target, Value::Variant { ref name, .. } if name == "Some"))),
            "is_none" => Ok(Value::Bool(matches!(target, Value::Variant { ref name, .. } if name == "None"))),
            "is_ok" => Ok(Value::Bool(matches!(target, Value::Variant { ref name, .. } if name == "Ok"))),
            "is_err" => Ok(Value::Bool(matches!(target, Value::Variant { ref name, .. } if name == "Err"))),
            "unwrap" => match target {
                Value::Variant { name, payload } if (name == "Some" || name == "Ok") && !payload.is_empty() => Ok(payload[0].clone()),
                Value::Variant { name, .. } if name == "None" => Err(VmError::RuntimeError("unwrap called on None".into())),
                Value::Variant { name, payload } if name == "Err" && !payload.is_empty() => Err(VmError::RuntimeError(format!("unwrap called on Err({})", display_value(&payload[0])))),
                _ => Err(VmError::RuntimeError("unwrap expects Some or Ok".into())),
            },
            "unwrap_or" => match target {
                Value::Variant { name, payload } if (name == "Some" || name == "Ok") && !payload.is_empty() => Ok(payload[0].clone()),
                Value::Variant { name, .. } if name == "None" || name == "Err" => Ok(args.into_iter().next().unwrap_or(Value::Unit)),
                _ => Err(VmError::RuntimeError("unwrap_or expects Option or Result".into())),
            },
            _ => Err(VmError::RuntimeError(format!("unknown method: {method} on {:?}", target))),
        }
    }
}

fn is_truthy(val: &Value) -> bool {
    match val {
        Value::Bool(b) => *b,
        Value::Unit => false,
        Value::Int(0) => false,
        Value::Int(_) => true,
        _ => true,
    }
}

fn display_value(val: &Value) -> String {
    match val {
        Value::Unit => "()".into(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::List(elems) => {
            let parts: Vec<_> = elems.iter().map(display_value).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Tuple(elems) => {
            let parts: Vec<_> = elems.iter().map(display_value).collect();
            format!("({})", parts.join(", "))
        }
        Value::Record { name, fields } => {
            let parts: Vec<_> = fields.iter().map(|(k, v)| format!("{}: {}", k, display_value(v))).collect();
            format!("{} {{ {} }}", name, parts.join(", "))
        }
        Value::Variant { name, payload } => {
            if payload.is_empty() {
                name.clone()
            } else {
                let parts: Vec<_> = payload.iter().map(display_value).collect();
                format!("{}({})", name, parts.join(", "))
            }
        }
        Value::Map(m) => {
            let parts: Vec<_> = m.iter().map(|(k, v)| format!("{}: {}", k, display_value(v))).collect();
            format!("{{{}}}", parts.join(", "))
        }
        Value::Closure { .. } => "<closure>".into(),
    }
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)).unwrap_or(std::cmp::Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => std::cmp::Ordering::Equal,
    }
}

fn json_to_value(jv: serde_json::Value) -> Value {
    match jv {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => Value::List(arr.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            let mut m = HashMap::new();
            for (k, v) in obj {
                m.insert(k, json_to_value(v));
            }
            Value::Map(m)
        }
    }
}

fn value_to_json_string(v: &Value) -> String {
    match v {
        Value::Unit => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_else(|_| format!("{:?}", s)),
        Value::List(items) => {
            let parts: Vec<_> = items.iter().map(value_to_json_string).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Map(m) => {
            let parts: Vec<_> = m.iter().map(|(k, v)| {
                format!("{}:{}", serde_json::to_string(k).unwrap_or_else(|_| format!("{:?}", k)), value_to_json_string(v))
            }).collect();
            format!("{{{}}}", parts.join(","))
        }
        other => format!("{:?}", other),
    }
}
