use std::fmt;

/// All ways execution can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trap {
    OutOfBounds,
    OutOfMemory,
    DivisionByZero,
    Unreachable,
    StackOverflow,
    TypeMismatch,
    UndefinedExport(String),
    UndefinedImport(String),
    InvalidModule(String),
    HostError(String),
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Trap::OutOfBounds       => write!(f, "memory out-of-bounds access"),
            Trap::OutOfMemory       => write!(f, "out of memory"),
            Trap::DivisionByZero    => write!(f, "integer divide by zero"),
            Trap::Unreachable       => write!(f, "unreachable executed"),
            Trap::StackOverflow     => write!(f, "stack overflow"),
            Trap::TypeMismatch      => write!(f, "type mismatch"),
            Trap::UndefinedExport(n) => write!(f, "undefined export: {n}"),
            Trap::UndefinedImport(n) => write!(f, "undefined import: {n}"),
            Trap::InvalidModule(m)  => write!(f, "invalid module: {m}"),
            Trap::HostError(e)      => write!(f, "host error: {e}"),
        }
    }
}

impl std::error::Error for Trap {}

pub type Result<T> = std::result::Result<T, Trap>;
