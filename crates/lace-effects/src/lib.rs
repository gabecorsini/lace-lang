use std::collections::{HashMap, HashSet};

use lace_ast::{
    Block, EffectExpr, EffectTag, Expr, FnDecl, Program, Stmt, ToolDecl, TopLevelItem,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueLevel {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectIssue {
    pub function: String,
    pub level: IssueLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectSet(u8);

impl EffectSet {
    const PURE: u8 = 1 << 0;
    const IO: u8 = 1 << 1;
    const MUT: u8 = 1 << 2;
    const TOOL_CALL: u8 = 1 << 3;
    const TIME: u8 = 1 << 4;
    const RAND: u8 = 1 << 5;

    pub fn empty() -> Self {
        Self(0)
    }

    pub fn insert_tag(&mut self, tag: EffectTag) {
        match tag {
            EffectTag::Pure => self.0 |= Self::PURE,
            EffectTag::Io => self.0 |= Self::IO,
            EffectTag::Mut => self.0 |= Self::MUT,
            EffectTag::ToolCall => {
                self.0 |= Self::TOOL_CALL;
                self.0 |= Self::IO; // ToolCall implies IO
            }
            EffectTag::Time => {
                self.0 |= Self::TIME;
                self.0 |= Self::IO; // Time implies IO
            }
            EffectTag::Rand => {
                self.0 |= Self::RAND;
                self.0 |= Self::IO; // Rand implies IO
            }
        }
    }

    pub fn contains_tag(&self, tag: EffectTag) -> bool {
        let bit = match tag {
            EffectTag::Pure => Self::PURE,
            EffectTag::Io => Self::IO,
            EffectTag::Mut => Self::MUT,
            EffectTag::ToolCall => Self::TOOL_CALL,
            EffectTag::Time => Self::TIME,
            EffectTag::Rand => Self::RAND,
        };
        self.0 & bit != 0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn is_subset_of(self, other: Self) -> bool {
        (self.0 & !other.0) == 0
    }

    pub fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn from_effect_exprs(effects: &[EffectExpr]) -> Self {
        let mut set = Self::empty();
        for eff in effects {
            if let EffectExpr::Builtin(tag) = eff {
                set.insert_tag(*tag);
            }
        }
        set
    }

    pub fn to_names(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.contains_tag(EffectTag::Pure) {
            out.push("Pure");
        }
        if self.contains_tag(EffectTag::Io) {
            out.push("IO");
        }
        if self.contains_tag(EffectTag::Mut) {
            out.push("Mut");
        }
        if self.contains_tag(EffectTag::ToolCall) {
            out.push("ToolCall");
        }
        if self.contains_tag(EffectTag::Time) {
            out.push("Time");
        }
        if self.contains_tag(EffectTag::Rand) {
            out.push("Rand");
        }
        out
    }
}

#[derive(Debug, Default)]
struct Checker<'a> {
    fn_map: HashMap<&'a str, &'a FnDecl>,
    tool_names: HashSet<&'a str>,
    issues: Vec<EffectIssue>,
}

pub fn verify_declared_effects(items: &[TopLevelItem]) -> Vec<EffectIssue> {
    let program = Program {
        module: None,
        uses: Vec::new(),
        items: items.to_vec(),
    };
    check_program(&program)
}

pub fn check_program(program: &Program) -> Vec<EffectIssue> {
    let mut checker = Checker::default();

    for item in &program.items {
        match item {
            TopLevelItem::Function(f) => {
                checker.fn_map.insert(&f.name, f);
            }
            TopLevelItem::Tool(ToolDecl { name, .. }) => {
                checker.tool_names.insert(name);
            }
            _ => {}
        }
    }

    for item in &program.items {
        if let TopLevelItem::Function(f) = item {
            checker.validate_function(f);
        }
    }

    checker.issues
}

impl<'a> Checker<'a> {
    fn validate_function(&mut self, function: &'a FnDecl) {
        if function.effects.is_empty() {
            self.issues.push(EffectIssue {
                function: function.name.clone(),
                level: IssueLevel::Error,
                message: "missing effect annotation; functions must declare effects".into(),
            });
            return;
        }

        let declared = EffectSet::from_effect_exprs(&function.effects);
        let required = self.infer_block_effects(&function.body, function.name.as_str(), false);

        if declared.contains_tag(EffectTag::Pure)
            && (required.contains_tag(EffectTag::Io)
                || required.contains_tag(EffectTag::Mut)
                || required.contains_tag(EffectTag::ToolCall)
                || required.contains_tag(EffectTag::Time)
                || required.contains_tag(EffectTag::Rand))
        {
            self.issues.push(EffectIssue {
                function: function.name.clone(),
                level: IssueLevel::Error,
                message: "function declared [Pure] but body performs side effects".into(),
            });
        }

        if !required.is_subset_of(declared) {
            let missing = required.difference(declared).to_names().join(", ");
            self.issues.push(EffectIssue {
                function: function.name.clone(),
                level: IssueLevel::Error,
                message: format!("underdeclared effects: missing [{}]", missing),
            });
        }

        let overdeclared = declared.difference(required).difference(EffectSet::from_effect_exprs(&[EffectExpr::Builtin(EffectTag::Pure)]));
        if !overdeclared.is_empty() {
            let names = overdeclared.to_names();
            if !names.is_empty() {
                self.issues.push(EffectIssue {
                    function: function.name.clone(),
                    level: IssueLevel::Warning,
                    message: format!("overdeclared effects: declared but unused [{}]", names.join(", ")),
                });
            }
        }
    }

    fn infer_block_effects(&mut self, block: &Block, fn_name: &str, in_pure_block: bool) -> EffectSet {
        let mut out = EffectSet::empty();

        for stmt in &block.stmts {
            out = out.union(self.infer_stmt_effects(stmt, fn_name, in_pure_block));
        }
        if let Some(tail) = &block.tail_expr {
            out = out.union(self.infer_expr_effects(tail, fn_name, in_pure_block));
        }

        out
    }

    fn infer_stmt_effects(&mut self, stmt: &Stmt, fn_name: &str, in_pure_block: bool) -> EffectSet {
        match stmt {
            Stmt::Let(s) | Stmt::MutLet(s) => self.infer_expr_effects(&s.expr, fn_name, in_pure_block),
            Stmt::Assign(s) => self.infer_expr_effects(&s.expr, fn_name, in_pure_block),
            Stmt::Expr(e) => self.infer_expr_effects(e, fn_name, in_pure_block),
            Stmt::For(f) => {
                self.infer_expr_effects(&f.iter, fn_name, in_pure_block)
                    .union(self.infer_block_effects(&f.body, fn_name, in_pure_block))
            }
            Stmt::While(w) => {
                self.infer_expr_effects(&w.cond, fn_name, in_pure_block)
                    .union(self.infer_block_effects(&w.body, fn_name, in_pure_block))
            }
            Stmt::PureBlock(b) => {
                let effects = self.infer_block_effects(b, fn_name, true);
                if effects.contains_tag(EffectTag::Io)
                    || effects.contains_tag(EffectTag::Mut)
                    || effects.contains_tag(EffectTag::ToolCall)
                    || effects.contains_tag(EffectTag::Time)
                    || effects.contains_tag(EffectTag::Rand)
                {
                    self.issues.push(EffectIssue {
                        function: fn_name.to_string(),
                        level: IssueLevel::Error,
                        message: "pure block contains side effects".into(),
                    });
                }
                effects
            }
        }
    }

    fn infer_expr_effects(&mut self, expr: &Expr, fn_name: &str, in_pure_block: bool) -> EffectSet {
        match expr {
            Expr::Literal(_, _) | Expr::Ident(_, _) => EffectSet::empty(),
            Expr::Block(b) => self.infer_block_effects(b, fn_name, in_pure_block),
            Expr::If(i) => {
                let mut out = EffectSet::empty();
                for (cond, blk) in &i.branches {
                    out = out.union(self.infer_expr_effects(cond, fn_name, in_pure_block));
                    out = out.union(self.infer_block_effects(blk, fn_name, in_pure_block));
                }
                if let Some(else_blk) = &i.else_block {
                    out = out.union(self.infer_block_effects(else_blk, fn_name, in_pure_block));
                }
                out
            }
            Expr::Match(m) => {
                let mut out = self.infer_expr_effects(&m.expr, fn_name, in_pure_block);
                for arm in &m.arms {
                    out = out.union(self.infer_expr_effects(&arm.expr, fn_name, in_pure_block));
                }
                out
            }
            Expr::FnCall(call) => {
                let mut out = EffectSet::empty();
                for a in &call.args {
                    out = out.union(self.infer_expr_effects(a, fn_name, in_pure_block));
                }

                // User-defined function call
                if let Some(callee) = self.fn_map.get(call.name.as_str()) {
                    out = out.union(EffectSet::from_effect_exprs(&callee.effects));
                } else if self.tool_names.contains(call.name.as_str()) {
                    // Calling tool declarations carries ToolCall (and therefore IO)
                    out.insert_tag(EffectTag::ToolCall);
                } else if let Some(std_eff) = stdlib_effect_for_name(&call.name) {
                    out.insert_tag(std_eff);
                }

                if in_pure_block
                    && (out.contains_tag(EffectTag::Io)
                        || out.contains_tag(EffectTag::Mut)
                        || out.contains_tag(EffectTag::ToolCall)
                        || out.contains_tag(EffectTag::Time)
                        || out.contains_tag(EffectTag::Rand))
                {
                    self.issues.push(EffectIssue {
                        function: fn_name.to_string(),
                        level: IssueLevel::Error,
                        message: format!(
                            "pure block calls effectful function '{}'",
                            call.name
                        ),
                    });
                }

                out
            }
            Expr::MethodCall(call) => {
                let mut out = self.infer_expr_effects(&call.target, fn_name, in_pure_block);
                for a in &call.args {
                    out = out.union(self.infer_expr_effects(a, fn_name, in_pure_block));
                }
                out
            }
            Expr::FieldAccess { target, .. } => self.infer_expr_effects(target, fn_name, in_pure_block),
            Expr::Index { target, index, .. } => self
                .infer_expr_effects(target, fn_name, in_pure_block)
                .union(self.infer_expr_effects(index, fn_name, in_pure_block)),
            Expr::Pipeline { left, right, .. } | Expr::Binary { left, right, .. } => self
                .infer_expr_effects(left, fn_name, in_pure_block)
                .union(self.infer_expr_effects(right, fn_name, in_pure_block)),
            Expr::Unary { expr, .. } | Expr::ErrorProp { expr, .. } => {
                self.infer_expr_effects(expr, fn_name, in_pure_block)
            }
            Expr::Closure(c) => self.infer_block_effects(&c.body, fn_name, in_pure_block),
            Expr::RecordLiteral(r) => {
                let mut out = EffectSet::empty();
                for (_, e, _) in &r.fields {
                    out = out.union(self.infer_expr_effects(e, fn_name, in_pure_block));
                }
                out
            }
            Expr::ListLiteral { elems, .. } | Expr::TupleLiteral { elems, .. } => {
                let mut out = EffectSet::empty();
                for e in elems {
                    out = out.union(self.infer_expr_effects(e, fn_name, in_pure_block));
                }
                out
            }
            Expr::Return { value, .. } => value
                .as_ref()
                .map(|e| self.infer_expr_effects(e, fn_name, in_pure_block))
                .unwrap_or_else(EffectSet::empty),
        }
    }
}

fn stdlib_effect_for_name(name: &str) -> Option<EffectTag> {
    match name {
        "read_file"
        | "write_file"
        | "append_file"
        | "file_exists"
        | "delete_file"
        | "list_dir"
        | "env_var"
        | "env_var_required"
        | "sleep"
        | "print"
        | "println"
        | "read_line"
        | "context_remaining"
        | "context_used"
        | "context_assert" => Some(EffectTag::Io),
        "now_unix" | "now_millis" => Some(EffectTag::Time),
        "random_float" | "random_int" => Some(EffectTag::Rand),
        _ => None,
    }
}
