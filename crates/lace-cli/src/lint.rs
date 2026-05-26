//! Source-level lint pass for Lace programs.
//!
//! Checks for common code quality issues:
//! - L001: unused `let` bindings
//! - L002: functions with no effect annotation
//! - L003: shadowed variables

use lace_ast::{
    Block, ClosureExpr, Expr, FnDecl, MatchArm, Pattern, Program, Stmt, TopLevelItem,
};

/// A single lint warning emitted by the lint pass.
#[derive(Debug, Clone)]
pub struct LintWarning {
    pub rule: &'static str,
    pub message: String,
}

/// Run all lint rules over a parsed program and return collected warnings.
pub fn lint_program(program: &Program, _source: &str) -> Vec<LintWarning> {
    let mut warnings = Vec::new();
    for item in &program.items {
        if let TopLevelItem::Function(f) = item {
            lint_fn(f, &mut warnings);
        }
    }
    warnings
}

fn lint_fn(f: &FnDecl, warnings: &mut Vec<LintWarning>) {
    // L002: no effect annotation on non-trivial functions
    if f.effects.is_empty() {
        warnings.push(LintWarning {
            rule: "L002",
            message: format!(
                "function '{}' has no effect annotation (consider adding [Pure] or [IO])",
                f.name
            ),
        });
    }

    // Walk function body for L001/L003
    let mut scope_stack: Vec<std::collections::HashMap<String, (u32, bool)>> = Vec::new();
    scope_stack.push(std::collections::HashMap::new());

    // Register parameters as used (they are named, not let-bindings)
    for p in &f.params {
        if let Some(top) = scope_stack.last_mut() {
            top.insert(p.name.clone(), (0, true)); // params are always "used"
        }
    }

    lint_block(&f.body, &mut scope_stack, warnings);

    // Pop top scope and check for unused
    check_unused_in_scope(scope_stack.pop(), warnings);
}

fn lint_block(
    block: &Block,
    scope_stack: &mut Vec<std::collections::HashMap<String, (u32, bool)>>,
    warnings: &mut Vec<LintWarning>,
) {
    scope_stack.push(std::collections::HashMap::new());

    for stmt in &block.stmts {
        lint_stmt(stmt, scope_stack, warnings);
    }
    if let Some(tail) = &block.tail_expr {
        lint_expr(tail, scope_stack, warnings);
    }

    check_unused_in_scope(scope_stack.pop(), warnings);
}

fn lint_stmt(
    stmt: &Stmt,
    scope_stack: &mut Vec<std::collections::HashMap<String, (u32, bool)>>,
    warnings: &mut Vec<LintWarning>,
) {
    match stmt {
        Stmt::Let(s) | Stmt::MutLet(s) => {
            lint_expr(&s.expr, scope_stack, warnings);
            let line = 0u32; // line tracking would need span→line resolution
            // L003: shadowed variable
            for scope in scope_stack.iter().rev() {
                if scope.contains_key(&s.name) {
                    warnings.push(LintWarning {
                        rule: "L003",
                        message: format!(
                            "variable '{}' shadows an outer binding",
                            s.name
                        ),
                    });
                    break;
                }
            }
            // Register binding as unused
            if let Some(top) = scope_stack.last_mut() {
                top.insert(s.name.clone(), (line, false));
            }
        }
        Stmt::Assign(s) => {
            lint_expr(&s.expr, scope_stack, warnings);
        }
        Stmt::Expr(e) => {
            lint_expr(e, scope_stack, warnings);
        }
        Stmt::For(s) => {
            lint_expr(&s.iter, scope_stack, warnings);
            // loop variable
            scope_stack.push(std::collections::HashMap::new());
            if let Some(top) = scope_stack.last_mut() {
                top.insert(s.name.clone(), (0, true)); // for-var considered used
            }
            lint_block(&s.body, scope_stack, warnings);
            scope_stack.pop();
        }
        Stmt::While(s) => {
            lint_expr(&s.cond, scope_stack, warnings);
            lint_block(&s.body, scope_stack, warnings);
        }
        Stmt::PureBlock(b) => {
            lint_block(b, scope_stack, warnings);
        }

    }
}

fn lint_expr(
    expr: &Expr,
    scope_stack: &mut Vec<std::collections::HashMap<String, (u32, bool)>>,
    warnings: &mut Vec<LintWarning>,
) {
    match expr {
        Expr::Ident(name, _) => {
            // Mark as used in nearest enclosing scope
            for scope in scope_stack.iter_mut().rev() {
                if let Some(entry) = scope.get_mut(name.as_str()) {
                    entry.1 = true;
                    break;
                }
            }
        }
        Expr::Binary { left, right, .. } => {
            lint_expr(left, scope_stack, warnings);
            lint_expr(right, scope_stack, warnings);
        }
        Expr::Unary { expr, .. } => {
            lint_expr(expr, scope_stack, warnings);
        }
        Expr::FnCall(call) => {
            for arg in &call.args {
                lint_expr(arg, scope_stack, warnings);
            }
        }
        Expr::MethodCall(call) => {
            lint_expr(&call.target, scope_stack, warnings);
            for arg in &call.args {
                lint_expr(arg, scope_stack, warnings);
            }
        }
        Expr::FieldAccess { target, .. } => {
            lint_expr(target, scope_stack, warnings);
        }
        Expr::Index { target, index, .. } => {
            lint_expr(target, scope_stack, warnings);
            lint_expr(index, scope_stack, warnings);
        }
        Expr::Pipeline { left, right, .. } => {
            lint_expr(left, scope_stack, warnings);
            lint_expr(right, scope_stack, warnings);
        }
        Expr::Block(b) => {
            lint_block(b, scope_stack, warnings);
        }
        Expr::If(if_expr) => {
            for (cond, body) in &if_expr.branches {
                lint_expr(cond, scope_stack, warnings);
                lint_block(body, scope_stack, warnings);
            }
            if let Some(else_b) = &if_expr.else_block {
                lint_block(else_b, scope_stack, warnings);
            }
        }
        Expr::Match(match_expr) => {
            lint_expr(&match_expr.expr, scope_stack, warnings);
            for arm in &match_expr.arms {
                lint_match_arm(arm, scope_stack, warnings);
            }
        }
        Expr::Closure(c) => {
            lint_closure(c, scope_stack, warnings);
        }
        Expr::Return { value, .. } => {
            if let Some(v) = value {
                lint_expr(v, scope_stack, warnings);
            }
        }
        Expr::ErrorProp { expr, .. } => {
            lint_expr(expr, scope_stack, warnings);
        }
        Expr::ListLiteral { elems, .. } | Expr::TupleLiteral { elems, .. } => {
            for e in elems {
                lint_expr(e, scope_stack, warnings);
            }
        }
        Expr::MapLiteral { pairs, .. } => {
            for (k, v) in pairs {
                lint_expr(k, scope_stack, warnings);
                lint_expr(v, scope_stack, warnings);
            }
        }
        Expr::RecordLiteral(r) => {
            for (_, val, _) in &r.fields {
                lint_expr(val, scope_stack, warnings);
            }
        }
        // Literals and control flow with no sub-expressions
        Expr::Literal(..) | Expr::Break { .. } | Expr::Continue { .. } => {}
    }
}

fn lint_match_arm(
    arm: &MatchArm,
    scope_stack: &mut Vec<std::collections::HashMap<String, (u32, bool)>>,
    warnings: &mut Vec<LintWarning>,
) {
    scope_stack.push(std::collections::HashMap::new());
    collect_pattern_bindings(&arm.pattern, scope_stack);
    lint_expr(&arm.expr, scope_stack, warnings);
    check_unused_in_scope(scope_stack.pop(), warnings);
}

fn lint_closure(
    c: &ClosureExpr,
    scope_stack: &mut Vec<std::collections::HashMap<String, (u32, bool)>>,
    warnings: &mut Vec<LintWarning>,
) {
    scope_stack.push(std::collections::HashMap::new());
    for p in &c.params {
        if let Some(top) = scope_stack.last_mut() {
            top.insert(p.name.clone(), (0, true)); // closure params: always "used"
        }
    }
    lint_block(&c.body, scope_stack, warnings);
    scope_stack.pop();
}

fn collect_pattern_bindings(
    pat: &Pattern,
    scope_stack: &mut Vec<std::collections::HashMap<String, (u32, bool)>>,
) {
    match pat {
        Pattern::Ident(name, _) => {
            if let Some(top) = scope_stack.last_mut() {
                top.insert(name.clone(), (0, true)); // pattern bindings considered used
            }
        }
        Pattern::Tuple(pats, _) => {
            for p in pats {
                collect_pattern_bindings(p, scope_stack);
            }
        }
        Pattern::EnumTuple { elems, .. } => {
            for p in elems {
                collect_pattern_bindings(p, scope_stack);
            }
        }
        Pattern::EnumStruct { fields, .. } | Pattern::Record { fields, .. } => {
            for (_, p) in fields {
                collect_pattern_bindings(p, scope_stack);
            }
        }
        _ => {}
    }
}

fn check_unused_in_scope(
    scope: Option<std::collections::HashMap<String, (u32, bool)>>,
    warnings: &mut Vec<LintWarning>,
) {
    if let Some(scope) = scope {
        for (name, (_line, used)) in &scope {
            if !used && !name.starts_with('_') {
                warnings.push(LintWarning {
                    rule: "L001",
                    message: format!("unused variable '{name}'"),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lace_parser::parse_program;

    #[test]
    fn test_lint_unused_variable() {
        let src = "fn foo() [Pure] {\n  let x = 1\n  42\n}\n";
        let (program, _) = parse_program(src);
        let warnings = lint_program(&program.unwrap(), src);
        assert!(
            warnings.iter().any(|w| w.rule == "L001" && w.message.contains("'x'")),
            "Expected L001 for unused 'x', got: {:?}",
            warnings
        );
    }

    #[test]
    fn test_lint_no_effect_annotation() {
        let src = "fn bar() {\n  1\n}\n";
        let (program, _) = parse_program(src);
        let warnings = lint_program(&program.unwrap(), src);
        assert!(
            warnings.iter().any(|w| w.rule == "L002" && w.message.contains("'bar'")),
            "Expected L002 for 'bar', got: {:?}",
            warnings
        );
    }

    #[test]
    fn test_lint_clean_code_no_warnings() {
        let src = "fn add(a: Int, b: Int) -> Int [Pure] {\n  a + b\n}\n";
        let (program, _) = parse_program(src);
        let program = program.expect("should parse without errors");
        let warnings = lint_program(&program, src);
        // should have no L001/L003
        let bad: Vec<_> = warnings.iter().filter(|w| w.rule == "L001" || w.rule == "L003").collect();
        assert!(bad.is_empty(), "Expected no unused/shadow warnings, got: {:?}", bad);
    }
}
