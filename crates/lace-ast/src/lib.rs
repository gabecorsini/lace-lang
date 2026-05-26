use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Program {
    pub module: Option<ModuleDecl>,
    pub uses: Vec<UseDecl>,
    pub imports: Vec<ImportDecl>,
    pub items: Vec<TopLevelItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleDecl {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub imports: Option<Vec<String>>,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDecl {
    /// Relative file path, e.g. "./utils.lace"
    pub file_path: String,
    /// Alias identifier, e.g. `utils` in `import "./utils.lace" as utils`
    pub alias: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopLevelItem {
    Function(FnDecl),
    Tool(ToolDecl),
    Record(RecordDecl),
    Enum(EnumDecl),
    TypeAlias(TypeAliasDecl),
    Const(ConstDecl),
    Extern(ExternDecl),
    Statement(Stmt),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub name: String,
    pub args: Vec<AnnotationArg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationArg {
    pub name: String,
    pub value: AnnotationValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotationValue {
    Int(i64),
    String(String),
    Bool(bool),
    Duration(DurationLit),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationLit {
    pub value: i64,
    pub unit: DurationUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurationUnit {
    Ms,
    S,
    M,
    H,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnDecl {
    pub doc_comment: Option<String>,
    pub annotations: Vec<Annotation>,
    pub is_pub: bool,
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret_ty: Option<TypeExpr>,
    pub effects: Vec<EffectExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDecl {
    pub doc_comment: Option<String>,
    pub annotations: Vec<Annotation>,
    pub is_pub: bool,
    pub name: String,
    pub params: Vec<ToolParam>,
    pub ret_ty: TypeExpr,
    pub options: Vec<ToolOption>,
    pub body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordDecl {
    pub doc_comment: Option<String>,
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub fields: Vec<RecordField>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeAliasDecl {
    pub name: String,
    pub generics: Vec<GenericParam>,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstDecl {
    pub name: String,
    pub ty: TypeExpr,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternDecl {
    pub name: String,
    pub params: Vec<Param>,
    pub ret_ty: TypeExpr,
    pub effects: Vec<EffectExpr>,
    pub source: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericParam {
    pub name: String,
    pub kind: GenericParamKind,
    pub bounds: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenericParamKind {
    Type,
    Effect,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolParam {
    pub name: String,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolOption {
    Retries(i64, Span),
    Timeout(DurationLit, Span),
    Mock(String, Span),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordField {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub body: Option<EnumVariantBody>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnumVariantBody {
    Tuple(Vec<TypeExpr>),
    Struct(Vec<RecordField>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectExpr {
    Builtin(EffectTag),
    Variable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectTag {
    Pure,
    Io,
    Mut,
    ToolCall,
    Time,
    Rand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeExpr {
    Primitive(PrimitiveType, Span),
    Generic {
        name: String,
        args: Vec<TypeExpr>,
        span: Span,
    },
    Tuple {
        elems: Vec<TypeExpr>,
        span: Span,
    },
    Function {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
        effects: Vec<EffectExpr>,
        span: Span,
    },
    Dynamic(Span),
    Named {
        name: String,
        span: Span,
    },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Primitive(_, s)
            | TypeExpr::Dynamic(s)
            | TypeExpr::Named { span: s, .. }
            | TypeExpr::Generic { span: s, .. }
            | TypeExpr::Tuple { span: s, .. }
            | TypeExpr::Function { span: s, .. } => *s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrimitiveType {
    Int,
    Float,
    Bool,
    String,
    Bytes,
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail_expr: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stmt {
    Let(LetStmt),
    MutLet(LetStmt),
    Assign(AssignStmt),
    Expr(Expr),
    For(ForStmt),
    While(WhileStmt),
    PureBlock(Block),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LetStmt {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignStmt {
    pub name: String,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForStmt {
    pub name: String,
    pub iter: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhileStmt {
    pub cond: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    Literal(Literal, Span),
    Ident(String, Span),
    Block(Block),
    If(IfExpr),
    Match(MatchExpr),
    FnCall(FnCallExpr),
    MethodCall(MethodCallExpr),
    FieldAccess {
        target: Box<Expr>,
        field: String,
        span: Span,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Pipeline {
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
        span: Span,
    },
    Closure(ClosureExpr),
    RecordLiteral(RecordLiteralExpr),
    ListLiteral {
        elems: Vec<Expr>,
        span: Span,
    },
    TupleLiteral {
        elems: Vec<Expr>,
        span: Span,
    },
    Return {
        value: Option<Box<Expr>>,
        span: Span,
    },
    ErrorProp {
        expr: Box<Expr>,
        span: Span,
    },
    Break {
        span: Span,
    },
    Continue {
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Literal(_, s)
            | Expr::Ident(_, s)
            | Expr::ListLiteral { span: s, .. }
            | Expr::TupleLiteral { span: s, .. }
            | Expr::FieldAccess { span: s, .. }
            | Expr::Index { span: s, .. }
            | Expr::Pipeline { span: s, .. }
            | Expr::Binary { span: s, .. }
            | Expr::Unary { span: s, .. }
            | Expr::Return { span: s, .. }
            | Expr::ErrorProp { span: s, .. }
            | Expr::Break { span: s }
            | Expr::Continue { span: s } => *s,
            Expr::Block(b) => b.span,
            Expr::If(i) => i.span,
            Expr::Match(m) => m.span,
            Expr::FnCall(c) => c.span,
            Expr::MethodCall(c) => c.span,
            Expr::Closure(c) => c.span,
            Expr::RecordLiteral(r) => r.span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Literal {
    Int(i64),
    Float(String),
    String(String),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    IntDiv,
    Rem,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Concat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IfExpr {
    pub branches: Vec<(Expr, Block)>,
    pub else_block: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchExpr {
    pub expr: Box<Expr>,
    pub arms: Vec<MatchArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Box<Expr>>,
    pub expr: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    Wildcard(Span),
    Literal(Literal, Span),
    Ident(String, Span),
    Tuple(Vec<Pattern>, Span),
    EnumTuple {
        name: String,
        elems: Vec<Pattern>,
        span: Span,
    },
    EnumStruct {
        name: String,
        fields: Vec<(String, Pattern)>,
        span: Span,
    },
    Record {
        name: String,
        fields: Vec<(String, Pattern)>,
        rest: bool,
        span: Span,
    },
    Or(Box<Pattern>, Box<Pattern>, Span),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnCallExpr {
    pub name: String,
    pub type_arg: Option<String>,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MethodCallExpr {
    pub target: Box<Expr>,
    pub method: String,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureExpr {
    pub params: Vec<ClosureParam>,
    pub ret_ty: Option<TypeExpr>,
    pub effects: Vec<EffectExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosureParam {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordLiteralExpr {
    pub name: String,
    pub fields: Vec<(String, Expr, Span)>,
    pub span: Span,
}

pub trait Visitor {
    fn visit_program(&mut self, program: &Program) {
        walk_program(self, program)
    }

    fn visit_top_level_item(&mut self, item: &TopLevelItem) {
        walk_top_level_item(self, item)
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt)
    }

    fn visit_expr(&mut self, expr: &Expr) {
        walk_expr(self, expr)
    }

    fn visit_type_expr(&mut self, ty: &TypeExpr) {
        walk_type_expr(self, ty)
    }
}

pub fn walk_program<V: Visitor + ?Sized>(v: &mut V, program: &Program) {
    for item in &program.items {
        v.visit_top_level_item(item);
    }
}

pub fn walk_top_level_item<V: Visitor + ?Sized>(v: &mut V, item: &TopLevelItem) {
    match item {
        TopLevelItem::Function(f) => {
            for p in &f.params {
                v.visit_type_expr(&p.ty);
            }
            if let Some(ret) = &f.ret_ty {
                v.visit_type_expr(ret);
            }
            for s in &f.body.stmts {
                v.visit_stmt(s);
            }
            if let Some(e) = &f.body.tail_expr {
                v.visit_expr(e);
            }
        }
        TopLevelItem::Tool(t) => {
            for p in &t.params {
                v.visit_type_expr(&p.ty);
                if let Some(d) = &p.default {
                    v.visit_expr(d);
                }
            }
            v.visit_type_expr(&t.ret_ty);
        }
        TopLevelItem::Record(r) => {
            for f in &r.fields {
                v.visit_type_expr(&f.ty);
            }
        }
        TopLevelItem::Enum(e) => {
            for variant in &e.variants {
                match &variant.body {
                    Some(EnumVariantBody::Tuple(ts)) => {
                        for t in ts {
                            v.visit_type_expr(t);
                        }
                    }
                    Some(EnumVariantBody::Struct(fs)) => {
                        for f in fs {
                            v.visit_type_expr(&f.ty);
                        }
                    }
                    None => {}
                }
            }
        }
        TopLevelItem::TypeAlias(t) => v.visit_type_expr(&t.ty),
        TopLevelItem::Const(c) => {
            v.visit_type_expr(&c.ty);
            v.visit_expr(&c.expr);
        }
        TopLevelItem::Extern(ex) => {
            for p in &ex.params {
                v.visit_type_expr(&p.ty);
            }
            v.visit_type_expr(&ex.ret_ty);
        }
        TopLevelItem::Statement(s) => v.visit_stmt(s),
    }
}

pub fn walk_stmt<V: Visitor + ?Sized>(v: &mut V, stmt: &Stmt) {
    match stmt {
        Stmt::Let(s) | Stmt::MutLet(s) => {
            if let Some(t) = &s.ty {
                v.visit_type_expr(t);
            }
            v.visit_expr(&s.expr);
        }
        Stmt::Assign(a) => v.visit_expr(&a.expr),
        Stmt::Expr(e) => v.visit_expr(e),
        Stmt::For(f) => {
            v.visit_expr(&f.iter);
            for s in &f.body.stmts {
                v.visit_stmt(s);
            }
            if let Some(e) = &f.body.tail_expr {
                v.visit_expr(e);
            }
        }
        Stmt::While(w) => {
            v.visit_expr(&w.cond);
            for s in &w.body.stmts {
                v.visit_stmt(s);
            }
            if let Some(e) = &w.body.tail_expr {
                v.visit_expr(e);
            }
        }
        Stmt::PureBlock(b) => {
            for s in &b.stmts {
                v.visit_stmt(s);
            }
            if let Some(e) = &b.tail_expr {
                v.visit_expr(e);
            }
        }
    }
}

pub fn walk_expr<V: Visitor + ?Sized>(v: &mut V, expr: &Expr) {
    match expr {
        Expr::Block(b) => {
            for s in &b.stmts {
                v.visit_stmt(s);
            }
            if let Some(e) = &b.tail_expr {
                v.visit_expr(e);
            }
        }
        Expr::If(i) => {
            for (cond, blk) in &i.branches {
                v.visit_expr(cond);
                for s in &blk.stmts {
                    v.visit_stmt(s);
                }
                if let Some(e) = &blk.tail_expr {
                    v.visit_expr(e);
                }
            }
            if let Some(blk) = &i.else_block {
                for s in &blk.stmts {
                    v.visit_stmt(s);
                }
                if let Some(e) = &blk.tail_expr {
                    v.visit_expr(e);
                }
            }
        }
        Expr::Match(m) => {
            v.visit_expr(&m.expr);
            for arm in &m.arms {
                v.visit_expr(&arm.expr);
            }
        }
        Expr::FnCall(c) => {
            for a in &c.args {
                v.visit_expr(a);
            }
        }
        Expr::MethodCall(c) => {
            v.visit_expr(&c.target);
            for a in &c.args {
                v.visit_expr(a);
            }
        }
        Expr::FieldAccess { target, .. } => v.visit_expr(target),
        Expr::Index { target, index, .. } => {
            v.visit_expr(target);
            v.visit_expr(index);
        }
        Expr::Pipeline { left, right, .. } | Expr::Binary { left, right, .. } => {
            v.visit_expr(left);
            v.visit_expr(right);
        }
        Expr::Unary { expr, .. } | Expr::ErrorProp { expr, .. } => v.visit_expr(expr),
        Expr::Closure(c) => {
            if let Some(r) = &c.ret_ty {
                v.visit_type_expr(r);
            }
            for p in &c.params {
                if let Some(t) = &p.ty {
                    v.visit_type_expr(t);
                }
            }
            for s in &c.body.stmts {
                v.visit_stmt(s);
            }
            if let Some(e) = &c.body.tail_expr {
                v.visit_expr(e);
            }
        }
        Expr::RecordLiteral(r) => {
            for (_, e, _) in &r.fields {
                v.visit_expr(e);
            }
        }
        Expr::ListLiteral { elems, .. } | Expr::TupleLiteral { elems, .. } => {
            for e in elems {
                v.visit_expr(e);
            }
        }
        Expr::Return { value, .. } => {
            if let Some(vv) = value {
                v.visit_expr(vv);
            }
        }
        Expr::Literal(_, _) | Expr::Ident(_, _) | Expr::Break { .. } | Expr::Continue { .. } => {}
    }
}

pub fn walk_type_expr<V: Visitor + ?Sized>(v: &mut V, ty: &TypeExpr) {
    match ty {
        TypeExpr::Generic { args, .. } | TypeExpr::Tuple { elems: args, .. } => {
            for t in args {
                v.visit_type_expr(t);
            }
        }
        TypeExpr::Function { params, ret, .. } => {
            for p in params {
                v.visit_type_expr(p);
            }
            v.visit_type_expr(ret);
        }
        TypeExpr::Primitive(_, _) | TypeExpr::Dynamic(_) | TypeExpr::Named { .. } => {}
    }
}
