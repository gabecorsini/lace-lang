use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use lace_interp::Value;

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
    tool_log: bool,
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
        Vm { stack: Vec::new(), frames: Vec::new(), globals, chunks, tool_log }
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
            self.execute()
        } else {
            Ok(Value::Unit)
        }
    }

    fn execute(&mut self) -> Result<Value, VmError> {
        loop {
            if self.frames.is_empty() {
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
                    let ts_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64;
                    if self.tool_log {
                        eprintln!(r#"{{"event":"tool_call","tool":"{}","timestamp":{}}}"#, tool_name, ts_ms);
                    }
                    let start = std::time::Instant::now();

                    // Tool stubs just return Unit (no body to execute)
                    self.stack.push(Value::Unit);

                    let duration_ms = start.elapsed().as_millis() as u64;
                    if self.tool_log {
                        eprintln!(r#"{{"event":"tool_ok","tool":"{}","duration_ms":{}}}"#, tool_name, duration_ms);
                    }
                    let _ = args;
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
