pub mod opcode;
pub mod chunk;
pub mod error;
pub mod compiler;
pub mod vm;

pub use error::VmError;
pub use chunk::Chunk;

use vm::Vm;

fn decl_keyword(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("fn ")
        || t.starts_with("tool ")
        || t.starts_with("record ")
        || t.starts_with("enum ")
        || t.starts_with("const ")
        || t.starts_with("type ")
        || t.starts_with("extern ")
        || t.starts_with("module ")
        || t.starts_with("use ")
        || t.starts_with("import ")
}

/// Wrap bare source code in a `fn main` block.
/// - If source already contains only declarations (all non-empty lines start with a
///   declaration keyword or are inside a block), return as-is.
/// - If source contains no declarations (purely statements), wrap all in main.
/// - If mixed (e.g. fn definitions + bare statements), split: keep declarations at top
///   level, collect bare statements into a main fn appended at the end.
fn wrap_in_main(source: &str) -> String {
    // Simple heuristic: if every non-empty, non-comment line at depth 0 is a decl
    // keyword, it's already structured.
    let trimmed = source.trim();
    
    // Check if there's already an explicit `fn main`
    if trimmed.contains("fn main") {
        return source.to_string();
    }

    // Split into declaration lines vs bare statement lines at top level (depth 0)
    let mut decl_lines: Vec<&str> = Vec::new();
    let mut stmt_lines: Vec<&str> = Vec::new();
    let mut depth: usize = 0;
    let mut in_decl_block = false;

    for line in source.lines() {
        let open = line.chars().filter(|&c| c == '{').count();
        let close = line.chars().filter(|&c| c == '}').count();
        
        if depth == 0 {
            if decl_keyword(line) {
                in_decl_block = true;
                decl_lines.push(line);
            } else if line.trim().is_empty() {
                if in_decl_block {
                    decl_lines.push(line);
                } else {
                    stmt_lines.push(line);
                }
            } else {
                in_decl_block = false;
                stmt_lines.push(line);
            }
        } else {
            if in_decl_block {
                decl_lines.push(line);
            } else {
                stmt_lines.push(line);
            }
        }

        depth = depth.saturating_add(open).saturating_sub(close);
        if depth == 0 {
            in_decl_block = false;
        }
    }

    let has_stmts = stmt_lines.iter().any(|l| !l.trim().is_empty());
    let has_decls = decl_lines.iter().any(|l| !l.trim().is_empty());

    if !has_stmts {
        // Only declarations — return as-is
        source.to_string()
    } else if !has_decls {
        // Only statements — wrap all in main with IO effect
        format!("fn main() -> Unit [IO] {{\n{}\n}}\n", source)
    } else {
        // Mixed — keep decls, wrap stmts in main
        let decl_part = decl_lines.join("\n");
        let stmt_part = stmt_lines.join("\n");
        format!("{}\n\nfn main() -> Unit [IO] {{\n{}\n}}\n", decl_part, stmt_part)
    }
}

pub fn run_source(source: &str, tool_log: bool) -> Result<(), VmError> {
    let wrapped = wrap_in_main(source);
    let chunks = compiler::compile_program(&wrapped)?;
    let mut vm = Vm::new(chunks, tool_log);
    vm.run()?;
    Ok(())
}

pub fn compile_to_bytes(source: &str) -> Result<Vec<u8>, VmError> {
    let wrapped = wrap_in_main(source);
    let chunks = compiler::compile_program(&wrapped)?;
    let magic: [u8; 4] = [0x4C, 0x41, 0x43, 0x45];
    let version: u32 = 1;
    let encoded = bincode::serialize(&chunks)
        .map_err(|e| VmError::RuntimeError(format!("serialize error: {}", e)))?;
    let mut out = Vec::new();
    out.extend_from_slice(&magic);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend(encoded);
    Ok(out)
}

pub fn run_bytes(bytes: &[u8], tool_log: bool) -> Result<(), VmError> {
    if bytes.len() < 8 {
        return Err(VmError::RuntimeError("bytecode too short".into()));
    }
    let magic = &bytes[0..4];
    if magic != [0x4C, 0x41, 0x43, 0x45] {
        return Err(VmError::RuntimeError("invalid magic bytes".into()));
    }
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 1 {
        return Err(VmError::RuntimeError(format!("unsupported bytecode version {}", version)));
    }
    let chunks: Vec<Chunk> = bincode::deserialize(&bytes[8..])
        .map_err(|e| VmError::RuntimeError(format!("deserialize error: {}", e)))?;
    let mut vm = Vm::new(chunks, tool_log);
    vm.run()?;
    Ok(())
}
