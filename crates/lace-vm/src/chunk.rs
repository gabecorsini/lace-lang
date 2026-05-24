use lace_interp::Value;
use serde::{Deserialize, Serialize};

use crate::opcode::OpCode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub name: String,
    pub arity: usize,
    pub is_tool: bool,
    pub code: Vec<OpCode>,
    pub constants: Vec<Value>,
}

impl Chunk {
    pub fn new(name: impl Into<String>, arity: usize, is_tool: bool) -> Self {
        Chunk {
            name: name.into(),
            arity,
            is_tool,
            code: Vec::new(),
            constants: Vec::new(),
        }
    }

    pub fn add_const(&mut self, val: Value) -> usize {
        let idx = self.constants.len();
        self.constants.push(val);
        idx
    }

    pub fn emit(&mut self, op: OpCode) -> usize {
        let idx = self.code.len();
        self.code.push(op);
        idx
    }

    /// Emit a placeholder jump and return its index for backpatching.
    pub fn emit_jump(&mut self, op: OpCode) -> usize {
        self.emit(op)
    }

    /// Patch a previously-emitted jump opcode to point to `target`.
    pub fn patch_jump(&mut self, idx: usize, target: usize) {
        match &mut self.code[idx] {
            OpCode::Jump(ref mut d)
            | OpCode::JumpIfFalse(ref mut d)
            | OpCode::JumpIfTrue(ref mut d) => *d = target,
            _ => panic!("patch_jump called on non-jump opcode"),
        }
    }
}
