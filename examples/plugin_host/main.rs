//! examples/plugin_host — demonstrates module serialization and loading.
//!
//! Simulates a host application that:
//!   1. Builds a plugin module.
//!   2. Serializes it to bytes (as if saving to disk).
//!   3. Loads it back and calls exported functions.

use rune::{
    ir::{BlockType, Function, Op},
    module::Module,
    runtime::Runtime,
    types::{FuncType, Val, ValType},
};

fn build_plugin() -> Vec<u8> {
    let mut m = Module::new();

    // Fibonacci function
    m.functions.push(Function {
        name: "fib".into(),
        ty: FuncType { params: vec![ValType::I32], results: vec![ValType::I32] },
        locals: vec![],
        body: vec![
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32LeS,
            Op::If(BlockType::Val(ValType::I32)),
                Op::LocalGet(0),
            Op::Else,
                Op::LocalGet(0), Op::I32Const(1), Op::I32Sub, Op::Call(0),
                Op::LocalGet(0), Op::I32Const(2), Op::I32Sub, Op::Call(0),
                Op::I32Add,
            Op::End,
            Op::Return,
        ].into(),  // Add .into() here
    });
    m.exports.push(("fib".into(), 0));

    m.to_bytes()
}

fn main() {
    let bytes = build_plugin();
    println!("Plugin size: {} bytes", bytes.len());

    let module = Module::from_bytes(&bytes).expect("failed to load plugin");
    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).expect("instantiation failed");

    for n in 0..=10 {
        let result = inst.call("fib", &[Val::I32(n)]).expect("call failed");
        println!("fib({n}) = {:?}", result.unwrap());
    }
}
