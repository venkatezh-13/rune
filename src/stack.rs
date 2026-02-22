//! Stack management for Rune instances.
//!
//! The software value stack used by the interpreter lives in `instance.rs`
//! as a `Vec<Val>`. This module provides the *native* stack allocation
//! used by AOT-compiled code (Phase 1 Week 3+).
//!
//! In the MVP, `NativeStack` is only allocated as a placeholder; all actual
//! stack frames live on the Rust call stack (recursion in `run_frame`).

use crate::trap::{Result, Trap};

/// Default stack size: 1MB — enough for ~10k recursive calls.
pub const DEFAULT_STACK_SIZE: usize = 1 * 1024 * 1024;

/// A native stack allocation for AOT-compiled guest code.
///
/// Backed by a heap allocation in the MVP; a real implementation would use
/// mmap with guard pages above and below.
pub struct NativeStack {
    /// The backing buffer. Index 0 is the low end (guard territory).
    storage: Vec<u8>,
    /// Logical stack pointer — starts at the top (high address).
    sp: usize,
}

impl NativeStack {
    /// Allocate a new native stack of `size` bytes.
    pub fn new(size: usize) -> Result<Self> {
        if size == 0 {
            return Err(Trap::StackOverflow);
        }
        Ok(NativeStack {
            storage: vec![0u8; size],
            sp: size, // starts at top
        })
    }

    /// Current stack pointer offset (from the base of the storage).
    pub fn sp(&self) -> usize {
        self.sp
    }

    /// Pointer to the base of the stack buffer (lowest address).
    pub fn base(&self) -> *const u8 {
        self.storage.as_ptr()
    }

    /// Pointer to one-past-the-top (the initial stack pointer).
    pub fn top(&self) -> *const u8 {
        unsafe { self.storage.as_ptr().add(self.storage.len()) }
    }

    /// Push `n` bytes onto the stack (grows downward).
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.len() > self.sp {
            return Err(Trap::StackOverflow);
        }
        self.sp -= bytes.len();
        self.storage[self.sp..self.sp + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Pop `n` bytes off the stack.
    pub fn pop_bytes(&mut self, n: usize) -> Result<&[u8]> {
        if self.sp + n > self.storage.len() {
            return Err(Trap::StackOverflow); // underflow
        }
        let slice = &self.storage[self.sp..self.sp + n];
        self.sp += n;
        Ok(slice)
    }

    /// How many bytes are currently on the stack.
    pub fn depth(&self) -> usize {
        self.storage.len() - self.sp
    }

    /// Reset the stack to empty.
    pub fn reset(&mut self) {
        self.sp = self.storage.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_roundtrip() {
        let mut s = NativeStack::new(4096).unwrap();
        assert_eq!(s.depth(), 0);
        s.push_bytes(&42i32.to_le_bytes()).unwrap();
        assert_eq!(s.depth(), 4);
        let bytes = s.pop_bytes(4).unwrap();
        assert_eq!(i32::from_le_bytes(bytes.try_into().unwrap()), 42);
        assert_eq!(s.depth(), 0);
    }

    #[test]
    fn overflow_detected() {
        let mut s = NativeStack::new(8).unwrap();
        assert!(s.push_bytes(&[0u8; 9]).is_err());
    }

    #[test]
    fn reset_clears_stack() {
        let mut s = NativeStack::new(4096).unwrap();
        s.push_bytes(&[1, 2, 3, 4]).unwrap();
        s.reset();
        assert_eq!(s.depth(), 0);
    }
}
