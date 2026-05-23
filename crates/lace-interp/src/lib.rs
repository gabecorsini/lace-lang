use lace_ast::Program;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeError {
    pub message: String,
}

pub fn run(_program: &Program) -> Result<Value, RuntimeError> {
    Ok(Value::Unit)
}
