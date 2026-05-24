use std::fmt;

#[derive(Debug, Clone)]
pub enum VmError {
    RuntimeError(String),
    DivisionByZero,
    StackUnderflow,
    UndefinedVariable(String),
    TypeError(String),
    CompileError(String),
}

impl fmt::Display for VmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmError::RuntimeError(s) => write!(f, "RuntimeError: {}", s),
            VmError::DivisionByZero => write!(f, "DivisionByZero"),
            VmError::StackUnderflow => write!(f, "StackUnderflow"),
            VmError::UndefinedVariable(s) => write!(f, "UndefinedVariable: {}", s),
            VmError::TypeError(s) => write!(f, "TypeError: {}", s),
            VmError::CompileError(s) => write!(f, "CompileError: {}", s),
        }
    }
}

impl std::error::Error for VmError {}
