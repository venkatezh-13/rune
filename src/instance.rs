//! Instance execution — stack interpreter.
//!
//! ## Performance fixes (v0.1.1)
//!
//! Three bugs caused the interpreter to benchmark 10-50x slower than JS:
//!
//! ### Fix 1 — `body.clone()` on every `Call` (→ 50x fib slowdown)
//! Every internal function call cloned the entire `Vec<Op>` body.
//! For recursive fib(30) that is ~2.7M clones of growing vectors.
//! **Fix:** store ops in `Arc<Vec<Op>>` — clone is an atomic refcount bump.
//!
//! ### Fix 2 — `compute_end_targets` + `find_else` called per frame (→ 10x slowdown)
//! The end-target O(n) scan ran fresh on every `run_frame`, including
//! every recursive call. `find_else` was a linear scan on every `If`.
//! **Fix:** precompute `ends` and `elses` tables once per function at module
//! load time, store in `PreparedFunc`. Lookup is now O(1) table access.
//!
//! ### Fix 3 — `Vec::drain()` for call args (→ 2x host-call slowdown)
//! Every Call/CallHost allocated a fresh `Vec<Val>` by draining the stack.
//! **Fix:** slice args directly from the value stack, copy into the new
//! locals vec, then `stack.truncate()` (O(1), no allocation).

use std::sync::Arc;

use crate::{
    ir::{BlockType, Op},
    memory::Memory,
    module::Module,
    trap::{Result, Trap},
    types::{Val, ValType},
};

// ── Prepared function (built once at instantiation time) ──────────────────────

/// A function with its jump tables precomputed.
/// `Arc` fields make `clone()` O(1) — just bumps refcounts.
#[derive(Clone)]
pub(crate) struct PreparedFunc {
    /// The instruction stream (shared, never mutated).
    pub ops: Arc<Vec<Op>>,
    /// `ends[i]` = index of the matching `End` for ops[i] (Block/Loop/If).
    pub ends: Arc<Vec<usize>>,
    /// `elses[i]` = index of the matching `Else` for ops[i] (If), or usize::MAX.
    pub elses: Arc<Vec<usize>>,
    /// Number of function parameters (= first N locals).
    pub n_params: usize,
    /// Types of extra (non-param) locals, zero-initialised at call entry.
    pub extra_locals: Vec<ValType>,
    /// Return type, or None for void.
    pub result_type: Option<ValType>,
}

fn prepare_func(func: &crate::ir::Function) -> PreparedFunc {
    let ops = func.body.clone();
    let n = ops.len();
    let mut ends  = vec![0usize;        n];
    let mut elses = vec![usize::MAX;    n];
    let mut stack: Vec<usize> = Vec::new();

    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Block(_) | Op::Loop(_) | Op::If(_) => stack.push(i),
            Op::Else => {
                if let Some(&if_pc) = stack.last() {
                    elses[if_pc] = i;
                }
            }
            Op::End => {
                if let Some(start) = stack.pop() {
                    ends[start] = i;
                }
            }
            _ => {}
        }
    }

    PreparedFunc {
        ops,
        ends: Arc::new(ends),
        elses: Arc::new(elses),
        n_params: func.ty.params.len(),
        extra_locals: func.locals.clone(),
        result_type: func.ty.results.first().copied(),
    }
}

// ── Control-flow stack frame ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum FrameKind { Block, Loop, If }

struct CtrlFrame {
    kind: FrameKind,
    stack_base: usize,         // value-stack depth at frame entry
    target_pc: usize,         // End index (Block/If) or Loop op index (Loop)
    result_type: Option<ValType>,
}

// ── Instance ──────────────────────────────────────────────────────────────────

/// A live instantiation of a Rune module.
pub struct Instance<'m> {
    pub memory: Memory,
    module: &'m Module,
    prepared: Vec<PreparedFunc>,  // one per module function
}

impl<'m> Instance<'m> {
    pub fn new(module: &'m Module) -> Result<Self> {
        let mut memory = Memory::new(module.initial_memory_pages, module.max_memory_pages);
        for (offset, bytes) in &module.data_segments {
            memory.write_bytes(*offset as usize, bytes)?;
        }
        // Fix 2: precompute jump tables once, at load time.
        let prepared = module.functions.iter().map(prepare_func).collect();
        Ok(Instance { memory, module, prepared })
    }

    /// Call an exported function by name.
    pub fn call(&mut self, func_name: &str, args: &[Val]) -> Result<Option<Val>> {
        let idx = self.module.find_export(func_name)
            .ok_or_else(|| Trap::UndefinedExport(func_name.into()))? as usize;
        // Fix 1: PreparedFunc::clone() is O(1).
        let pf = self.prepared.get(idx)
            .ok_or_else(|| Trap::UndefinedExport(format!("func#{idx}")))?
            .clone();
        let mut locals: Vec<Val> = Vec::with_capacity(args.len() + pf.extra_locals.len());
        locals.extend_from_slice(args);
        for &ty in &pf.extra_locals { 
            locals.push(Val::default_for(ty)); 
        }
        self.exec(&pf, locals)
    }

    // ── Core dispatch loop ────────────────────────────────────────────────────

    fn exec(&mut self, pf: &PreparedFunc, locals: Vec<Val>) -> Result<Option<Val>> {
        let ops = &*pf.ops;
        let ends = &*pf.ends;
        let elses = &*pf.elses;

        let mut stack: Vec<Val> = Vec::with_capacity(16);
        let mut ctrl: Vec<CtrlFrame> = Vec::with_capacity(8);
        let mut locs = locals;
        let mut pc = 0usize;

        // ── Typed-pop macros ─────────────────────────────────────────────────
        macro_rules! pop {
            () => { stack.pop().ok_or(Trap::TypeMismatch)? };
        }
        macro_rules! pop_i32 {
            () => { match stack.pop().ok_or(Trap::TypeMismatch)? {
                Val::I32(v) => v,
                _ => return Err(Trap::TypeMismatch),
            }};
        }
        macro_rules! pop_i64 {
            () => { match stack.pop().ok_or(Trap::TypeMismatch)? {
                Val::I64(v) => v,
                _ => return Err(Trap::TypeMismatch),
            }};
        }
        macro_rules! pop_f32 {
            () => { match stack.pop().ok_or(Trap::TypeMismatch)? {
                Val::F32(v) => v,
                _ => return Err(Trap::TypeMismatch),
            }};
        }
        macro_rules! pop_f64 {
            () => { match stack.pop().ok_or(Trap::TypeMismatch)? {
                Val::F64(v) => v,
                _ => return Err(Trap::TypeMismatch),
            }};
        }

        // ── Branch macro: Fix 2 — O(1) table lookup, no Vec allocation ───────
        macro_rules! do_branch {
            ($depth:expr) => {{
                let depth = $depth as usize;
                let frame = ctrl.get(ctrl.len().saturating_sub(1 + depth))
                    .ok_or(Trap::TypeMismatch)?;
                let is_loop = frame.kind == FrameKind::Loop;
                let target = frame.target_pc;
                let base = frame.stack_base;
                
                // Capture result BEFORE manipulating stack if this is a block with result
                let result = if !is_loop {
                    if let Some(expected_ty) = frame.result_type {
                        // POP the result value (don't just clone it!)
                        let val = stack.pop().ok_or(Trap::TypeMismatch)?;
                        // Verify the value matches expected type
                        match (expected_ty, &val) {
                            (ValType::I32, Val::I32(_)) => Ok(Some(val)),
                            (ValType::I64, Val::I64(_)) => Ok(Some(val)),
                            (ValType::F32, Val::F32(_)) => Ok(Some(val)),
                            (ValType::F64, Val::F64(_)) => Ok(Some(val)),
                            _ => Err(Trap::TypeMismatch),
                        }?
                    } else {
                        None
                    }
                } else {
                    None
                };
                
                // Pop control frames (including the target frame)
                for _ in 0..=depth { 
                    ctrl.pop(); 
                }
                
                // Restore stack to base
                stack.truncate(base);
                
                // Push result if any
                if let Some(v) = result { 
                    stack.push(v); 
                }
                
                if is_loop { target + 1 } else { target }
            }};
        }

        loop {
            if pc >= ops.len() { break; }
            let op = &ops[pc];
            pc += 1;

            match op {
                // ── Constants ─────────────────────────────────────────────────
                Op::I32Const(v) => stack.push(Val::I32(*v)),
                Op::I64Const(v) => stack.push(Val::I64(*v)),
                Op::F32Const(v) => stack.push(Val::F32(*v)),
                Op::F64Const(v) => stack.push(Val::F64(*v)),

                // ── Locals ────────────────────────────────────────────────────
                Op::LocalGet(i) => {
                    let v = *locs.get(*i as usize).ok_or(Trap::TypeMismatch)?;
                    stack.push(v);
                }
                Op::LocalSet(i) => {
                    let v = pop!();
                    *locs.get_mut(*i as usize).ok_or(Trap::TypeMismatch)? = v;
                }
                Op::LocalTee(i) => {
                    let v = *stack.last().ok_or(Trap::TypeMismatch)?;
                    *locs.get_mut(*i as usize).ok_or(Trap::TypeMismatch)? = v;
                }

                // ── Stack ops ─────────────────────────────────────────────────
                Op::Drop => { pop!(); }
                Op::Select => {
                    let cond = pop_i32!();
                    let b = pop!();
                    let a = pop!();
                    stack.push(if cond != 0 { a } else { b });
                }
                Op::Nop => {}
                Op::Unreachable => return Err(Trap::Unreachable),

                // ── i32 arithmetic ────────────────────────────────────────────
                Op::I32Add => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a.wrapping_add(b))); }
                Op::I32Sub => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a.wrapping_sub(b))); }
                Op::I32Mul => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a.wrapping_mul(b))); }
                Op::I32DivS => {
                    let b = pop_i32!();
                    let a = pop_i32!();
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    if a == i32::MIN && b == -1 { return Err(Trap::IntegerOverflow); }
                    stack.push(Val::I32(a / b));
                }
                Op::I32DivU => {
                    let b = pop_i32!() as u32;
                    let a = pop_i32!() as u32;
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I32((a / b) as i32));
                }
                Op::I32RemS => {
                    let b = pop_i32!();
                    let a = pop_i32!();
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I32(a.wrapping_rem(b)));
                }
                Op::I32RemU => {
                    let b = pop_i32!() as u32;
                    let a = pop_i32!() as u32;
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I32((a % b) as i32));
                }
                Op::I32And => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a & b)); }
                Op::I32Or => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a | b)); }
                Op::I32Xor => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a ^ b)); }
                Op::I32Shl => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a.wrapping_shl(b as u32))); }
                Op::I32ShrS => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(a.wrapping_shr(b as u32))); }
                Op::I32ShrU => { let b = pop_i32!() as u32; let a = pop_i32!() as u32; stack.push(Val::I32((a >> (b & 31)) as i32)); }
                Op::I32Clz => { let a = pop_i32!(); stack.push(Val::I32(a.leading_zeros() as i32)); }
                Op::I32Ctz => { let a = pop_i32!(); stack.push(Val::I32(a.trailing_zeros() as i32)); }
                Op::I32Popcnt => { let a = pop_i32!(); stack.push(Val::I32(a.count_ones() as i32)); }
                Op::I32Eqz => { let a = pop_i32!(); stack.push(Val::I32(if a == 0 { 1 } else { 0 })); }

                // ── i32 comparisons ───────────────────────────────────────────
                Op::I32Eq => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
                Op::I32Ne => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
                Op::I32LtS => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
                Op::I32LtU => { let b = pop_i32!() as u32; let a = pop_i32!() as u32; stack.push(Val::I32(if a < b { 1 } else { 0 })); }
                Op::I32GtS => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(if a > b { 1 } else { 0 })); }
                Op::I32GtU => { let b = pop_i32!() as u32; let a = pop_i32!() as u32; stack.push(Val::I32(if a > b { 1 } else { 0 })); }
                Op::I32LeS => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
                Op::I32LeU => { let b = pop_i32!() as u32; let a = pop_i32!() as u32; stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
                Op::I32GeS => { let b = pop_i32!(); let a = pop_i32!(); stack.push(Val::I32(if a >= b { 1 } else { 0 })); }
                Op::I32GeU => { let b = pop_i32!() as u32; let a = pop_i32!() as u32; stack.push(Val::I32(if a >= b { 1 } else { 0 })); }

                // ── i64 arithmetic ────────────────────────────────────────────
                Op::I64Add => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a.wrapping_add(b))); }
                Op::I64Sub => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a.wrapping_sub(b))); }
                Op::I64Mul => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a.wrapping_mul(b))); }
                Op::I64DivS => {
                    let b = pop_i64!();
                    let a = pop_i64!();
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I64(a.wrapping_div(b)));
                }
                Op::I64DivU => {
                    let b = pop_i64!() as u64;
                    let a = pop_i64!() as u64;
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I64((a / b) as i64));
                }
                Op::I64RemS => {
                    let b = pop_i64!();
                    let a = pop_i64!();
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I64(a.wrapping_rem(b)));
                }
                Op::I64RemU => {
                    let b = pop_i64!() as u64;
                    let a = pop_i64!() as u64;
                    if b == 0 { return Err(Trap::DivisionByZero); }
                    stack.push(Val::I64((a % b) as i64));
                }
                Op::I64And => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a & b)); }
                Op::I64Or => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a | b)); }
                Op::I64Xor => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a ^ b)); }
                Op::I64Shl => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a.wrapping_shl(b as u32))); }
                Op::I64ShrS => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I64(a.wrapping_shr(b as u32))); }
                Op::I64ShrU => { let b = pop_i64!() as u64; let a = pop_i64!() as u64; stack.push(Val::I64((a >> (b & 63)) as i64)); }
                Op::I64Eqz => { let a = pop_i64!(); stack.push(Val::I32(if a == 0 { 1 } else { 0 })); }

                // ── i64 comparisons ───────────────────────────────────────────
                Op::I64Eq => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
                Op::I64Ne => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
                Op::I64LtS => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
                Op::I64GtS => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I32(if a > b { 1 } else { 0 })); }
                Op::I64LeS => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
                Op::I64GeS => { let b = pop_i64!(); let a = pop_i64!(); stack.push(Val::I32(if a >= b { 1 } else { 0 })); }
                Op::I64LtU => { let b = pop_i64!() as u64; let a = pop_i64!() as u64; stack.push(Val::I32(if a < b { 1 } else { 0 })); }
                Op::I64GtU => { let b = pop_i64!() as u64; let a = pop_i64!() as u64; stack.push(Val::I32(if a > b { 1 } else { 0 })); }
                Op::I64LeU => { let b = pop_i64!() as u64; let a = pop_i64!() as u64; stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
                Op::I64GeU => { let b = pop_i64!() as u64; let a = pop_i64!() as u64; stack.push(Val::I32(if a >= b { 1 } else { 0 })); }

                // ── f32 arithmetic ────────────────────────────────────────────
                Op::F32Add => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::F32(a + b)); }
                Op::F32Sub => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::F32(a - b)); }
                Op::F32Mul => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::F32(a * b)); }
                Op::F32Div => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::F32(a / b)); }
                Op::F32Sqrt => { let a = pop_f32!(); stack.push(Val::F32(a.sqrt())); }
                Op::F32Min => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::F32(a.min(b))); }
                Op::F32Max => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::F32(a.max(b))); }
                Op::F32Abs => { let a = pop_f32!(); stack.push(Val::F32(a.abs())); }
                Op::F32Neg => { let a = pop_f32!(); stack.push(Val::F32(-a)); }
                Op::F32Ceil => { let a = pop_f32!(); stack.push(Val::F32(a.ceil())); }
                Op::F32Floor => { let a = pop_f32!(); stack.push(Val::F32(a.floor())); }

                // ── f64 arithmetic ────────────────────────────────────────────
                Op::F64Add => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::F64(a + b)); }
                Op::F64Sub => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::F64(a - b)); }
                Op::F64Mul => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::F64(a * b)); }
                Op::F64Div => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::F64(a / b)); }
                Op::F64Sqrt => { let a = pop_f64!(); stack.push(Val::F64(a.sqrt())); }
                Op::F64Min => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::F64(a.min(b))); }
                Op::F64Max => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::F64(a.max(b))); }
                Op::F64Abs => { let a = pop_f64!(); stack.push(Val::F64(a.abs())); }
                Op::F64Neg => { let a = pop_f64!(); stack.push(Val::F64(-a)); }
                Op::F64Ceil => { let a = pop_f64!(); stack.push(Val::F64(a.ceil())); }
                Op::F64Floor => { let a = pop_f64!(); stack.push(Val::F64(a.floor())); }

                // ── f32/f64 comparisons ───────────────────────────────────────
                Op::F32Eq => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
                Op::F32Ne => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
                Op::F32Lt => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
                Op::F32Gt => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::I32(if a > b { 1 } else { 0 })); }
                Op::F32Le => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
                Op::F32Ge => { let b = pop_f32!(); let a = pop_f32!(); stack.push(Val::I32(if a >= b { 1 } else { 0 })); }
                Op::F64Eq => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::I32(if a == b { 1 } else { 0 })); }
                Op::F64Ne => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::I32(if a != b { 1 } else { 0 })); }
                Op::F64Lt => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::I32(if a < b { 1 } else { 0 })); }
                Op::F64Gt => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::I32(if a > b { 1 } else { 0 })); }
                Op::F64Le => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::I32(if a <= b { 1 } else { 0 })); }
                Op::F64Ge => { let b = pop_f64!(); let a = pop_f64!(); stack.push(Val::I32(if a >= b { 1 } else { 0 })); }

                // ── Conversions ───────────────────────────────────────────────
                Op::I32WrapI64 => { let a = pop_i64!(); stack.push(Val::I32(a as i32)); }
                Op::I64ExtendI32S => { let a = pop_i32!(); stack.push(Val::I64(a as i64)); }
                Op::I64ExtendI32U => { let a = pop_i32!() as u32; stack.push(Val::I64(a as i64)); }
                Op::F32ConvertI32S => { let a = pop_i32!(); stack.push(Val::F32(a as f32)); }
                Op::F32ConvertI32U => { let a = pop_i32!() as u32; stack.push(Val::F32(a as f32)); }
                Op::F64ConvertI32S => { let a = pop_i32!(); stack.push(Val::F64(a as f64)); }
                Op::F64ConvertI32U => { let a = pop_i32!() as u32; stack.push(Val::F64(a as f64)); }
                Op::F64ConvertI64S => { let a = pop_i64!(); stack.push(Val::F64(a as f64)); }
                Op::F64ConvertI64U => { let a = pop_i64!() as u64; stack.push(Val::F64(a as f64)); }
                Op::I32TruncF32S => { let a = pop_f32!(); stack.push(Val::I32(a as i32)); }
                Op::I32TruncF32U => { let a = pop_f32!(); stack.push(Val::I32(a as u32 as i32)); }
                Op::I32TruncF64S => { let a = pop_f64!(); stack.push(Val::I32(a as i32)); }
                Op::I32TruncF64U => { let a = pop_f64!(); stack.push(Val::I32(a as u32 as i32)); }
                Op::F32DemoteF64 => { let a = pop_f64!(); stack.push(Val::F32(a as f32)); }
                Op::F64PromoteF32 => { let a = pop_f32!(); stack.push(Val::F64(a as f64)); }
                Op::I32ReinterpretF32 => { let a = pop_f32!(); stack.push(Val::I32(a.to_bits() as i32)); }
                Op::F32ReinterpretI32 => { let a = pop_i32!(); stack.push(Val::F32(f32::from_bits(a as u32))); }
                Op::I64ReinterpretF64 => { let a = pop_f64!(); stack.push(Val::I64(a.to_bits() as i64)); }
                Op::F64ReinterpretI64 => { let a = pop_i64!(); stack.push(Val::F64(f64::from_bits(a as u64))); }

                // ── Memory ops ────────────────────────────────────────────────
                Op::MemorySize => stack.push(Val::I32(self.memory.pages() as i32)),
                Op::MemoryGrow => {
                    let delta = pop_i32!() as usize;
                    let old = self.memory.grow(delta).map(|p| p as i32).unwrap_or(-1);
                    stack.push(Val::I32(old));
                }
                Op::I32Load { offset, .. } => { 
                    let addr = pop_i32!() as usize; 
                    stack.push(Val::I32(self.memory.read_i32(addr + *offset as usize)?)); 
                }
                Op::I32Store { offset, .. } => { 
                    let val = pop_i32!(); 
                    let addr = pop_i32!() as usize; 
                    self.memory.write_i32(addr + *offset as usize, val)?; 
                }
                Op::I64Load { offset, .. } => { 
                    let addr = pop_i32!() as usize; 
                    stack.push(Val::I64(self.memory.read_i64(addr + *offset as usize)?)); 
                }
                Op::I64Store { offset, .. } => { 
                    let val = pop_i64!(); 
                    let addr = pop_i32!() as usize; 
                    self.memory.write_i64(addr + *offset as usize, val)?; 
                }
                Op::F32Load { offset, .. } => { 
                    let addr = pop_i32!() as usize; 
                    stack.push(Val::F32(self.memory.read_f32(addr + *offset as usize)?)); 
                }
                Op::F32Store { offset, .. } => { 
                    let val = pop_f32!(); 
                    let addr = pop_i32!() as usize; 
                    self.memory.write_f32(addr + *offset as usize, val)?; 
                }
                Op::F64Load { offset, .. } => { 
                    let addr = pop_i32!() as usize; 
                    stack.push(Val::F64(self.memory.read_f64(addr + *offset as usize)?)); 
                }
                Op::F64Store { offset, .. } => { 
                    let val = pop_f64!(); 
                    let addr = pop_i32!() as usize; 
                    self.memory.write_f64(addr + *offset as usize, val)?; 
                }

                // ── Control flow ──────────────────────────────────────────────
                Op::Block(bt) => {
                    ctrl.push(CtrlFrame {
                        kind: FrameKind::Block,
                        stack_base: stack.len(),
                        target_pc: ends[pc - 1],
                        result_type: block_result(bt),
                    });
                }
                Op::Loop(bt) => {
                    ctrl.push(CtrlFrame {
                        kind: FrameKind::Loop,
                        stack_base: stack.len(),
                        target_pc: pc - 1,      // branch back to Loop op
                        result_type: block_result(bt),
                    });
                }
                Op::If(bt) => {
                    let cond = pop_i32!();
                    ctrl.push(CtrlFrame {
                        kind: FrameKind::If,
                        stack_base: stack.len(),
                        target_pc: ends[pc - 1],
                        result_type: block_result(bt),
                    });
                    if cond == 0 {
                        // Fix 2: O(1) precomputed Else lookup (no linear scan).
                        let else_pc = elses[pc - 1];
                        if else_pc != usize::MAX {
                            pc = else_pc + 1;
                        } else {
                            pc = ends[pc - 1];
                            ctrl.pop();
                        }
                    }
                }
                Op::Else => {
                    // End of "then" branch — jump to End.
                    let end_pc = ctrl.last().ok_or(Trap::TypeMismatch)?.target_pc;
                    ctrl.pop();
                    pc = end_pc;
                }
                Op::End => {
                    if !ctrl.is_empty() { 
                        ctrl.pop(); 
                    } else { 
                        break; 
                    }
                }
                Op::Return => break,

                Op::Br(depth) => { 
                    pc = do_branch!(*depth); 
                }
                Op::BrIf(depth) => {
                    let cond = pop_i32!();
                    if cond != 0 { 
                        pc = do_branch!(*depth); 
                    }
                }

                // ── Function calls ────────────────────────────────────────────
                Op::Call(idx) => {
                    let idx = *idx as usize;
                    // Fix 1: O(1) clone (Arc refcount bump, no memcopy).
                    let callee = self.prepared.get(idx)
                        .ok_or_else(|| Trap::UndefinedExport(format!("func#{idx}")))?
                        .clone();
                    let n = callee.n_params;
                    if stack.len() < n { return Err(Trap::TypeMismatch); }
                    let arg_start = stack.len() - n;

                    // Fix 3: slice off stack directly — no Vec::drain() allocation.
                    let mut call_locals: Vec<Val> = Vec::with_capacity(n + callee.extra_locals.len());
                    call_locals.extend_from_slice(&stack[arg_start..]);
                    for &ty in &callee.extra_locals { 
                        call_locals.push(Val::default_for(ty)); 
                    }
                    stack.truncate(arg_start);  // O(1) — just moves the length

                    let result = self.exec(&callee, call_locals)?;
                    if let Some(v) = result { 
                        stack.push(v); 
                    }
                }
                Op::CallHost(idx) => {
                    let idx = *idx as usize;
                    let host = self.module.host_funcs.get(idx)
                        .ok_or_else(|| Trap::UndefinedImport(format!("host#{idx}")))?;
                    let n = host.ty.params.len();
                    if stack.len() < n { return Err(Trap::TypeMismatch); }
                    let arg_start = stack.len() - n;

                    // Fix 3: pass args as slice — zero allocation on hot path.
                    let result = (host.func)(&stack[arg_start..])?;
                    stack.truncate(arg_start);
                    if let Some(v) = result { 
                        stack.push(v); 
                    }
                }
            }
        }

        // At function return, the top of stack should be the return value
        match pf.result_type {
            Some(expected_ty) => {
                let val = stack.pop().ok_or(Trap::TypeMismatch)?;
                // Verify the return value type
                match (expected_ty, &val) {
                    (ValType::I32, Val::I32(_)) => Ok(Some(val)),
                    (ValType::I64, Val::I64(_)) => Ok(Some(val)),
                    (ValType::F32, Val::F32(_)) => Ok(Some(val)),
                    (ValType::F64, Val::F64(_)) => Ok(Some(val)),
                    _ => Err(Trap::TypeMismatch),
                }
            }
            None => {
                // Ensure stack is empty for void functions
                if !stack.is_empty() {
                    Err(Trap::TypeMismatch)
                } else {
                    Ok(None)
                }
            }
        }
    }
}

fn block_result(bt: &BlockType) -> Option<ValType> {
    match bt {
        BlockType::Empty => None,
        BlockType::Val(vt) => Some(*vt),
    }
}
