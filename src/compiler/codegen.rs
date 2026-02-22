//! Code generation stub — to be implemented in Phase 1 Week 3.

use crate::{ir::Function, trap::Result};

/// Placeholder: in the real implementation, emits native code via Cranelift.
pub fn compile(_func: &Function) -> Result<Vec<u8>> {
    // TODO: translate IR → Cranelift IR → machine code
    Ok(Vec::new())
}
