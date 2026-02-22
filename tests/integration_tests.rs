//! Integration tests for the Rune runtime.
//!
//! Each test builds a Module in-memory using the builder API, instantiates it,
//! and verifies the output. This exercises the full pipeline:
//!   Module builder → Module::to_bytes → Module::from_bytes → Instance::call

use rune::{
    ir::{BlockType, Function, Op},
    module::Module,
    runtime::Runtime,
    trap::Trap,
    types::{FuncType, Val, ValType},
};
use std::sync::Arc;

// Helper: build a Function using the new Arc-body API from a raw Vec<Op>
fn func(
    name: &str,
    params: Vec<ValType>,
    results: Vec<ValType>,
    locals: Vec<ValType>,
    body: Vec<Op>,
) -> Function {
    Function::new(name, FuncType { params, results }, locals, body)
}

fn rt() -> Runtime {
    Runtime::new()
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn single_func(name: &str, params: &[ValType], result: Option<ValType>, body: Vec<Op>) -> Module {
    let mut m = Module::new();
    m.functions.push(Function::new(
        name,
        FuncType {
            params: params.to_vec(),
            results: result.into_iter().collect(),
        },
        vec![],
        body,
    ));
    m.exports.push((name.into(), 0));
    m
}

// ── Basic arithmetic ──────────────────────────────────────────────────────────

#[test]
fn test_i32_add() {
    let m = single_func(
        "add",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Add, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("add", &[Val::I32(10), Val::I32(32)]).unwrap(),
        Some(Val::I32(42))
    );
}

#[test]
fn test_i32_sub() {
    let m = single_func(
        "sub",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Sub, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("sub", &[Val::I32(100), Val::I32(58)]).unwrap(),
        Some(Val::I32(42))
    );
}

#[test]
fn test_i32_mul() {
    let m = single_func(
        "mul",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Mul, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("mul", &[Val::I32(6), Val::I32(7)]).unwrap(),
        Some(Val::I32(42))
    );
}

#[test]
fn test_i32_div_s() {
    let m = single_func(
        "div",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32DivS, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("div", &[Val::I32(84), Val::I32(2)]).unwrap(),
        Some(Val::I32(42))
    );
}

#[test]
fn test_i32_div_by_zero() {
    let m = single_func(
        "divz",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32DivS, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("divz", &[Val::I32(1), Val::I32(0)]).unwrap_err(),
        Trap::DivisionByZero
    );
}

#[test]
fn test_wrapping_add() {
    let m = single_func(
        "wadd",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Add, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("wadd", &[Val::I32(i32::MAX), Val::I32(1)])
            .unwrap(),
        Some(Val::I32(i32::MIN))
    ); // wrapping
}

// ── Bitwise ───────────────────────────────────────────────────────────────────

#[test]
fn test_i32_and() {
    let m = single_func(
        "and",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32And, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("and", &[Val::I32(0xFF), Val::I32(0x0F)]).unwrap(),
        Some(Val::I32(0x0F))
    );
}

#[test]
fn test_i32_or() {
    let m = single_func(
        "or",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Or, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("or", &[Val::I32(0xF0), Val::I32(0x0F)]).unwrap(),
        Some(Val::I32(0xFF))
    );
}

#[test]
fn test_i32_xor() {
    let m = single_func(
        "xor",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Xor, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("xor", &[Val::I32(0xFF), Val::I32(0xFF)]).unwrap(),
        Some(Val::I32(0))
    );
}

// ── Comparisons ───────────────────────────────────────────────────────────────

#[test]
fn test_i32_eq() {
    let m = single_func(
        "eq",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Eq, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("eq", &[Val::I32(5), Val::I32(5)]).unwrap(),
        Some(Val::I32(1))
    );
    assert_eq!(
        inst.call("eq", &[Val::I32(5), Val::I32(6)]).unwrap(),
        Some(Val::I32(0))
    );
}

#[test]
fn test_i32_lt_s() {
    let m = single_func(
        "lt",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32LtS, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("lt", &[Val::I32(-1), Val::I32(0)]).unwrap(),
        Some(Val::I32(1))
    );
    assert_eq!(
        inst.call("lt", &[Val::I32(1), Val::I32(0)]).unwrap(),
        Some(Val::I32(0))
    );
}

// ── i64 ───────────────────────────────────────────────────────────────────────

#[test]
fn test_i64_add() {
    let m = single_func(
        "add64",
        &[ValType::I64, ValType::I64],
        Some(ValType::I64),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I64Add, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    let big = 1_000_000_000i64;
    assert_eq!(
        inst.call("add64", &[Val::I64(big), Val::I64(big)]).unwrap(),
        Some(Val::I64(2_000_000_000))
    );
}

// ── f64 ───────────────────────────────────────────────────────────────────────

#[test]
fn test_f64_sqrt() {
    let m = single_func(
        "mysqrt",
        &[ValType::F64],
        Some(ValType::F64),
        vec![Op::LocalGet(0), Op::F64Sqrt, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    let result = inst.call("mysqrt", &[Val::F64(9.0)]).unwrap();
    if let Some(Val::F64(v)) = result {
        assert!((v - 3.0).abs() < 1e-12);
    } else {
        panic!("expected F64");
    }
}

// ── Locals ────────────────────────────────────────────────────────────────────

#[test]
fn test_local_set_get() {
    // double(x) = x + x  using a local
    let mut m = Module::new();
    m.functions.push(Function::new(
        "double",
        FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        vec![ValType::I32], // local[1]
        vec![
            Op::LocalGet(0),
            Op::LocalGet(0),
            Op::I32Add,
            Op::LocalSet(1),
            Op::LocalGet(1),
            Op::Return,
        ],
    ));
    m.exports.push(("double".into(), 0));
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("double", &[Val::I32(21)]).unwrap(),
        Some(Val::I32(42))
    );
}

// ── Constants ─────────────────────────────────────────────────────────────────

#[test]
fn test_constant_return() {
    let m = single_func(
        "answer",
        &[],
        Some(ValType::I32),
        vec![Op::I32Const(42), Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("answer", &[]).unwrap(), Some(Val::I32(42)));
}

// ── Select ────────────────────────────────────────────────────────────────────

#[test]
fn test_select() {
    // select(a, b, cond) => if cond != 0 then a else b
    let m = single_func(
        "sel",
        &[ValType::I32, ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![
            Op::LocalGet(0),
            Op::LocalGet(1),
            Op::LocalGet(2),
            Op::Select,
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("sel", &[Val::I32(1), Val::I32(2), Val::I32(1)])
            .unwrap(),
        Some(Val::I32(1))
    );
    assert_eq!(
        inst.call("sel", &[Val::I32(1), Val::I32(2), Val::I32(0)])
            .unwrap(),
        Some(Val::I32(2))
    );
}

// ── Memory ────────────────────────────────────────────────────────────────────

#[test]
fn test_memory_store_load() {
    // write 99 at offset 0, then read it back
    let m = single_func(
        "memtest",
        &[],
        Some(ValType::I32),
        vec![
            Op::I32Const(0),  // address
            Op::I32Const(99), // value
            Op::I32Store {
                align: 2,
                offset: 0,
            },
            Op::I32Const(0),
            Op::I32Load {
                align: 2,
                offset: 0,
            },
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("memtest", &[]).unwrap(), Some(Val::I32(99)));
}

#[test]
fn test_memory_oob() {
    // try to read past the end of memory
    let m = single_func(
        "oob",
        &[],
        Some(ValType::I32),
        vec![
            Op::I32Const(i32::MAX),
            Op::I32Load {
                align: 2,
                offset: 0,
            },
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("oob", &[]).unwrap_err(), Trap::OutOfBounds);
}

#[test]
fn test_memory_grow() {
    let m = single_func(
        "grow",
        &[],
        Some(ValType::I32),
        vec![
            Op::I32Const(2), // grow by 2 pages
            Op::MemoryGrow,  // returns old page count (1)
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("grow", &[]).unwrap(), Some(Val::I32(1)));
    assert_eq!(inst.memory.pages(), 3);
}

#[test]
fn test_memory_size() {
    let m = single_func(
        "msize",
        &[],
        Some(ValType::I32),
        vec![Op::MemorySize, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("msize", &[]).unwrap(), Some(Val::I32(1)));
}

// ── Data segments ─────────────────────────────────────────────────────────────

#[test]
fn test_data_segment() {
    let mut m = single_func(
        "read_data",
        &[],
        Some(ValType::I32),
        vec![
            Op::I32Const(0),
            Op::I32Load {
                align: 2,
                offset: 0,
            },
            Op::Return,
        ],
    );
    // Write 0xDEADBEEFu32 at offset 0
    m.data_segments.push((0, vec![0xEF, 0xBE, 0xAD, 0xDE]));
    let mut inst = rt().instantiate(&m).unwrap();
    let result = inst.call("read_data", &[]).unwrap();
    assert_eq!(result, Some(Val::I32(0xDEADBEEFu32 as i32)));
}

// ── Control flow ──────────────────────────────────────────────────────────────

#[test]
fn test_if_then_else() {
    // abs(x) = if x < 0 then -x else x
    let m = single_func(
        "abs",
        &[ValType::I32],
        Some(ValType::I32),
        vec![
            Op::LocalGet(0),
            Op::I32Const(0),
            Op::I32LtS,
            Op::If(BlockType::Val(ValType::I32)),
            Op::I32Const(0),
            Op::LocalGet(0),
            Op::I32Sub,
            Op::Else,
            Op::LocalGet(0),
            Op::End,
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("abs", &[Val::I32(-5)]).unwrap(),
        Some(Val::I32(5))
    );
    assert_eq!(inst.call("abs", &[Val::I32(7)]).unwrap(), Some(Val::I32(7)));
}

#[test]
fn test_block_br() {
    // Returns 99 by branching out of a block.
    let m = single_func(
        "blk",
        &[],
        Some(ValType::I32),
        vec![
            Op::Block(BlockType::Val(ValType::I32)),
            Op::I32Const(99),
            Op::Br(0),
            Op::I32Const(0), // unreachable
            Op::End,
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("blk", &[]).unwrap(), Some(Val::I32(99)));
}

#[test]
fn test_loop_countdown() {
    // count down from N to 0 using a loop, return 0.
    //
    // Br(0) = continue loop (targets the Loop frame itself)
    // Br(1) = break out     (targets the wrapping Block, exits past its End)
    // A bare Loop with no enclosing Block cannot be exited with Br(1) —
    // the Block wrapper is required so depth-1 has a valid target.
    let m = single_func(
        "countdown",
        &[ValType::I32],
        Some(ValType::I32),
        vec![
            Op::Block(BlockType::Empty), // depth 1 — break target
            Op::Loop(BlockType::Empty),  // depth 0 — continue target
            Op::LocalGet(0),
            Op::I32Eqz,
            Op::BrIf(1), // if i==0: exit Block
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32Sub,
            Op::LocalSet(0),
            Op::Br(0), // continue Loop
            Op::End,   // End Loop
            Op::End,   // End Block
            Op::LocalGet(0),
            Op::Return,
        ],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("countdown", &[Val::I32(10)]).unwrap(),
        Some(Val::I32(0))
    );
}

// ── Internal function calls ───────────────────────────────────────────────────

#[test]
fn test_internal_call() {
    // square(x) calls mul(x, x)
    let mut m = Module::new();
    // func[0] = mul(a, b) = a * b
    m.functions.push(Function::new(
        "mul",
        FuncType {
            params: vec![ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        },
        vec![],
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Mul, Op::Return],
    ));
    // func[1] = square(x) = mul(x, x)
    m.functions.push(Function::new(
        "square",
        FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        vec![],
        vec![Op::LocalGet(0), Op::LocalGet(0), Op::Call(0), Op::Return],
    ));
    m.exports.push(("square".into(), 1));
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("square", &[Val::I32(7)]).unwrap(),
        Some(Val::I32(49))
    );
}

// ── Host function calls ───────────────────────────────────────────────────────

#[test]
fn test_host_call() {
    use std::sync::{Arc, Mutex};

    let log: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();

    let mut m = Module::new();
    m.register_host(
        "log",
        FuncType {
            params: vec![ValType::I32],
            results: vec![],
        },
        move |args| {
            log2.lock().unwrap().push(args[0].as_i32().unwrap());
            Ok(None)
        },
    );
    m.functions.push(Function::new(
        "run",
        FuncType {
            params: vec![],
            results: vec![],
        },
        vec![],
        vec![
            Op::I32Const(42),
            Op::CallHost(0),
            Op::I32Const(7),
            Op::CallHost(0),
            Op::Return,
        ],
    ));
    m.exports.push(("run".into(), 0));

    let mut inst = rt().instantiate(&m).unwrap();
    inst.call("run", &[]).unwrap();
    assert_eq!(*log.lock().unwrap(), vec![42, 7]);
}

// ── Module serialization round-trip ──────────────────────────────────────────

#[test]
fn test_module_roundtrip() {
    let m = single_func(
        "add",
        &[ValType::I32, ValType::I32],
        Some(ValType::I32),
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Add, Op::Return],
    );

    let bytes = m.to_bytes();
    let m2 = Module::from_bytes(&bytes).expect("failed to deserialize");

    let mut inst = rt().instantiate(&m2).unwrap();
    assert_eq!(
        inst.call("add", &[Val::I32(20), Val::I32(22)]).unwrap(),
        Some(Val::I32(42))
    );
}

#[test]
fn test_module_bad_magic() {
    let bytes = b"XXXX\x00\x00\x00\x00".to_vec();
    assert!(Module::from_bytes(&bytes).is_err());
}

// ── Conversions ───────────────────────────────────────────────────────────────

#[test]
fn test_i64_extend_i32_s() {
    let m = single_func(
        "ext",
        &[ValType::I32],
        Some(ValType::I64),
        vec![Op::LocalGet(0), Op::I64ExtendI32S, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(
        inst.call("ext", &[Val::I32(-1)]).unwrap(),
        Some(Val::I64(-1))
    );
}

#[test]
fn test_f64_promote_f32() {
    let m = single_func(
        "prom",
        &[ValType::F32],
        Some(ValType::F64),
        vec![Op::LocalGet(0), Op::F64PromoteF32, Op::Return],
    );
    let mut inst = rt().instantiate(&m).unwrap();
    if let Some(Val::F64(v)) = inst.call("prom", &[Val::F32(1.5)]).unwrap() {
        assert!((v - 1.5).abs() < 1e-12);
    } else {
        panic!();
    }
}

// ── Undefined export ──────────────────────────────────────────────────────────

#[test]
fn test_undefined_export() {
    let m = Module::new();
    let mut inst = rt().instantiate(&m).unwrap();
    assert!(matches!(
        inst.call("nope", &[]),
        Err(Trap::UndefinedExport(_))
    ));
}

// ── Benchmarks / Stress Tests (reviewer-requested) ───────────────────────────

/// Stress-test: recursive fib(30) exercises deep Call stacks.
/// Passes when it completes and returns the correct value.
/// (Performance target for AOT: <1ms; interpreter: ~50ms is acceptable.)
#[test]
fn bench_fibonacci_30() {
    let mut m = Module::new();
    m.functions.push(Function::new(
        "fib",
        FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        vec![],
        vec![
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32LeS,
            Op::If(BlockType::Val(ValType::I32)),
            Op::LocalGet(0),
            Op::Else,
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32Sub,
            Op::Call(0),
            Op::LocalGet(0),
            Op::I32Const(2),
            Op::I32Sub,
            Op::Call(0),
            Op::I32Add,
            Op::End,
            Op::Return,
        ],
    ));
    m.exports.push(("fib".into(), 0));
    let mut inst = rt().instantiate(&m).unwrap();
    // fib(30) = 832040
    let result = inst.call("fib", &[Val::I32(30)]).unwrap();
    assert_eq!(result, Some(Val::I32(832040)));
}

/// Stress-test: allocate and grow to 100 pages (~6.4MB), write a pattern
/// across every 4 bytes, then verify all reads. Catches memory bounds bugs.
#[test]
fn memory_stress_100_pages() {
    use rune::memory::PAGE_SIZE;

    let mut m = Module::new();
    // A no-op function — we test memory directly via the instance API.
    m.functions.push(Function::new(
        "nop",
        FuncType {
            params: vec![],
            results: vec![],
        },
        vec![],
        vec![Op::Return],
    ));
    m.exports.push(("nop".into(), 0));
    m.initial_memory_pages = 1;
    m.max_memory_pages = Some(200);

    let mut inst = rt().instantiate(&m).unwrap();

    // Grow to 100 pages.
    inst.memory.grow(99).expect("grow failed");
    assert_eq!(inst.memory.pages(), 100);

    let total_bytes = 100 * PAGE_SIZE;

    // Write a u32 pattern across the full memory space.
    let mut offset = 0usize;
    let mut value = 0u32;
    while offset + 4 <= total_bytes {
        inst.memory.write_u32(offset, value).unwrap();
        offset += 4;
        value = value.wrapping_add(1);
    }

    // Verify every value.
    let mut offset = 0usize;
    let mut value = 0u32;
    let mut errors = 0u32;
    while offset + 4 <= total_bytes {
        let v = inst.memory.read_u32(offset).unwrap();
        if v != value {
            errors += 1;
        }
        offset += 4;
        value = value.wrapping_add(1);
    }

    assert_eq!(errors, 0, "{errors} memory read/write mismatches");
}

/// Stress-test: call a host function 100,000 times in a Rune loop.
/// Validates that the host call dispatch path has no memory/stack issues.
/// (100k instead of 1M to keep CI fast; the overhead scales linearly.)
#[test]
fn host_callback_loop_100k() {
    use std::sync::{Arc, Mutex};

    const ITERATIONS: i32 = 100_000;

    let counter: Arc<Mutex<i32>> = Arc::new(Mutex::new(0));
    let counter2 = counter.clone();

    let mut m = Module::new();
    // host[0]: increment()
    m.register_host(
        "increment",
        FuncType {
            params: vec![],
            results: vec![],
        },
        move |_args| {
            *counter2.lock().unwrap() += 1;
            Ok(None)
        },
    );

    // Guest: loop ITERATIONS times, call host each time.
    //
    // locals[0] = i (counter, starts at ITERATIONS counts down to 0)
    m.functions.push(Function::new(
        "run",
        FuncType {
            params: vec![],
            results: vec![],
        },
        vec![ValType::I32],
        vec![
            // i = ITERATIONS
            Op::I32Const(ITERATIONS),
            Op::LocalSet(0),
            // Block wraps the Loop so BrIf(1) has a valid exit target.
            // Br(0) = continue Loop, Br(1) = break out of Block.
            Op::Block(BlockType::Empty), // depth 1 — break target
            Op::Loop(BlockType::Empty),  // depth 0 — continue target
            // if i == 0: exit Block
            Op::LocalGet(0),
            Op::I32Eqz,
            Op::BrIf(1),
            // call host
            Op::CallHost(0),
            // i -= 1
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32Sub,
            Op::LocalSet(0),
            // continue Loop
            Op::Br(0),
            Op::End, // End Loop
            Op::End, // End Block
            Op::Return,
        ],
    ));
    m.exports.push(("run".into(), 0));

    let mut inst = rt().instantiate(&m).unwrap();
    inst.call("run", &[]).expect("run failed");

    assert_eq!(
        *counter.lock().unwrap(),
        ITERATIONS,
        "host was not called the expected number of times"
    );
}

// ── Fibonacci (recursive) ─────────────────────────────────────────────────────

#[test]
fn test_fibonacci() {
    // fib(n): recursive fibonacci
    // fib(0) = 0, fib(1) = 1, fib(n) = fib(n-1) + fib(n-2)
    let mut m = Module::new();
    m.functions.push(Function::new(
        "fib",
        FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        vec![],
        vec![
            // if n <= 1 return n
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32LeS,
            Op::If(BlockType::Val(ValType::I32)),
            Op::LocalGet(0),
            Op::Else,
            // fib(n-1) + fib(n-2)
            Op::LocalGet(0),
            Op::I32Const(1),
            Op::I32Sub,
            Op::Call(0), // fib(n-1)
            Op::LocalGet(0),
            Op::I32Const(2),
            Op::I32Sub,
            Op::Call(0), // fib(n-2)
            Op::I32Add,
            Op::End,
            Op::Return,
        ],
    ));
    m.exports.push(("fib".into(), 0));
    let mut inst = rt().instantiate(&m).unwrap();
    assert_eq!(inst.call("fib", &[Val::I32(0)]).unwrap(), Some(Val::I32(0)));
    assert_eq!(inst.call("fib", &[Val::I32(1)]).unwrap(), Some(Val::I32(1)));
    assert_eq!(
        inst.call("fib", &[Val::I32(10)]).unwrap(),
        Some(Val::I32(55))
    );
}
