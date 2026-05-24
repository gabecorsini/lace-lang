use lace_ast::*;
use lace_interp::Value;
use lace_parser::parse_program;

use crate::chunk::Chunk;
use crate::error::VmError;
use crate::opcode::OpCode;

struct Compiler {
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
            Stmt::For(_) | Stmt::While(_) | Stmt::PureBlock(_) => {
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

            Expr::Block(block) => {
                let prev_depth = self.scope_depth;
                self.scope_depth += 1;
                self.compile_block(block, chunk)?;
                self.scope_depth = prev_depth;
            }

            Expr::FnCall(call) => {
                if call.name == "print" {
                    // compile first arg (or Unit)
                    if let Some(arg) = call.args.first() {
                        self.compile_expr(arg, chunk)?;
                    } else {
                        let idx = chunk.add_const(Value::Unit);
                        chunk.emit(OpCode::LoadConst(idx));
                    }
                    chunk.emit(OpCode::Print);
                } else {
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

            // Unsupported — push Unit
            Expr::Match(_)
            | Expr::Closure(_)
            | Expr::RecordLiteral(_)
            | Expr::TupleLiteral { .. }
            | Expr::MethodCall(_)
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
