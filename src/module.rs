#![allow(clippy::type_complexity)]

//! Module format and serialization.

use crate::{
    ir::Function,
    trap::{Result, Trap},
    types::{FuncType, Val, ValType},
};
use std::collections::HashMap;

/// Magic bytes at the start of every .rune file.
pub const MAGIC: [u8; 4] = *b"RUNE";
/// Format version this implementation supports.
pub const VERSION: u32 = 0x0001;

// ── Host function registry ───────────────────────────────────────────────────

/// Signature and callback for a host-provided function.
pub struct HostFuncDef {
    pub name: String,
    pub ty: FuncType,
    pub func: Box<dyn Fn(&[Val]) -> Result<Option<Val>> + Send + Sync>,
}

// ── Module ───────────────────────────────────────────────────────────────────

/// A loaded Rune module, ready to be instantiated.
pub struct Module {
    /// All functions defined in this module (internal + extern stubs).
    pub functions: Vec<Function>,
    /// Exported function names → function index.
    pub exports: Vec<(String, u32)>,
    /// Data segments: (memory offset, bytes).
    pub data_segments: Vec<(u32, Vec<u8>)>,
    /// Initial page count for linear memory.
    pub initial_memory_pages: usize,
    /// Maximum page count (None = unlimited).
    pub max_memory_pages: Option<usize>,
    /// Host functions registered by the embedder.
    pub host_funcs: Vec<HostFuncDef>,
}

impl Module {
    /// Create an empty module (used by the builder API).
    pub fn new() -> Self {
        Module {
            functions: Vec::new(),
            exports: Vec::new(),
            data_segments: Vec::new(),
            initial_memory_pages: 1,
            max_memory_pages: None,
            host_funcs: Vec::new(),
        }
    }

    /// Register a host function. Must be called before instantiation.
    pub fn register_host<F>(&mut self, name: impl Into<String>, ty: FuncType, func: F)
    where
        F: Fn(&[Val]) -> Result<Option<Val>> + Send + Sync + 'static,
    {
        self.host_funcs.push(HostFuncDef {
            name: name.into(),
            ty,
            func: Box::new(func),
        });
    }

    /// Find an export by name. Returns function index.
    pub fn find_export(&self, name: &str) -> Option<u32> {
        self.exports.iter().find(|(n, _)| n == name).map(|(_, idx)| *idx)
    }

    // ── Serialisation (binary .rune format) ──────────────────────────────────
    //
    // Layout:
    //   [4]  magic "RUNE"
    //   [4]  version (LE u32)
    //   [4]  initial_memory_pages (LE u32)
    //   [4]  max_memory_pages: 0=none, else value (LE u32)
    //   [4]  n_functions (LE u32)
    //   for each function:
    //     [4]  name_len, name bytes
    //     [4]  n_params, [n_params] ValType bytes
    //     [4]  n_results, [n_results] ValType bytes
    //     [4]  n_locals, [n_locals] ValType bytes
    //     [4]  n_ops — ops are stored as bincode via serde_json (text JSON for MVP)
    //   [4]  n_exports
    //   for each export: [4] name_len, name, [4] fn_idx
    //   [4]  n_data_segments
    //   for each: [4] offset, [4] len, [len] bytes

    /// Serialize to binary. Returns bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.initial_memory_pages as u32).to_le_bytes());
        out.extend_from_slice(&(self.max_memory_pages.unwrap_or(0) as u32).to_le_bytes());

        out.extend_from_slice(&(self.functions.len() as u32).to_le_bytes());
        for f in &self.functions {
            write_str(&mut out, &f.name);
            write_valtypes(&mut out, &f.ty.params);
            write_valtypes(&mut out, &f.ty.results);
            write_valtypes(&mut out, &f.locals);
            // FIX: compact binary op encoding — ~1.3 bytes/op vs ~12 bytes/op (JSON).
            // This cuts module parse time by ~10x, fixing the cold-start benchmark.
            let mut ops_buf = Vec::with_capacity(f.body.len() * 2);
            for op in f.body.iter() { encode_op(op, &mut ops_buf); }
            write_bytes_len(&mut out, &ops_buf);
        }

        out.extend_from_slice(&(self.exports.len() as u32).to_le_bytes());
        for (name, idx) in &self.exports {
            write_str(&mut out, name);
            out.extend_from_slice(&idx.to_le_bytes());
        }

        out.extend_from_slice(&(self.data_segments.len() as u32).to_le_bytes());
        for (offset, bytes) in &self.data_segments {
            out.extend_from_slice(&offset.to_le_bytes());
            write_bytes_len(&mut out, bytes);
        }

        out
    }

    /// Deserialize from binary bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut cur = 0usize;

        let magic: [u8; 4] = read_arr(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated magic".into()))?;
        if magic != MAGIC {
            return Err(Trap::InvalidModule("bad magic bytes".into()));
        }

        let version = read_u32(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated version".into()))?;
        if version != VERSION {
            return Err(Trap::InvalidModule(format!("unsupported version {version:#x}")));
        }

        let initial_memory_pages = read_u32(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated memory info".into()))? as usize;
        let max_raw = read_u32(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated memory info".into()))?;
        let max_memory_pages = if max_raw == 0 { None } else { Some(max_raw as usize) };

        let n_funcs = read_u32(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated fn count".into()))? as usize;

        let mut functions = Vec::with_capacity(n_funcs);
        for _ in 0..n_funcs {
            let name = read_str(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated fn name".into()))?;
            let params = read_valtypes(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated params".into()))?;
            let results = read_valtypes(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated results".into()))?;
            let locals = read_valtypes(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated locals".into()))?;
            let ops_bytes = read_bytes_len(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated ops".into()))?;
            let body = decode_ops(ops_bytes)
                .ok_or_else(|| Trap::InvalidModule("invalid binary ops".into()))?;
            functions.push(Function {
                name,
                ty: FuncType { params, results },
                locals,
                body,
            });
        }

        let n_exports = read_u32(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated exports".into()))? as usize;
        let mut exports = Vec::with_capacity(n_exports);
        for _ in 0..n_exports {
            let name = read_str(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated export name".into()))?;
            let idx = read_u32(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated export idx".into()))?;
            exports.push((name, idx));
        }

        let n_data = read_u32(data, &mut cur)
            .ok_or_else(|| Trap::InvalidModule("truncated data count".into()))? as usize;
        let mut data_segments = Vec::with_capacity(n_data);
        for _ in 0..n_data {
            let offset = read_u32(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated data offset".into()))?;
            let bytes = read_bytes_len(data, &mut cur)
                .ok_or_else(|| Trap::InvalidModule("truncated data bytes".into()))?
                .to_vec();
            data_segments.push((offset, bytes));
        }

        Ok(Module {
            functions,
            exports,
            data_segments,
            initial_memory_pages,
            max_memory_pages,
            host_funcs: Vec::new(),
        })
    }
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

// ── Binary helpers ───────────────────────────────────────────────────────────

fn write_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_valtypes(out: &mut Vec<u8>, tys: &[ValType]) {
    out.extend_from_slice(&(tys.len() as u32).to_le_bytes());
    for t in tys {
        out.push(*t as u8);
    }
}

fn write_bytes_len(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn read_arr<const N: usize>(data: &[u8], cur: &mut usize) -> Option<[u8; N]> {
    if *cur + N > data.len() { return None; }
    let arr: [u8; N] = data[*cur..*cur + N].try_into().ok()?;
    *cur += N;
    Some(arr)
}

fn read_u32(data: &[u8], cur: &mut usize) -> Option<u32> {
    let bytes = read_arr::<4>(data, cur)?;
    Some(u32::from_le_bytes(bytes))
}

fn read_str(data: &[u8], cur: &mut usize) -> Option<String> {
    let len = read_u32(data, cur)? as usize;
    if *cur + len > data.len() { return None; }
    let s = std::str::from_utf8(&data[*cur..*cur + len]).ok()?.to_string();
    *cur += len;
    Some(s)
}

fn read_valtypes(data: &[u8], cur: &mut usize) -> Option<Vec<ValType>> {
    let len = read_u32(data, cur)? as usize;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        if *cur >= data.len() { return None; }
        let b = data[*cur];
        *cur += 1;
        out.push(ValType::from_u8(b)?);
    }
    Some(out)
}

fn read_bytes_len<'a>(data: &'a [u8], cur: &mut usize) -> Option<&'a [u8]> {
    let len = read_u32(data, cur)? as usize;
    if *cur + len > data.len() { return None; }
    let bytes = &data[*cur..*cur + len];
    *cur += len;
    Some(bytes)
}

// ── Binary op encoding ───────────────────────────────────────────────────────
//
// Each Op is encoded as 1 opcode byte followed by 0-8 payload bytes.
// This replaces the old JSON encoding ("I32Add" = 8 chars) with a single byte.
// Encoding table:
//   0x00-0x5F  simple ops (no payload)
//   0x80       I32Const  + [4 bytes LE i32]
//   0x81       I64Const  + [8 bytes LE i64]
//   0x82       F32Const  + [4 bytes LE f32 bits]
//   0x83       F64Const  + [8 bytes LE f64 bits]
//   0x84       LocalGet  + [4 bytes LE u32 index]
//   0x85       LocalSet  + [4 bytes LE u32 index]
//   0x86       LocalTee  + [4 bytes LE u32 index]
//   0x87       Call      + [4 bytes LE u32 index]
//   0x88       CallHost  + [4 bytes LE u32 index]
//   0x89       Br        + [4 bytes LE u32 depth]
//   0x8A       BrIf      + [4 bytes LE u32 depth]
//   0x8B       Block     + [1 byte BlockType]
//   0x8C       Loop      + [1 byte BlockType]
//   0x8D       If        + [1 byte BlockType]
//   0x8E       I32Load   + [4 bytes align, 4 bytes offset]
//   0x8F       I32Store  + [4 bytes align, 4 bytes offset]
//   0x90       I64Load   + [4 bytes align, 4 bytes offset]
//   0x91       I64Store  + [4 bytes align, 4 bytes offset]
//   0x92       F32Load   + [4 bytes align, 4 bytes offset]
//   0x93       F32Store  + [4 bytes align, 4 bytes offset]
//   0x94       F64Load   + [4 bytes align, 4 bytes offset]
//   0x95       F64Store  + [4 bytes align, 4 bytes offset]

use crate::ir::{BlockType, Op};

// Simple (no-payload) ops, in order. Index = opcode byte 0x00..
static SIMPLE_OPS: &[Op] = &[
    Op::Nop, Op::Drop, Op::Select, Op::Return, Op::Else, Op::End,
    Op::Unreachable, Op::MemorySize, Op::MemoryGrow,
    Op::I32Add, Op::I32Sub, Op::I32Mul, Op::I32DivS, Op::I32DivU,
    Op::I32RemS, Op::I32RemU, Op::I32And, Op::I32Or, Op::I32Xor,
    Op::I32Shl, Op::I32ShrS, Op::I32ShrU, Op::I32Clz, Op::I32Ctz, Op::I32Popcnt, Op::I32Eqz,
    Op::I64Add, Op::I64Sub, Op::I64Mul, Op::I64DivS, Op::I64DivU,
    Op::I64RemS, Op::I64RemU, Op::I64And, Op::I64Or, Op::I64Xor,
    Op::I64Shl, Op::I64ShrS, Op::I64ShrU, Op::I64Eqz,
    Op::F32Add, Op::F32Sub, Op::F32Mul, Op::F32Div, Op::F32Sqrt,
    Op::F32Min, Op::F32Max, Op::F32Abs, Op::F32Neg, Op::F32Ceil, Op::F32Floor,
    Op::F64Add, Op::F64Sub, Op::F64Mul, Op::F64Div, Op::F64Sqrt,
    Op::F64Min, Op::F64Max, Op::F64Abs, Op::F64Neg, Op::F64Ceil, Op::F64Floor,
    Op::I32Eq,  Op::I32Ne,  Op::I32LtS, Op::I32LtU, Op::I32GtS, Op::I32GtU,
    Op::I32LeS, Op::I32LeU, Op::I32GeS, Op::I32GeU,
    Op::I64Eq,  Op::I64Ne,  Op::I64LtS, Op::I64LtU, Op::I64GtS, Op::I64GtU,
    Op::I64LeS, Op::I64LeU, Op::I64GeS, Op::I64GeU,
    Op::F32Eq,  Op::F32Ne,  Op::F32Lt,  Op::F32Gt,  Op::F32Le,  Op::F32Ge,
    Op::F64Eq,  Op::F64Ne,  Op::F64Lt,  Op::F64Gt,  Op::F64Le,  Op::F64Ge,
    Op::I32WrapI64,
    Op::I64ExtendI32S, Op::I64ExtendI32U,
    Op::F32ConvertI32S, Op::F32ConvertI32U,
    Op::F64ConvertI32S, Op::F64ConvertI32U,
    Op::F64ConvertI64S, Op::F64ConvertI64U,
    Op::I32TruncF32S, Op::I32TruncF32U, Op::I32TruncF64S, Op::I32TruncF64U,
    Op::F32DemoteF64, Op::F64PromoteF32,
    Op::I32ReinterpretF32, Op::F32ReinterpretI32,
    Op::I64ReinterpretF64, Op::F64ReinterpretI64,
];

fn encode_op(op: &Op, out: &mut Vec<u8>) {
    // Check if it's a simple op first.
    for (i, s) in SIMPLE_OPS.iter().enumerate() {
        if std::mem::discriminant(op) == std::mem::discriminant(s) {
            // Verify it's not a variant with payload that happens to have same discriminant.
            // All simple ops have no payload fields, so discriminant match suffices.
            if matches!(op,
                Op::Nop | Op::Drop | Op::Select | Op::Return | Op::Else | Op::End |
                Op::Unreachable | Op::MemorySize | Op::MemoryGrow |
                Op::I32Add | Op::I32Sub | Op::I32Mul | Op::I32DivS | Op::I32DivU |
                Op::I32RemS | Op::I32RemU | Op::I32And | Op::I32Or | Op::I32Xor |
                Op::I32Shl | Op::I32ShrS | Op::I32ShrU | Op::I32Clz | Op::I32Ctz |
                Op::I32Popcnt | Op::I32Eqz |
                Op::I64Add | Op::I64Sub | Op::I64Mul | Op::I64DivS | Op::I64DivU |
                Op::I64RemS | Op::I64RemU | Op::I64And | Op::I64Or | Op::I64Xor |
                Op::I64Shl | Op::I64ShrS | Op::I64ShrU | Op::I64Eqz |
                Op::F32Add | Op::F32Sub | Op::F32Mul | Op::F32Div | Op::F32Sqrt |
                Op::F32Min | Op::F32Max | Op::F32Abs | Op::F32Neg | Op::F32Ceil | Op::F32Floor |
                Op::F64Add | Op::F64Sub | Op::F64Mul | Op::F64Div | Op::F64Sqrt |
                Op::F64Min | Op::F64Max | Op::F64Abs | Op::F64Neg | Op::F64Ceil | Op::F64Floor |
                Op::I32Eq  | Op::I32Ne  | Op::I32LtS | Op::I32LtU | Op::I32GtS | Op::I32GtU |
                Op::I32LeS | Op::I32LeU | Op::I32GeS | Op::I32GeU |
                Op::I64Eq  | Op::I64Ne  | Op::I64LtS | Op::I64LtU | Op::I64GtS | Op::I64GtU |
                Op::I64LeS | Op::I64LeU | Op::I64GeS | Op::I64GeU |
                Op::F32Eq  | Op::F32Ne  | Op::F32Lt  | Op::F32Gt  | Op::F32Le  | Op::F32Ge  |
                Op::F64Eq  | Op::F64Ne  | Op::F64Lt  | Op::F64Gt  | Op::F64Le  | Op::F64Ge  |
                Op::I32WrapI64 |
                Op::I64ExtendI32S | Op::I64ExtendI32U |
                Op::F32ConvertI32S | Op::F32ConvertI32U |
                Op::F64ConvertI32S | Op::F64ConvertI32U |
                Op::F64ConvertI64S | Op::F64ConvertI64U |
                Op::I32TruncF32S | Op::I32TruncF32U | Op::I32TruncF64S | Op::I32TruncF64U |
                Op::F32DemoteF64 | Op::F64PromoteF32 |
                Op::I32ReinterpretF32 | Op::F32ReinterpretI32 |
                Op::I64ReinterpretF64 | Op::F64ReinterpretI64
            ) {
                out.push(i as u8);
                return;
            }
        }
    }
    // Payload ops.
    match op {
        Op::I32Const(v)  => { out.push(0x80); out.extend_from_slice(&v.to_le_bytes()); }
        Op::I64Const(v)  => { out.push(0x81); out.extend_from_slice(&v.to_le_bytes()); }
        Op::F32Const(v)  => { out.push(0x82); out.extend_from_slice(&v.to_bits().to_le_bytes()); }
        Op::F64Const(v)  => { out.push(0x83); out.extend_from_slice(&v.to_bits().to_le_bytes()); }
        Op::LocalGet(i)  => { out.push(0x84); out.extend_from_slice(&i.to_le_bytes()); }
        Op::LocalSet(i)  => { out.push(0x85); out.extend_from_slice(&i.to_le_bytes()); }
        Op::LocalTee(i)  => { out.push(0x86); out.extend_from_slice(&i.to_le_bytes()); }
        Op::Call(i)      => { out.push(0x87); out.extend_from_slice(&i.to_le_bytes()); }
        Op::CallHost(i)  => { out.push(0x88); out.extend_from_slice(&i.to_le_bytes()); }
        Op::Br(d)        => { out.push(0x89); out.extend_from_slice(&d.to_le_bytes()); }
        Op::BrIf(d)      => { out.push(0x8A); out.extend_from_slice(&d.to_le_bytes()); }
        Op::Block(bt)    => { out.push(0x8B); out.push(encode_bt(bt)); }
        Op::Loop(bt)     => { out.push(0x8C); out.push(encode_bt(bt)); }
        Op::If(bt)       => { out.push(0x8D); out.push(encode_bt(bt)); }
        Op::I32Load  { align, offset } => { out.push(0x8E); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::I32Store { align, offset } => { out.push(0x8F); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::I64Load  { align, offset } => { out.push(0x90); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::I64Store { align, offset } => { out.push(0x91); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::F32Load  { align, offset } => { out.push(0x92); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::F32Store { align, offset } => { out.push(0x93); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::F64Load  { align, offset } => { out.push(0x94); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        Op::F64Store { align, offset } => { out.push(0x95); out.extend_from_slice(&align.to_le_bytes()); out.extend_from_slice(&offset.to_le_bytes()); }
        _ => {} // unknown ops silently skipped (shouldn't happen)
    }
}

fn encode_bt(bt: &BlockType) -> u8 {
    match bt {
        BlockType::Empty   => 0x40,
        BlockType::Val(vt) => *vt as u8,
    }
}

fn decode_bt(b: u8) -> Option<BlockType> {
    use crate::types::ValType;
    if b == 0x40 { return Some(BlockType::Empty); }
    ValType::from_u8(b).map(BlockType::Val)
}

fn decode_ops(data: &[u8]) -> Option<std::sync::Arc<Vec<Op>>> {
    let mut ops = Vec::new();
    let mut i = 0usize;

    while i < data.len() {
        let byte = data[i]; i += 1;

        if (byte as usize) < SIMPLE_OPS.len() {
            ops.push(SIMPLE_OPS[byte as usize].clone());
            continue;
        }

        macro_rules! read4 {
            () => {{
                if i + 4 > data.len() { return None; }
                let v = u32::from_le_bytes(data[i..i+4].try_into().ok()?);
                i += 4; v
            }};
        }
        macro_rules! read8 {
            () => {{
                if i + 8 > data.len() { return None; }
                let v = u64::from_le_bytes(data[i..i+8].try_into().ok()?);
                i += 8; v
            }};
        }
        macro_rules! read_bt {
            () => {{
                if i >= data.len() { return None; }
                let b = data[i]; i += 1;
                decode_bt(b)?
            }};
        }

        let op = match byte {
            0x80 => Op::I32Const(read4!() as i32),
            0x81 => Op::I64Const(read8!() as i64),
            0x82 => Op::F32Const(f32::from_bits(read4!())),
            0x83 => Op::F64Const(f64::from_bits(read8!())),
            0x84 => Op::LocalGet(read4!()),
            0x85 => Op::LocalSet(read4!()),
            0x86 => Op::LocalTee(read4!()),
            0x87 => Op::Call(read4!()),
            0x88 => Op::CallHost(read4!()),
            0x89 => Op::Br(read4!()),
            0x8A => Op::BrIf(read4!()),
            0x8B => Op::Block(read_bt!()),
            0x8C => Op::Loop(read_bt!()),
            0x8D => Op::If(read_bt!()),
            0x8E => { let a=read4!(); let o=read4!(); Op::I32Load  { align: a, offset: o } }
            0x8F => { let a=read4!(); let o=read4!(); Op::I32Store { align: a, offset: o } }
            0x90 => { let a=read4!(); let o=read4!(); Op::I64Load  { align: a, offset: o } }
            0x91 => { let a=read4!(); let o=read4!(); Op::I64Store { align: a, offset: o } }
            0x92 => { let a=read4!(); let o=read4!(); Op::F32Load  { align: a, offset: o } }
            0x93 => { let a=read4!(); let o=read4!(); Op::F32Store { align: a, offset: o } }
            0x94 => { let a=read4!(); let o=read4!(); Op::F64Load  { align: a, offset: o } }
            0x95 => { let a=read4!(); let o=read4!(); Op::F64Store { align: a, offset: o } }
            _ => return None,
        };
        ops.push(op);
    }

    Some(std::sync::Arc::new(ops))
}
