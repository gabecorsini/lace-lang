use lace_ast::{EffectExpr, FnDecl, TopLevelItem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectIssue {
    pub function: String,
    pub message: String,
}

pub fn verify_declared_effects(items: &[TopLevelItem]) -> Vec<EffectIssue> {
    let mut issues = Vec::new();
    for item in items {
        if let TopLevelItem::Function(f) = item {
            validate_function_effects(f, &mut issues);
        }
    }
    issues
}

fn validate_function_effects(function: &FnDecl, issues: &mut Vec<EffectIssue>) {
    if function.effects.is_empty() {
        issues.push(EffectIssue {
            function: function.name.clone(),
            message: "missing effect annotation; functions must declare effects".into(),
        });
    }

    let has_time = function
        .effects
        .iter()
        .any(|e| matches!(e, EffectExpr::Builtin(lace_ast::EffectTag::Time)));
    let has_rand = function
        .effects
        .iter()
        .any(|e| matches!(e, EffectExpr::Builtin(lace_ast::EffectTag::Rand)));
    let has_io = function
        .effects
        .iter()
        .any(|e| matches!(e, EffectExpr::Builtin(lace_ast::EffectTag::Io)));

    if (has_time || has_rand) && !has_io {
        issues.push(EffectIssue {
            function: function.name.clone(),
            message: "Time/Rand imply IO; add IO to effect annotation".into(),
        });
    }
}
