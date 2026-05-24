use lace_ast::*;
use lace_interp::Value;
use lace_parser::parse_program;

use crate::chunk::Chunk;
use crate::error::VmError;
use crate::opcode::OpCode;

struct Compiler {
    #[allow(dead_code)]
    chunks: Vec<Chunk>,
    /// Names of tool functions (for CallTool vs Call distinction).
    tool_names: std::collections::HashSet<String>,
    /// Current local variable scope: name → slot index.
    locals: Vec<String>,
    /// Depth of local scope (0 = global).
    scope_depth: usize,
}

impl Compiler {
    fn new() -> Self {
        Compiler {
            chunks: Vec::new(),
            tool_names: std::collections::HashSet::new(),
            locals: Vec::new(),
            scope_depth: 0,
        }
    }

    fn resolve_local(&self, name: &str) -> Option<usize> {
        self.locals.iter().rposition(|n| n == name)
    }

    fn compile_block(&mut self, block: &Block, chunk: &mut Chunk) -> Result<(), VmError> {
        let before = self.locals.len();
        for stmt in &block.stmts {
            self.compile_stmt(stmt, chunk)?;
        }
        if let Some(tail) = &block.tail_expr {
            self.compile_expr(tail, chunk)?;
        } else {
            // Push Unit as the block's value
            let idx = chunk.add_const(Value::Unit);
            chunk.emit(OpCode::LoadConst(idx));
        }
        // Pop locals added inside this block (but NOT the tail value on top)
        let added = self.locals.len() - before;
        // We need to swap: pop locals but keep top value.
        // Simple approach: emit Pop for each local *before* the tail value.
        // But since stmts are all popped in compile_stmt, locals from Let
        // remain on stack only as StoreLocal entries (the value stays in locals vec).
        // Actually with our StoreLocal design the value is stored in locals[] not on stack.
        // So no extra pops needed here — just trim locals.
        for _ in 0..added {
            self.locals.pop();
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt, chunk: &mut Chunk) -> Result<(), VmError> {
        match stmt {
            Stmt::Let(let_stmt) | Stmt::MutLet(let_stmt) => {
                self.compile_expr(&let_stmt.expr, chunk)?;
                if self.scope_depth > 0 {
                    // Allocate a slot for this local
                    let slot = self.locals.len();
                    self.locals.push(let_stmt.name.clone());
                    chunk.emit(OpCode::StoreLocal(slot));
                } else {
                    chunk.emit(OpCode::StoreGlobal(let_stmt.name.clone()));
                }
            }
            Stmt::Assign(assign) => {
                self.compile_expr(&assign.expr, chunk)?;
                if let Some(slot) = self.resolve_local(&assign.name) {
                    chunk.emit(OpCode::StoreLocal(slot));
                } else {
                    chunk.emit(OpCode::StoreGlobal(assign.name.clone()));
                }
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr, chunk)?;
                chunk.emit(OpCode::Pop);
            }
            Stmt::For(for_stmt) => {
                self.compile_for(for_stmt, chunk)?;
            }
            Stmt::While(while_stmt) => {
                self.compile_while(while_stmt, chunk)?;
            }
            Stmt::PureBlock(_) => {
                // Not implemented — no-op
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr, chunk: &mut Chunk) -> Result<(), VmError> {
        match expr {
            Expr::Literal(lit, _) => {
                let val = match lit {
                    Literal::Int(n) => Value::Int(*n),
                    Literal::Float(s) => {
                        let f: f64 = s.parse().unwrap_or(0.0);
                        Value::Float(f)
                    }
                    Literal::String(s) => Value::String(s.clone()),
                    Literal::Bool(b) => Value::Bool(*b),
                };
                let idx = chunk.add_const(val);
                chunk.emit(OpCode::LoadConst(idx));
            }

            Expr::Ident(name, _) => {
                if let Some(slot) = self.resolve_local(name) {
                    chunk.emit(OpCode::LoadLocal(slot));
                } else {
                    chunk.emit(OpCode::LoadGlobal(name.clone()));
                }
            }

            Expr::Binary { left, op, right, .. } => {
                self.compile_expr(left, chunk)?;
                self.compile_expr(right, chunk)?;
                let opcode = match op {
                    BinaryOp::Add => OpCode::Add,
                    BinaryOp::Sub => OpCode::Sub,
                    BinaryOp::Mul => OpCode::Mul,
                    BinaryOp::Div => OpCode::Div,
                    BinaryOp::IntDiv => OpCode::IntDiv,
                    BinaryOp::Rem => OpCode::Mod,
                    BinaryOp::Eq => OpCode::Eq,
                    BinaryOp::Ne => OpCode::Ne,
                    BinaryOp::Lt => OpCode::Lt,
                    BinaryOp::Le => OpCode::Le,
                    BinaryOp::Gt => OpCode::Gt,
                    BinaryOp::Ge => OpCode::Ge,
                    BinaryOp::And => OpCode::And,
                    BinaryOp::Or => OpCode::Or,
                    BinaryOp::Concat => OpCode::Concat,
                };
                chunk.emit(opcode);
            }

            Expr::Unary { op, expr, .. } => {
                self.compile_expr(expr, chunk)?;
                match op {
                    UnaryOp::Neg => chunk.emit(OpCode::Neg),
                    UnaryOp::Not => chunk.emit(OpCode::Not),
                };
            }

            Expr::If(if_expr) => {
                self.compile_if(if_expr, chunk)?;
            }

            Expr::Match(match_expr) => {
                self.compile_match(match_expr, chunk)?;
            }

            Expr::Block(block) => {
                let prev_depth = self.scope_depth;
                self.scope_depth += 1;
                self.compile_block(block, chunk)?;
                self.scope_depth = prev_depth;
            }

            Expr::FnCall(call) => {
                if call.name == "print" || call.name == "println" {
                    // compile first arg (or Unit)
                    if let Some(arg) = call.args.first() {
                        self.compile_expr(arg, chunk)?;
                    } else {
                        let idx = chunk.add_const(Value::Unit);
                        chunk.emit(OpCode::LoadConst(idx));
                    }
                    chunk.emit(OpCode::Print);
                } else {
                    match call.name.as_str() {
                        "Some" => {
                            let arg = call.args.first().ok_or_else(|| VmError::CompileError("Some expects 1 arg".into()))?;
                            self.compile_expr(arg, chunk)?;
                            chunk.emit(OpCode::MakeSome);
                        }
                        "Ok" => {
                            let arg = call.args.first().ok_or_else(|| VmError::CompileError("Ok expects 1 arg".into()))?;
                            self.compile_expr(arg, chunk)?;
                            chunk.emit(OpCode::MakeOk);
                        }
                        "Err" => {
                            let arg = call.args.first().ok_or_else(|| VmError::CompileError("Err expects 1 arg".into()))?;
                            self.compile_expr(arg, chunk)?;
                            chunk.emit(OpCode::MakeErr);
                        }
                        "None" => {
                            let idx = chunk.add_const(Value::Variant { name: "None".into(), payload: vec![] });
                            chunk.emit(OpCode::LoadConst(idx));
                        }
                        _ if is_free_builtin(&call.name) => {
                            let n = call.args.len();
                            for arg in &call.args {
                                self.compile_expr(arg, chunk)?;
                            }
                            chunk.emit(OpCode::CallBuiltin(call.name.clone(), n));
                        }
                        _ => {
                            let n = call.args.len();
                            for arg in &call.args {
                                self.compile_expr(arg, chunk)?;
                            }
                            chunk.emit(OpCode::LoadGlobal(call.name.clone()));
                            if self.tool_names.contains(&call.name) {
                                chunk.emit(OpCode::CallTool(n));
                            } else {
                                chunk.emit(OpCode::Call(n));
                            }
                        }
                    }
                }
            }

            Expr::Return { value, .. } => {
                if let Some(val) = value {
                    self.compile_expr(val, chunk)?;
                } else {
                    let idx = chunk.add_const(Value::Unit);
                    chunk.emit(OpCode::LoadConst(idx));
                }
                chunk.emit(OpCode::Return);
            }

            Expr::ListLiteral { elems, .. } => {
                let n = elems.len();
                for elem in elems {
                    self.compile_expr(elem, chunk)?;
                }
                chunk.emit(OpCode::MakeList(n));
            }

            Expr::FieldAccess { target, field, .. } => {
                self.compile_expr(target, chunk)?;
                chunk.emit(OpCode::GetField(field.clone()));
            }

            Expr::MethodCall(call) => {
                // Check if target is a module identifier (List, Json, etc.)
                if let Expr::Ident(module_name, _) = call.target.as_ref() {
                    if is_stdlib_module(module_name) {
                        // e.g. List.range(0, 5) — compile as CallBuiltin("List.range", n)
                        let qualified = format!("{}.{}", module_name, call.method);
                        let n = call.args.len();
                        for arg in &call.args {
                            self.compile_expr(arg, chunk)?;
                        }
                        chunk.emit(OpCode::CallBuiltin(qualified, n));
                        return Ok(());
                    }
                }
                // Otherwise: value.method(args) — compile as CallMethod
                let n = call.args.len();
                for arg in &call.args {
                    self.compile_expr(arg, chunk)?;
                }
                self.compile_expr(&call.target, chunk)?;
                chunk.emit(OpCode::CallMethod(call.method.clone(), n));
            }
            // Unsupported — push Unit
            Expr::Closure(_)
            | Expr::RecordLiteral(_)
            | Expr::TupleLiteral { .. }
            | Expr::Pipeline { .. }
            | Expr::Index { .. }
            | Expr::ErrorProp { .. }
            | Expr::Break { .. }
            | Expr::Continue { .. } => {
                let idx = chunk.add_const(Value::Unit);
                chunk.emit(OpCode::LoadConst(idx));
            }
        }
        Ok(())
    }

    fn compile_if(&mut self, if_expr: &IfExpr, chunk: &mut Chunk) -> Result<(), VmError> {
        let mut end_jumps: Vec<usize> = Vec::new();

        for (cond, body) in &if_expr.branches {
            self.compile_expr(cond, chunk)?;
            let jif_idx = chunk.emit_jump(OpCode::JumpIfFalse(0));

            let prev_depth = self.scope_depth;
            self.scope_depth += 1;
            self.compile_block(body, chunk)?;
            self.scope_depth = prev_depth;

            let jmp_idx = chunk.emit_jump(OpCode::Jump(0));
            end_jumps.push(jmp_idx);

            let after = chunk.code.len();
            chunk.patch_jump(jif_idx, after);
        }

        // else block or push Unit
        if let Some(else_block) = &if_expr.else_block {
            let prev_depth = self.scope_depth;
            self.scope_depth += 1;
            self.compile_block(else_block, chunk)?;
            self.scope_depth = prev_depth;
        } else {
            let idx = chunk.add_const(Value::Unit);
            chunk.emit(OpCode::LoadConst(idx));
        }

        let end = chunk.code.len();
        for jmp in end_jumps {
            chunk.patch_jump(jmp, end);
        }

        Ok(())
    }

    fn compile_while(&mut self, while_stmt: &WhileStmt, chunk: &mut Chunk) -> Result<(), VmError> {
        let loop_start = chunk.code.len();

        self.compile_expr(&while_stmt.cond, chunk)?;
        let jif_idx = chunk.emit_jump(OpCode::JumpIfFalse(0));

        let prev_depth = self.scope_depth;
        self.scope_depth += 1;
        self.compile_block(&while_stmt.body, chunk)?;
        self.scope_depth = prev_depth;
        chunk.emit(OpCode::Pop); // discard block value

        chunk.emit(OpCode::Jump(loop_start));

        let after = chunk.code.len();
        chunk.patch_jump(jif_idx, after);

        let idx = chunk.add_const(Value::Unit);
        chunk.emit(OpCode::LoadConst(idx));
        Ok(())
    }

    fn compile_for(&mut self, for_stmt: &ForStmt, chunk: &mut Chunk) -> Result<(), VmError> {
        // Compile iterable
        self.compile_expr(&for_stmt.iter, chunk)?;

        // Store iterable in a hidden local __iter
        let iter_slot = self.locals.len();
        self.locals.push("__iter__".to_string());
        chunk.emit(OpCode::StoreLocal(iter_slot));

        // __i = 0
        let i_slot = self.locals.len();
        self.locals.push("__i__".to_string());
        let zero_idx = chunk.add_const(Value::Int(0));
        chunk.emit(OpCode::LoadConst(zero_idx));
        chunk.emit(OpCode::StoreLocal(i_slot));

        // loop_start: while __i < List.length(__iter)
        let loop_start = chunk.code.len();
        chunk.emit(OpCode::LoadLocal(i_slot));
        chunk.emit(OpCode::LoadLocal(iter_slot));
        chunk.emit(OpCode::CallBuiltin("List.length".to_string(), 1));
        chunk.emit(OpCode::Lt);
        let jif_idx = chunk.emit_jump(OpCode::JumpIfFalse(0));

        // let <var> = List.get(__iter, __i)
        let var_slot = self.locals.len();
        self.locals.push(for_stmt.name.clone());
        chunk.emit(OpCode::LoadLocal(iter_slot));
        chunk.emit(OpCode::LoadLocal(i_slot));
        chunk.emit(OpCode::CallBuiltin("List.get".to_string(), 2));
        chunk.emit(OpCode::StoreLocal(var_slot));

        // body
        let prev_depth = self.scope_depth;
        self.scope_depth += 1;
        self.compile_block(&for_stmt.body, chunk)?;
        self.scope_depth = prev_depth;
        chunk.emit(OpCode::Pop); // discard block value

        // __i = __i + 1
        chunk.emit(OpCode::LoadLocal(i_slot));
        let one_idx = chunk.add_const(Value::Int(1));
        chunk.emit(OpCode::LoadConst(one_idx));
        chunk.emit(OpCode::Add);
        chunk.emit(OpCode::StoreLocal(i_slot));

        chunk.emit(OpCode::Jump(loop_start));

        let after = chunk.code.len();
        chunk.patch_jump(jif_idx, after);

        // pop hidden locals
        self.locals.pop(); // var
        self.locals.pop(); // __i__
        self.locals.pop(); // __iter__

        let idx = chunk.add_const(Value::Unit);
        chunk.emit(OpCode::LoadConst(idx));
        Ok(())
    }

    fn compile_match(&mut self, match_expr: &MatchExpr, chunk: &mut Chunk) -> Result<(), VmError> {
        // Compile the subject expression and store in a local slot
        self.compile_expr(&match_expr.expr, chunk)?;
        let subject_slot = self.locals.len();
        self.locals.push("__match__".to_string());
        chunk.emit(OpCode::StoreLocal(subject_slot));

        let mut end_jumps: Vec<usize> = Vec::new();

        for arm in &match_expr.arms {
            match &arm.pattern {
                Pattern::Wildcard(_) | Pattern::Ident(_, _) => {
                    // Always matches; bind name if Ident
                    if let Pattern::Ident(name, _) = &arm.pattern {
                        let slot = self.locals.len();
                        self.locals.push(name.clone());
                        chunk.emit(OpCode::LoadLocal(subject_slot));
                        chunk.emit(OpCode::StoreLocal(slot));
                    }
                    let prev_depth = self.scope_depth;
                    self.scope_depth += 1;
                    self.compile_expr(&arm.expr, chunk)?;
                    self.scope_depth = prev_depth;
                    let jmp = chunk.emit_jump(OpCode::Jump(0));
                    end_jumps.push(jmp);
                    // Clean up binding if added
                    if let Pattern::Ident(_, _) = &arm.pattern {
                        self.locals.pop();
                    }
                    break; // wildcard always matches, no further arms
                }
                Pattern::Literal(lit, _) => {
                    // Dup subject and compare
                    chunk.emit(OpCode::LoadLocal(subject_slot));
                    let val = match lit {
                        Literal::Int(n) => Value::Int(*n),
                        Literal::Float(s) => Value::Float(s.parse().unwrap_or(0.0)),
                        Literal::String(s) => Value::String(s.clone()),
                        Literal::Bool(b) => Value::Bool(*b),
                    };
                    let cidx = chunk.add_const(val);
                    chunk.emit(OpCode::LoadConst(cidx));
                    chunk.emit(OpCode::Eq);
                    let jif = chunk.emit_jump(OpCode::JumpIfFalse(0));
                    let prev_depth = self.scope_depth;
                    self.scope_depth += 1;
                    self.compile_expr(&arm.expr, chunk)?;
                    self.scope_depth = prev_depth;
                    let jmp = chunk.emit_jump(OpCode::Jump(0));
                    end_jumps.push(jmp);
                    let next = chunk.code.len();
                    chunk.patch_jump(jif, next);
                }
                Pattern::EnumTuple { name, elems, .. } => {
                    // Check if subject is this variant
                    chunk.emit(OpCode::LoadLocal(subject_slot));
                    chunk.emit(OpCode::IsVariant(name.clone()));
                    let jif = chunk.emit_jump(OpCode::JumpIfFalse(0));

                    // Bind payload elements
                    let mut binding_slots = Vec::new();
                    for (i, elem_pat) in elems.iter().enumerate() {
                        if let Pattern::Ident(bind_name, _) = elem_pat {
                            let slot = self.locals.len();
                            self.locals.push(bind_name.clone());
                            binding_slots.push(slot);
                            chunk.emit(OpCode::LoadLocal(subject_slot));
                            chunk.emit(OpCode::ExtractPayload(i));
                            chunk.emit(OpCode::StoreLocal(slot));
                        }
                    }

                    let prev_depth = self.scope_depth;
                    self.scope_depth += 1;
                    self.compile_expr(&arm.expr, chunk)?;
                    self.scope_depth = prev_depth;
                    let jmp = chunk.emit_jump(OpCode::Jump(0));
                    end_jumps.push(jmp);
                    let next = chunk.code.len();
                    chunk.patch_jump(jif, next);

                    // Pop binding locals
                    for _ in &binding_slots {
                        self.locals.pop();
                    }
                }
                _ => {
                    // Fallback: push Unit for unsupported patterns
                    let idx = chunk.add_const(Value::Unit);
                    chunk.emit(OpCode::LoadConst(idx));
                    let jmp = chunk.emit_jump(OpCode::Jump(0));
                    end_jumps.push(jmp);
                }
            }
        }

        // No arm matched — push Unit
        let idx = chunk.add_const(Value::Unit);
        chunk.emit(OpCode::LoadConst(idx));

        let end = chunk.code.len();
        for jmp in end_jumps {
            chunk.patch_jump(jmp, end);
        }

        self.locals.pop(); // __match__
        Ok(())
    }

    fn compile_fn(&mut self, decl: &FnDecl) -> Result<Chunk, VmError> {
        let mut fn_chunk = Chunk::new(&decl.name, decl.params.len(), false);

        let old_locals = std::mem::take(&mut self.locals);
        let old_depth = self.scope_depth;
        self.scope_depth = 1;

        // Params become locals at slots 0..n
        for param in &decl.params {
            self.locals.push(param.name.clone());
        }

        // Compile body statements
        for stmt in &decl.body.stmts {
            self.compile_stmt(stmt, &mut fn_chunk)?;
        }
        // Tail expression is the return value
        if let Some(tail) = &decl.body.tail_expr {
            self.compile_expr(tail, &mut fn_chunk)?;
        } else {
            let idx = fn_chunk.add_const(Value::Unit);
            fn_chunk.emit(OpCode::LoadConst(idx));
        }
        fn_chunk.emit(OpCode::Return);

        self.locals = old_locals;
        self.scope_depth = old_depth;
        Ok(fn_chunk)
    }
}

/// Compile a Lace program source string into a list of Chunks.
/// Index 0 is always "main" (either the bootstrap loader or the actual main fn).
pub fn compile_program(source: &str) -> Result<Vec<Chunk>, VmError> {
    let (program_opt, errors) = parse_program(source);
    if !errors.is_empty() {
        return Err(VmError::CompileError(format!("{:?}", errors)));
    }
    let program = program_opt.ok_or_else(|| VmError::CompileError("parse returned None".into()))?;

    let mut compiler = Compiler::new();

    // First pass: collect tool names
    for item in &program.items {
        if let TopLevelItem::Tool(tool) = item {
            compiler.tool_names.insert(tool.name.clone());
        }
    }

    // The bootstrap chunk registers all functions as globals then calls main.
    // Named fn chunks go into compiler.chunks.
    let mut bootstrap = Chunk::new("__bootstrap__", 0, false);
    let mut fn_chunks: Vec<Chunk> = Vec::new();

    for item in &program.items {
        match item {
            TopLevelItem::Function(fn_decl) => {
                let fn_chunk = compiler.compile_fn(fn_decl)?;
                fn_chunks.push(fn_chunk);
            }
            TopLevelItem::Tool(tool_decl) => {
                // ToolDecl has no body — create a stub chunk
                let stub = Chunk::new(&tool_decl.name, tool_decl.params.len(), true);
                fn_chunks.push(stub);
            }
            TopLevelItem::Const(const_decl) => {
                compiler.compile_expr(&const_decl.expr, &mut bootstrap)?;
                bootstrap.emit(OpCode::StoreGlobal(const_decl.name.clone()));
            }
            _ => {}
        }
    }

    bootstrap.emit(OpCode::Halt);

    // Build final chunk list:
    // [0] = bootstrap, [1..] = fn chunks
    // The VM's run() finds chunk named "main" by name.
    let mut all_chunks = vec![bootstrap];
    all_chunks.extend(fn_chunks);
    Ok(all_chunks)
}

/// Returns true if `name` is a known free stdlib builtin (not a module-qualified call).
fn is_free_builtin(name: &str) -> bool {
    matches!(
        name,
        "to_string"
            | "len"
            | "type_of"
            | "assert"
            | "assert_eq"
            | "assert_err"
            | "now_unix"
            | "now_millis"
            | "int_to_float"
            | "float_to_int"
            | "parse_int"
            | "parse_float"
    )
}

/// Returns true if `name` is a known stdlib module name.
fn is_stdlib_module(name: &str) -> bool {
    matches!(
        name,
        "List"
            | "Json"
            | "Http"
            | "File"
            | "Env"
            | "Math"
            | "String"
            | "Map"
            | "Os"
            | "Process"
    )
}
