#![allow(dead_code, unused_imports)]
//! C API (FFI) implementation — corresponds to rune.h.
//!
//! This module exposes a stable C ABI so Rune can be embedded from C, Python,
//! Go, Swift, or any other language. All heap-allocated objects are opaque
//! pointers managed by the caller via the `_free` functions.
//!
//! Status: Phase 2 — implementations are correct for the interpreter path.
//!         AOT path will wire in automatically once `instance.rs` switches to
//!         native execution.

#![allow(clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::slice;

use crate::{
    module::Module,
    runtime::Runtime,
    instance::Instance,
    trap::Trap,
    types::{FuncType, Val, ValType},
};

// ── C-compatible error codes ──────────────────────────────────────────────────

#[repr(C)]
pub enum RuneError {
    Ok                = 0,
    InvalidModule     = 1,
    OutOfMemory       = 2,
    TrapOutOfBounds   = 3,
    TrapDivZero       = 4,
    TrapUnreachable   = 5,
    TrapStackOverflow = 6,
    TrapTypeMismatch  = 7,
    UndefinedExport   = 8,
    UndefinedImport   = 9,
    HostError         = 10,
}

impl From<&Trap> for RuneError {
    fn from(t: &Trap) -> Self {
        match t {
            Trap::OutOfBounds       => RuneError::TrapOutOfBounds,
            Trap::OutOfMemory       => RuneError::OutOfMemory,
            Trap::DivisionByZero    => RuneError::TrapDivZero,
            Trap::Unreachable       => RuneError::TrapUnreachable,
            Trap::StackOverflow     => RuneError::TrapStackOverflow,
            Trap::TypeMismatch      => RuneError::TrapTypeMismatch,
            Trap::UndefinedExport(_)=> RuneError::UndefinedExport,
            Trap::UndefinedImport(_)=> RuneError::UndefinedImport,
            Trap::InvalidModule(_)  => RuneError::InvalidModule,
            Trap::HostError(_)      => RuneError::HostError,
        }
    }
}

// ── C-compatible value types ──────────────────────────────────────────────────

#[repr(C)]
pub enum RuneValType {
    I32 = 0x7F,
    I64 = 0x7E,
    F32 = 0x7D,
    F64 = 0x7C,
}

impl TryFrom<u8> for RuneValType {
    type Error = ();
    fn try_from(v: u8) -> Result<Self, ()> {
        match v {
            0x7F => Ok(RuneValType::I32),
            0x7E => Ok(RuneValType::I64),
            0x7D => Ok(RuneValType::F32),
            0x7C => Ok(RuneValType::F64),
            _ => Err(()),
        }
    }
}

impl From<RuneValType> for ValType {
    fn from(r: RuneValType) -> ValType {
        match r {
            RuneValType::I32 => ValType::I32,
            RuneValType::I64 => ValType::I64,
            RuneValType::F32 => ValType::F32,
            RuneValType::F64 => ValType::F64,
        }
    }
}

/// C-compatible tagged union for values.
#[repr(C)]
pub union RuneVal {
    pub i32: i32,
    pub i64: i64,
    pub f32: f32,
    pub f64: f64,
}

fn rune_val_to_val(rv: &RuneVal, ty: ValType) -> Val {
    match ty {
        ValType::I32 => Val::I32(unsafe { rv.i32 }),
        ValType::I64 => Val::I64(unsafe { rv.i64 }),
        ValType::F32 => Val::F32(unsafe { rv.f32 }),
        ValType::F64 => Val::F64(unsafe { rv.f64 }),
    }
}

fn val_to_rune_val(v: Val) -> RuneVal {
    match v {
        Val::I32(x) => RuneVal { i32: x },
        Val::I64(x) => RuneVal { i64: x },
        Val::F32(x) => RuneVal { f32: x },
        Val::F64(x) => RuneVal { f64: x },
    }
}

// ── Host function callback type ───────────────────────────────────────────────

pub type RuneHostFn = unsafe extern "C" fn(
    instance: *mut CInstance,
    args: *const RuneVal,
    n_args: usize,
    result: *mut RuneVal,
    user_data: *mut c_void,
) -> RuneError;

// ── Opaque C wrappers ─────────────────────────────────────────────────────────

pub struct CRuntime(Runtime);
pub struct CModule(Module);
/// The Instance borrows the module, but for C ABI we need ownership.
/// We store the module inside the instance box.
pub struct CInstance {
    module: Box<Module>,
    // instance: Instance<'???> — lifetime issue with C ABI
    // Solution: store the module in the box and use raw ptr lifetime.
    // In Phase 2 we'll use an Arc<Module> to solve this cleanly.
    _placeholder: (),
}

// ── Runtime ───────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rune_runtime_new() -> *mut CRuntime {
    Box::into_raw(Box::new(CRuntime(Runtime::new())))
}

/// # Safety
/// Must only be called with a pointer returned by `rune_runtime_new`.
#[no_mangle]
pub unsafe extern "C" fn rune_runtime_free(rt: *mut CRuntime) {
    if !rt.is_null() { drop(Box::from_raw(rt)); }
}

// ── Module loading ────────────────────────────────────────────────────────────

/// # Safety
/// `data` must be valid for `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn rune_module_load_bytes(
    _rt: *mut CRuntime,
    data: *const u8,
    len: usize,
) -> *mut CModule {
    if data.is_null() { return ptr::null_mut(); }
    let bytes = slice::from_raw_parts(data, len);
    match Module::from_bytes(bytes) {
        Ok(m) => Box::into_raw(Box::new(CModule(m))),
        Err(_) => ptr::null_mut(),
    }
}

/// # Safety
/// `path` must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn rune_module_load_file(
    rt: *mut CRuntime,
    path: *const c_char,
) -> *mut CModule {
    if path.is_null() { return ptr::null_mut(); }
    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    let bytes = match std::fs::read(path_str) {
        Ok(b) => b,
        Err(_) => return ptr::null_mut(),
    };
    rune_module_load_bytes(rt, bytes.as_ptr(), bytes.len())
}

/// # Safety
#[no_mangle]
pub unsafe extern "C" fn rune_module_free(module: *mut CModule) {
    if !module.is_null() { drop(Box::from_raw(module)); }
}

// ── Error strings ─────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rune_error_string(err: RuneError) -> *const c_char {
    let s = match err {
        RuneError::Ok                => "ok\0",
        RuneError::InvalidModule     => "invalid module\0",
        RuneError::OutOfMemory       => "out of memory\0",
        RuneError::TrapOutOfBounds   => "memory out-of-bounds\0",
        RuneError::TrapDivZero       => "integer divide by zero\0",
        RuneError::TrapUnreachable   => "unreachable executed\0",
        RuneError::TrapStackOverflow => "stack overflow\0",
        RuneError::TrapTypeMismatch  => "type mismatch\0",
        RuneError::UndefinedExport   => "undefined export\0",
        RuneError::UndefinedImport   => "undefined import\0",
        RuneError::HostError         => "host error\0",
    };
    s.as_ptr() as *const c_char
}
