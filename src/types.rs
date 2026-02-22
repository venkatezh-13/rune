/// Primitive value types supported by Rune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ValType {
    I32 = 0x7F,
    I64 = 0x7E,
    F32 = 0x7D,
    F64 = 0x7C,
}

impl ValType {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            0x7F => Some(ValType::I32),
            0x7E => Some(ValType::I64),
            0x7D => Some(ValType::F32),
            0x7C => Some(ValType::F64),
            _ => None,
        }
    }
}

/// Function signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncType {
    pub params: Vec<ValType>,
    /// MVP: at most 1 result.
    pub results: Vec<ValType>,
}

/// A runtime value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Val {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl Val {
    pub fn ty(&self) -> ValType {
        match self {
            Val::I32(_) => ValType::I32,
            Val::I64(_) => ValType::I64,
            Val::F32(_) => ValType::F32,
            Val::F64(_) => ValType::F64,
        }
    }

    pub fn as_i32(self) -> Option<i32> {
        if let Val::I32(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_i64(self) -> Option<i64> {
        if let Val::I64(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_f32(self) -> Option<f32> {
        if let Val::F32(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_f64(self) -> Option<f64> {
        if let Val::F64(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn default_for(ty: ValType) -> Val {
        match ty {
            ValType::I32 => Val::I32(0),
            ValType::I64 => Val::I64(0),
            ValType::F32 => Val::F32(0.0),
            ValType::F64 => Val::F64(0.0),
        }
    }
}
