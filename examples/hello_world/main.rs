//! examples/hello_world — demonstrates building and running a module in Rust.

use rune::{
    ir::{BlockType, Function, Op},
    module::Module,
    runtime::Runtime,
    types::{FuncType, Val, ValType},
};

fn main() {
    // ── Build module ──────────────────────────────────────────────────────────
    let mut module = Module::new();

    // Register host function: print_i32(x: i32)
    module.register_host(
        "print_i32",
        FuncType { params: vec![ValType::I32], results: vec![] },
        |args| {
            println!("Guest says: {}", args[0].as_i32().unwrap());
            Ok(None)
        },
    );

    // Define guest function: run()  — calls print_i32(42)
    module.functions.push(Function {
        name: "run".into(),
        ty: FuncType { params: vec![], results: vec![] },
        locals: vec![],
        body: vec![
            Op::I32Const(42),
            Op::CallHost(0),
            Op::Return,
        ],
    });
    module.exports.push(("run".into(), 0));

    // ── Instantiate and run ───────────────────────────────────────────────────
    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).expect("instantiation failed");
    inst.call("run", &[]).expect("call failed");
}
