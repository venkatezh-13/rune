use crate::types::ValType;
use std::sync::Arc;

/// Block type for control flow ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockType {
    Empty,
    Val(ValType),
}

/// The Rune portable IR instruction set.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    // ── Constants ────────────────────────────────────────────────────────────
    I32Const(i32),
    I64Const(i64),
    F32Const(f32),
    F64Const(f64),

    // ── Stack / Locals ───────────────────────────────────────────────────────
    Drop,
    Select,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),

    // ── Memory ───────────────────────────────────────────────────────────────
    I32Load { align: u32, offset: u32 },
    I32Store { align: u32, offset: u32 },
    I64Load { align: u32, offset: u32 },
    I64Store { align: u32, offset: u32 },
    F32Load { align: u32, offset: u32 },
    F32Store { align: u32, offset: u32 },
    F64Load { align: u32, offset: u32 },
    F64Store { align: u32, offset: u32 },
    MemorySize,
    MemoryGrow,

    // ── i32 arithmetic ───────────────────────────────────────────────────────
    I32Add,
    I32Sub,
    I32Mul,
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrS,
    I32ShrU,
    I32Clz,
    I32Ctz,
    I32Popcnt,
    I32Eqz,

    // ── i64 arithmetic ───────────────────────────────────────────────────────
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64DivU,
    I64RemS,
    I64RemU,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrS,
    I64ShrU,
    I64Eqz,

    // ── f32 arithmetic ───────────────────────────────────────────────────────
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32Sqrt,
    F32Min,
    F32Max,
    F32Abs,
    F32Neg,
    F32Ceil,
    F32Floor,

    // ── f64 arithmetic ───────────────────────────────────────────────────────
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Sqrt,
    F64Min,
    F64Max,
    F64Abs,
    F64Neg,
    F64Ceil,
    F64Floor,

    // ── Comparisons ──────────────────────────────────────────────────────────
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtS,
    I32GtU,
    I32LeS,
    I32LeU,
    I32GeS,
    I32GeU,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LtU,
    I64GtS,
    I64GtU,
    I64LeS,
    I64LeU,
    I64GeS,
    I64GeU,
    F32Eq,
    F32Ne,
    F32Lt,
    F32Gt,
    F32Le,
    F32Ge,
    F64Eq,
    F64Ne,
    F64Lt,
    F64Gt,
    F64Le,
    F64Ge,

    // ── Conversions ──────────────────────────────────────────────────────────
    I32WrapI64,
    I64ExtendI32S,
    I64ExtendI32U,
    F32ConvertI32S,
    F32ConvertI32U,
    F64ConvertI32S,
    F64ConvertI32U,
    F64ConvertI64S,
    F64ConvertI64U,
    I32TruncF32S,
    I32TruncF32U,
    I32TruncF64S,
    I32TruncF64U,
    F32DemoteF64,
    F64PromoteF32,
    I32ReinterpretF32,
    F32ReinterpretI32,
    I64ReinterpretF64,
    F64ReinterpretI64,

    // ── Control flow ─────────────────────────────────────────────────────────
    Nop,
    Unreachable,
    Block(BlockType),
    Loop(BlockType),
    If(BlockType),
    Else,
    End,
    Br(u32),
    BrIf(u32),
    Return,

    // ── Calls ────────────────────────────────────────────────────────────────
    Call(u32),     // Index into module's function list
    CallHost(u32), // Index into module's import list
}

/// A compiled function (sequence of ops + metadata).
///
/// `body` is wrapped in `Arc` so that cloning a Function (e.g. when passing
/// it to a recursive call frame) is a single atomic increment — not a full
/// Vec copy. This eliminates the dominant allocation in recursive workloads.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub ty: crate::types::FuncType,
    pub locals: Vec<ValType>, // extra locals beyond params
    pub body: Arc<Vec<Op>>,
}

impl Function {
    /// Convenience constructor that wraps the body in an Arc.
    pub fn new(
        name: impl Into<String>,
        ty: crate::types::FuncType,
        locals: Vec<ValType>,
        body: Vec<Op>,
    ) -> Self {
        Function {
            name: name.into(),
            ty,
            locals,
            body: Arc::new(body),
        }
    }
}
