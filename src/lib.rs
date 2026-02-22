//! Rune — a low-latency, embeddable plugin runtime.
//!
//! # Quick start
//!
//! ```rust
//! use rune::{Module, Runtime, types::{FuncType, Val, ValType}, ir::{Function, Op}};
//!
//! let mut module = Module::new();
//! module.functions.push(Function::new(
//!     "add",
//!     FuncType { params: vec![ValType::I32, ValType::I32], results: vec![ValType::I32] },
//!     vec![],
//!     vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Add, Op::Return],
//! ));
//! module.exports.push(("add".into(), 0));
//!
//! let rt = Runtime::new();
//! let mut inst = rt.instantiate(&module).unwrap();
//! let result = inst.call("add", &[Val::I32(3), Val::I32(4)]).unwrap();
//! assert_eq!(result, Some(Val::I32(7)));
//! ```

pub mod ffi;
pub mod instance;
pub mod ir;
pub mod memory;
pub mod module;
pub mod runtime;
pub mod stack;
pub mod trap;
pub mod types;

pub use instance::Instance;
pub use module::Module;
pub use runtime::Runtime;
pub use trap::{Result, Trap};
pub use types::{FuncType, Val, ValType};
