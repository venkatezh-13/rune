//! Cranelift AOT compiler backend (Phase 1 stub).
//!
//! In the full implementation this module translates RuneIR → Cranelift IR →
//! native machine code. For the MVP, execution goes through the interpreter in
//! `instance.rs`.

pub mod codegen;
