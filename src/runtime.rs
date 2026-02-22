use crate::{
    instance::Instance,
    module::Module,
    trap::Result,
};

/// Top-level runtime context. Currently lightweight; reserve for future
/// shared resources (fuel budgets, JIT caches, etc.).
pub struct Runtime {
    _priv: (),
}

impl Runtime {
    pub fn new() -> Self {
        Runtime { _priv: () }
    }

    /// Instantiate a module, applying data segments and wiring host functions.
    pub fn instantiate<'m>(&self, module: &'m Module) -> Result<Instance<'m>> {
        Instance::new(module)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}
