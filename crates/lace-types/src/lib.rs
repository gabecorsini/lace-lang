use std::collections::{BTreeMap, HashMap};

use lace_ast::*;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Float,
    Bool,
    String,
    Bytes,
    Unit,
    Option(Box<Type>),
    Result(Box<Type>, Box<Type>),
    Confident(Box<Type>),
    Uncertain(Box<Type>),
    Scored(Box<Type>),
    Record(String, BTreeMap<String, Type>),
    Tuple(Vec<Type>),
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Fn(Vec<Type>, Box<Type>),
    Named(String, Vec<Type>),
    Dynamic,
    Unknown,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TypeError {
    #[error("unknown identifier '{name}' at {span_start}..{span_end}")]
    UnknownIdentifier {
        name: String,
        span_start: usize,
        span_end: usize,
    },
    #[error("type mismatch at {span_start}..{span_end}: expected {expected:?}, found {found:?}")]
    Mismatch {
        expected: Type,
        found: Type,
        span_start: usize,
        span_end: usize,
    },
    #[error("unknown function '{name}' at {span_start}..{span_end}")]
    UnknownFunction {
        name: String,
        span_start: usize,
        span_end: usize,
    },
    #[error("unknown record type '{name}'")]
    UnknownRecordType { name: String },
    #[error("invalid pattern at {span_start}..{span_end}: {message}")]
    InvalidPattern {
        message: String,
        span_start: usize,
        span_end: usize,
    },
    #[error("invalid tool declaration '{name}': {message}")]
    InvalidToolDecl { name: String, message: String },
}

#[derive(Debug, Clone)]
struct Scope {
    vars: HashMap<String, Type>,
}

pub fn check_program(program: &Program) -> Vec<TypeError> {
    let mut checker = Checker::new();
    // Register imported module names as Dynamic so field access type-checks pass
    for import in &program.imports {
        if let Some(name) = import.path.last() {
            checker.scopes[0].vars.insert(name.clone(), Type::Dynamic);
        }
    }
    checker.collect_signatures(program);
    checker.check(program);
    checker.errors
}

struct Checker {
    scopes: Vec<Scope>,
    fn_sigs: HashMap<String, (Vec<Type>, Type)>,
    record_types: HashMap<String, BTreeMap<String, Type>>,
    errors: Vec<TypeError>,
}

impl Checker {
    fn new() -> Self {
        let mut checker = Self {
            scopes: vec![Scope {
                vars: HashMap::new(),
            }],
            fn_sigs: HashMap::new(),
            record_types: HashMap::new(),
            errors: Vec::new(),
        };
        checker.install_prelude();
        checker
    }

    fn install_prelude(&mut self) {
        // Register module names as dynamic values so FieldAccess type-checks pass
        self.scopes[0].vars.insert("List".into(), Type::Dynamic);

        self.fn_sigs
            .insert("print".into(), (vec![Type::String], Type::Unit));
        self.fn_sigs
            .insert("println".into(), (vec![Type::String], Type::Unit));
        self.fn_sigs
            .insert("type_of".into(), (vec![Type::Dynamic], Type::String));
        self.fn_sigs
            .insert("to_string".into(), (vec![Type::Dynamic], Type::String));
        self.fn_sigs.insert(
            "List.length".into(),
            (vec![Type::List(Box::new(Type::Dynamic))], Type::Int),
        );
        self.fn_sigs.insert(
            "List.range".into(),
            (vec![Type::Int, Type::Int], Type::List(Box::new(Type::Int))),
        );
        self.fn_sigs.insert(
            "List.map".into(),
            (
                vec![
                    Type::List(Box::new(Type::Dynamic)),
                    Type::Fn(vec![Type::Dynamic], Box::new(Type::Dynamic)),
                ],
                Type::List(Box::new(Type::Dynamic)),
            ),
        );
        self.fn_sigs.insert(
            "List.filter".into(),
            (
                vec![
                    Type::List(Box::new(Type::Dynamic)),
                    Type::Fn(vec![Type::Dynamic], Box::new(Type::Bool)),
                ],
                Type::List(Box::new(Type::Dynamic)),
            ),
        );
    }

    fn collect_signatures(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                TopLevelItem::Record(r) => {
                    let mut fields = BTreeMap::new();
                    for f in &r.fields {
                        fields.insert(f.name.clone(), self.resolve_type_expr(&f.ty));
                    }
                    self.record_types.insert(r.name.clone(), fields);
                }
                TopLevelItem::Function(f) => {
                    let params = f
                        .params
                        .iter()
                        .map(|p| self.resolve_type_expr(&p.ty))
                        .collect::<Vec<_>>();
                    let ret = f
                        .ret_ty
                        .as_ref()
                        .map(|t| self.resolve_type_expr(t))
                        .unwrap_or(Type::Unit);
                    self.fn_sigs.insert(f.name.clone(), (params, ret));
                }
                TopLevelItem::Tool(t) => {
                    let params = t
                        .params
                        .iter()
                        .map(|p| self.resolve_type_expr(&p.ty))
                        .collect::<Vec<_>>();
                    let ret = self.resolve_type_expr(&t.ret_ty);
                    self.fn_sigs.insert(t.name.clone(), (params, ret));
                }
                _ => {}
            }
        }
    }

    fn check(&mut self, program: &Program) {
        for item in &program.items {
            match item {
                TopLevelItem::Const(c) => {
                    let expected = self.resolve_type_expr(&c.ty);
                    let found = self.infer_expr(&c.expr);
                    self.unify(expected, found, c.span);
                }
                TopLevelItem::Function(f) => self.check_fn(f),
                TopLevelItem::Tool(t) => self.check_tool_decl(t),
                _ => {}
            }
        }
    }

    fn check_tool_decl(&mut self, t: &ToolDecl) {
        for p in &t.params {
            if let Some(default) = &p.default {
                let expected = self.resolve_type_expr(&p.ty);
                let found = self.infer_expr(default);
                self.unify(expected, found, p.span);
            }
        }

        let mut seen_retries = false;
        let mut seen_timeout = false;
        let mut seen_mock = false;
        for opt in &t.options {
            match opt {
                ToolOption::Retries(v, _) => {
                    if *v < 0 {
                        self.errors.push(TypeError::InvalidToolDecl {
                            name: t.name.clone(),
                            message: "retries must be >= 0".into(),
                        });
                    }
                    if seen_retries {
                        self.errors.push(TypeError::InvalidToolDecl {
                            name: t.name.clone(),
                            message: "duplicate retries option".into(),
                        });
                    }
                    seen_retries = true;
                }
                ToolOption::Timeout(d, _) => {
                    if d.value <= 0 {
                        self.errors.push(TypeError::InvalidToolDecl {
                            name: t.name.clone(),
                            message: "timeout must be > 0".into(),
                        });
                    }
                    if seen_timeout {
                        self.errors.push(TypeError::InvalidToolDecl {
                            name: t.name.clone(),
                            message: "duplicate timeout option".into(),
                        });
                    }
                    seen_timeout = true;
                }
                ToolOption::Mock(mock_name, _) => {
                    if seen_mock {
                        self.errors.push(TypeError::InvalidToolDecl {
                            name: t.name.clone(),
                            message: "duplicate mock option".into(),
                        });
                    }
                    seen_mock = true;

                    if let Some((mock_params, mock_ret)) = self.fn_sigs.get(mock_name).cloned() {
                        let tool_params = t
                            .params
                            .iter()
                            .map(|p| self.resolve_type_expr(&p.ty))
                            .collect::<Vec<_>>();
                        let tool_ret = self.resolve_type_expr(&t.ret_ty);

                        if mock_params.len() != tool_params.len() {
                            self.errors.push(TypeError::InvalidToolDecl {
                                name: t.name.clone(),
                                message: format!(
                                    "mock '{}' arity mismatch: expected {}, got {}",
                                    mock_name,
                                    tool_params.len(),
                                    mock_params.len()
                                ),
                            });
                        }

                        for (expected, found) in tool_params.iter().zip(mock_params.iter()) {
                            if !self.compatible(expected, found) {
                                self.errors.push(TypeError::InvalidToolDecl {
                                    name: t.name.clone(),
                                    message: format!(
                                        "mock '{}' parameter type mismatch: expected {:?}, found {:?}",
                                        mock_name, expected, found
                                    ),
                                });
                            }
                        }

                        if !self.compatible(&tool_ret, &mock_ret) {
                            self.errors.push(TypeError::InvalidToolDecl {
                                name: t.name.clone(),
                                message: format!(
                                    "mock '{}' return type mismatch: expected {:?}, found {:?}",
                                    mock_name, tool_ret, mock_ret
                                ),
                            });
                        }
                    } else {
                        self.errors.push(TypeError::InvalidToolDecl {
                            name: t.name.clone(),
                            message: format!("mock function '{}' not found", mock_name),
                        });
                    }
                }
            }
        }
    }

    fn check_fn(&mut self, f: &FnDecl) {
        self.push_scope();
        for p in &f.params {
            let ty = self.resolve_type_expr(&p.ty);
            self.define(p.name.clone(), ty);
        }

        for stmt in &f.body.stmts {
            self.check_stmt(stmt);
        }
        let tail = f
            .body
            .tail_expr
            .as_ref()
            .map(|e| self.infer_expr(e))
            .unwrap_or(Type::Unit);

        if let Some(ret) = &f.ret_ty {
            self.unify(self.resolve_type_expr(ret), tail, f.span);
        }

        self.pop_scope();
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let(s) | Stmt::MutLet(s) => {
                let inferred = self.infer_expr(&s.expr);
                if let Some(explicit) = &s.ty {
                    let expected = self.resolve_type_expr(explicit);
                    self.unify(expected.clone(), inferred, s.span);
                    self.define(s.name.clone(), expected);
                } else {
                    self.define(s.name.clone(), inferred);
                }
            }
            Stmt::Assign(a) => {
                let expected = self.lookup(&a.name).unwrap_or(Type::Unknown);
                let found = self.infer_expr(&a.expr);
                self.unify(expected, found, a.span);
            }
            Stmt::Expr(e) => {
                let _ = self.infer_expr(e);
            }
            Stmt::For(f) => {
                let _iter = self.infer_expr(&f.iter);
                self.push_scope();
                self.define(f.name.clone(), Type::Unknown);
                for s in &f.body.stmts {
                    self.check_stmt(s);
                }
                if let Some(t) = &f.body.tail_expr {
                    let _ = self.infer_expr(t);
                }
                self.pop_scope();
            }
            Stmt::While(w) => {
                let cond = self.infer_expr(&w.cond);
                self.unify(Type::Bool, cond, w.span);
                self.push_scope();
                for s in &w.body.stmts {
                    self.check_stmt(s);
                }
                if let Some(t) = &w.body.tail_expr {
                    let _ = self.infer_expr(t);
                }
                self.pop_scope();
            }
            Stmt::PureBlock(b) => {
                self.push_scope();
                for s in &b.stmts {
                    self.check_stmt(s);
                }
                if let Some(t) = &b.tail_expr {
                    let _ = self.infer_expr(t);
                }
                self.pop_scope();
            }
        }
    }

    fn infer_expr(&mut self, expr: &Expr) -> Type {
        match expr {
            Expr::Literal(l, _) => match l {
                Literal::Int(_) => Type::Int,
                Literal::Float(_) => Type::Float,
                Literal::String(_) => Type::String,
                Literal::Bool(_) => Type::Bool,
            },
            Expr::Ident(name, span) => {
                if let Some(t) = self.lookup(name) {
                    t
                } else if self.fn_sigs.contains_key(name.as_str()) {
                    // Bare function name used as a first-class reference
                    let (params, ret) = self.fn_sigs[name.as_str()].clone();
                    Type::Fn(params, Box::new(ret))
                } else {
                    self.errors.push(TypeError::UnknownIdentifier {
                        name: name.clone(),
                        span_start: span.start,
                        span_end: span.end,
                    });
                    Type::Unknown
                }
            }
            Expr::Block(b) => {
                self.push_scope();
                for s in &b.stmts {
                    self.check_stmt(s);
                }
                let t = b
                    .tail_expr
                    .as_ref()
                    .map(|e| self.infer_expr(e))
                    .unwrap_or(Type::Unit);
                self.pop_scope();
                t
            }
            Expr::If(i) => {
                let mut branch_t: Option<Type> = None;
                for (cond, blk) in &i.branches {
                    let cond_ty = self.infer_expr(cond);
                    self.unify(Type::Bool, cond_ty, cond.span());
                    let t = self.infer_block_type(blk);
                    branch_t = Some(match branch_t {
                        Some(prev) => self.unify_soft(prev, t),
                        None => t,
                    });
                }
                if let Some(else_blk) = &i.else_block {
                    let t = self.infer_block_type(else_blk);
                    branch_t = Some(match branch_t {
                        Some(prev) => self.unify_soft(prev, t),
                        None => t,
                    });
                }
                branch_t.unwrap_or(Type::Unit)
            }
            Expr::Match(m) => {
                let scrutinee = self.infer_expr(&m.expr);
                let mut out: Option<Type> = None;
                for arm in &m.arms {
                    self.push_scope();
                    self.bind_pattern(&arm.pattern, &scrutinee);
                    let t = self.infer_expr(&arm.expr);
                    self.pop_scope();
                    out = Some(match out {
                        Some(prev) => self.unify_soft(prev, t),
                        None => t,
                    });
                }
                out.unwrap_or(Type::Unit)
            }
            Expr::FnCall(call) => {
                let args = call
                    .args
                    .iter()
                    .map(|a| self.infer_expr(a))
                    .collect::<Vec<_>>();
                if let Some((params, ret)) = self.fn_sigs.get(&call.name).cloned() {
                    for (i, arg) in args.iter().enumerate() {
                        if let Some(param) = params.get(i) {
                            self.unify(param.clone(), arg.clone(), call.span);
                        }
                    }
                    ret
                } else {
                    self.errors.push(TypeError::UnknownFunction {
                        name: call.name.clone(),
                        span_start: call.span.start,
                        span_end: call.span.end,
                    });
                    Type::Unknown
                }
            }
            Expr::MethodCall(c) => {
                let _ = self.infer_expr(&c.target);
                for a in &c.args {
                    let _ = self.infer_expr(a);
                }
                Type::Unknown
            }
            Expr::FieldAccess {
                target,
                field,
                span,
            } => match self.infer_expr(target) {
                Type::Record(_, fields) => fields.get(field).cloned().unwrap_or(Type::Unknown),
                // Module access (e.g. List.range) or dynamic value - return Dynamic
                Type::Dynamic | Type::Unknown | Type::String => {
                    let _ = (field, span);
                    Type::Dynamic
                }
                _ => {
                    let found = self.infer_expr(target);
                    self.errors.push(TypeError::Mismatch {
                        expected: Type::Record("<any>".into(), BTreeMap::new()),
                        found,
                        span_start: span.start,
                        span_end: span.end,
                    });
                    Type::Unknown
                }
            },
            Expr::Index { target, index, .. } => {
                let index_ty = self.infer_expr(index);
                self.unify(Type::Int, index_ty, index.span());
                match self.infer_expr(target) {
                    Type::List(t) => *t,
                    Type::Tuple(ts) => ts.into_iter().next().unwrap_or(Type::Unknown),
                    _ => Type::Unknown,
                }
            }
            Expr::Pipeline { left, right, .. } => {
                let _ = self.infer_expr(left);
                self.infer_expr(right)
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let lt = self.infer_expr(left);
                let rt = self.infer_expr(right);
                match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Rem => {
                        if lt == Type::Float || rt == Type::Float {
                            Type::Float
                        } else {
                            self.unify(Type::Int, lt, *span);
                            self.unify(Type::Int, rt, *span);
                            Type::Int
                        }
                    }
                    BinaryOp::Eq
                    | BinaryOp::Ne
                    | BinaryOp::Lt
                    | BinaryOp::Gt
                    | BinaryOp::Le
                    | BinaryOp::Ge
                    | BinaryOp::And
                    | BinaryOp::Or => Type::Bool,
                    BinaryOp::Concat => {
                        if lt == Type::String && rt == Type::String {
                            Type::String
                        } else {
                            Type::Unknown
                        }
                    }
                }
            }
            Expr::Unary { op, expr, span } => {
                let t = self.infer_expr(expr);
                match op {
                    UnaryOp::Neg => {
                        if t == Type::Float {
                            Type::Float
                        } else {
                            self.unify(Type::Int, t, *span);
                            Type::Int
                        }
                    }
                    UnaryOp::Not => {
                        self.unify(Type::Bool, t, *span);
                        Type::Bool
                    }
                }
            }
            Expr::Closure(c) => {
                let mut params = Vec::new();
                self.push_scope();
                for p in &c.params {
                    let t =
                        p.ty.as_ref()
                            .map(|x| self.resolve_type_expr(x))
                            .unwrap_or(Type::Dynamic);
                    self.define(p.name.clone(), t.clone());
                    params.push(t);
                }
                let ret = c
                    .ret_ty
                    .as_ref()
                    .map(|t| self.resolve_type_expr(t))
                    .unwrap_or_else(|| {
                        c.body
                            .tail_expr
                            .as_ref()
                            .map(|e| self.infer_expr(e))
                            .unwrap_or(Type::Unit)
                    });
                self.pop_scope();
                Type::Fn(params, Box::new(ret))
            }
            Expr::RecordLiteral(r) => {
                if let Some(fields) = self.record_types.get(&r.name).cloned() {
                    for (fname, expr, span) in &r.fields {
                        if let Some(expected) = fields.get(fname) {
                            let found = self.infer_expr(expr);
                            self.unify(expected.clone(), found, *span);
                        }
                    }
                    Type::Record(r.name.clone(), fields)
                } else {
                    self.errors.push(TypeError::UnknownRecordType {
                        name: r.name.clone(),
                    });
                    Type::Unknown
                }
            }
            Expr::ListLiteral { elems, .. } => {
                if elems.is_empty() {
                    Type::List(Box::new(Type::Unknown))
                } else {
                    let first = self.infer_expr(&elems[0]);
                    for e in &elems[1..] {
                        let other = self.infer_expr(e);
                        self.unify(first.clone(), other, e.span());
                    }
                    Type::List(Box::new(first))
                }
            }
            Expr::TupleLiteral { elems, .. } => {
                Type::Tuple(elems.iter().map(|e| self.infer_expr(e)).collect())
            }
            Expr::Return { value, .. } => value
                .as_ref()
                .map(|v| self.infer_expr(v))
                .unwrap_or(Type::Unit),
            Expr::ErrorProp { expr, .. } => match self.infer_expr(expr) {
                Type::Result(ok, _err) => *ok,
                other => other,
            },
        }
    }

    fn bind_pattern(&mut self, pat: &Pattern, scrutinee: &Type) {
        match pat {
            Pattern::Wildcard(_) => {}
            Pattern::Literal(l, span) => {
                let expected = match l {
                    Literal::Int(_) => Type::Int,
                    Literal::Float(_) => Type::Float,
                    Literal::String(_) => Type::String,
                    Literal::Bool(_) => Type::Bool,
                };
                self.unify(expected, scrutinee.clone(), *span);
            }
            Pattern::Ident(name, _) => {
                self.define(name.clone(), scrutinee.clone());
            }
            Pattern::Tuple(parts, span) => {
                if let Type::Tuple(elems) = scrutinee {
                    for (p, t) in parts.iter().zip(elems.iter()) {
                        self.bind_pattern(p, t);
                    }
                } else {
                    self.errors.push(TypeError::InvalidPattern {
                        message: "tuple pattern used on non-tuple value".into(),
                        span_start: span.start,
                        span_end: span.end,
                    });
                }
            }
            Pattern::EnumTuple { name, elems, span } => match (name.as_str(), scrutinee) {
                ("Confident", Type::Confident(inner)) if elems.len() == 1 => {
                    self.bind_pattern(&elems[0], inner);
                }
                ("Uncertain", Type::Uncertain(inner)) if elems.len() == 1 => {
                    self.bind_pattern(&elems[0], inner);
                }
                ("Ok", Type::Result(ok, _)) if elems.len() == 1 => {
                    self.bind_pattern(&elems[0], ok);
                }
                ("Err", Type::Result(_, err)) if elems.len() == 1 => {
                    self.bind_pattern(&elems[0], err);
                }
                ("Some", Type::Option(inner)) if elems.len() == 1 => {
                    self.bind_pattern(&elems[0], inner);
                }
                ("None", Type::Option(_)) if elems.is_empty() => {}
                _ => self.errors.push(TypeError::InvalidPattern {
                    message: format!(
                        "constructor pattern '{}' does not match scrutinee type {:?}",
                        name, scrutinee
                    ),
                    span_start: span.start,
                    span_end: span.end,
                }),
            },
            Pattern::EnumStruct { span, .. } | Pattern::Record { span, .. } => {
                // Structural pattern typing is intentionally lightweight in Phase 2.
                if matches!(scrutinee, Type::Unknown | Type::Dynamic) {
                    return;
                }
                let _ = span;
            }
            Pattern::Or(left, right, _) => {
                self.bind_pattern(left, scrutinee);
                self.bind_pattern(right, scrutinee);
            }
        }
    }

    fn infer_block_type(&mut self, b: &Block) -> Type {
        self.push_scope();
        for s in &b.stmts {
            self.check_stmt(s);
        }
        let t = b
            .tail_expr
            .as_ref()
            .map(|e| self.infer_expr(e))
            .unwrap_or(Type::Unit);
        self.pop_scope();
        t
    }

    fn resolve_type_expr(&self, ty: &TypeExpr) -> Type {
        match ty {
            TypeExpr::Primitive(p, _) => match p {
                PrimitiveType::Int => Type::Int,
                PrimitiveType::Float => Type::Float,
                PrimitiveType::Bool => Type::Bool,
                PrimitiveType::String => Type::String,
                PrimitiveType::Bytes => Type::Bytes,
                PrimitiveType::Unit => Type::Unit,
            },
            TypeExpr::Dynamic(_) => Type::Dynamic,
            TypeExpr::Tuple { elems, .. } => {
                Type::Tuple(elems.iter().map(|e| self.resolve_type_expr(e)).collect())
            }
            TypeExpr::Function { params, ret, .. } => Type::Fn(
                params.iter().map(|p| self.resolve_type_expr(p)).collect(),
                Box::new(self.resolve_type_expr(ret)),
            ),
            TypeExpr::Named { name, .. } => Type::Named(name.clone(), Vec::new()),
            TypeExpr::Generic { name, args, .. } => {
                let lowered = args
                    .iter()
                    .map(|a| self.resolve_type_expr(a))
                    .collect::<Vec<_>>();
                match name.as_str() {
                    "Option" if lowered.len() == 1 => Type::Option(Box::new(lowered[0].clone())),
                    "Result" if lowered.len() == 2 => {
                        Type::Result(Box::new(lowered[0].clone()), Box::new(lowered[1].clone()))
                    }
                    "List" if lowered.len() == 1 => Type::List(Box::new(lowered[0].clone())),
                    "Map" if lowered.len() == 2 => {
                        Type::Map(Box::new(lowered[0].clone()), Box::new(lowered[1].clone()))
                    }
                    "Confident" if lowered.len() == 1 => {
                        Type::Confident(Box::new(lowered[0].clone()))
                    }
                    "Uncertain" if lowered.len() == 1 => {
                        Type::Uncertain(Box::new(lowered[0].clone()))
                    }
                    "Scored" if lowered.len() == 1 => Type::Scored(Box::new(lowered[0].clone())),
                    _ => Type::Named(name.clone(), lowered),
                }
            }
        }
    }

    fn unify(&mut self, expected: Type, found: Type, span: Span) {
        if !self.compatible(&expected, &found) {
            self.errors.push(TypeError::Mismatch {
                expected,
                found,
                span_start: span.start,
                span_end: span.end,
            });
        }
    }

    fn unify_soft(&self, left: Type, right: Type) -> Type {
        if self.compatible(&left, &right) {
            left
        } else {
            Type::Unknown
        }
    }

    fn compatible(&self, a: &Type, b: &Type) -> bool {
        if *a == Type::Unknown || *b == Type::Unknown || *a == Type::Dynamic || *b == Type::Dynamic
        {
            return true;
        }
        a == b
    }

    fn define(&mut self, name: String, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.insert(name, ty);
        }
    }

    fn lookup(&self, name: &str) -> Option<Type> {
        for scope in self.scopes.iter().rev() {
            if let Some(t) = scope.vars.get(name) {
                return Some(t.clone());
            }
        }
        None
    }

    fn push_scope(&mut self) {
        self.scopes.push(Scope {
            vars: HashMap::new(),
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
}
