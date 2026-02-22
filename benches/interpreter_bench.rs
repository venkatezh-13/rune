//! Interpreter benchmarks using Criterion.
//!
//! Run with: `cargo bench --bench interpreter_bench`
//!
//! The middle number of the three in each result is your actual measurement:
//!   fibonacci/fib(10)    time: [4.8 µs  >>4.9 µs<<  5.1 µs]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rune::{
    ir::{BlockType, Function, Op},
    module::Module,
    runtime::Runtime,
    types::{FuncType, Val, ValType},
};

// ── Module builders ───────────────────────────────────────────────────────────

fn fib_module() -> Module {
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
    m
}

fn add_module() -> Module {
    let mut m = Module::new();
    m.functions.push(Function::new(
        "add",
        FuncType {
            params: vec![ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        },
        vec![],
        vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Add, Op::Return],
    ));
    m.exports.push(("add".into(), 0));
    m
}

fn host_call_module() -> Module {
    let mut m = Module::new();
    m.register_host(
        "noop",
        FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        |args| Ok(Some(args[0])),
    );
    m.functions.push(Function::new(
        "call_host",
        FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
        vec![],
        vec![Op::LocalGet(0), Op::CallHost(0), Op::Return],
    ));
    m.exports.push(("call_host".into(), 0));
    m
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

fn bench_fibonacci(c: &mut Criterion) {
    let module = fib_module();
    let rt = Runtime::new();
    let mut group = c.benchmark_group("fibonacci");
    for n in [10u32, 20, 25] {
        group.bench_with_input(BenchmarkId::new("fib", n), &n, |b, &n| {
            let mut inst = rt.instantiate(&module).unwrap();
            b.iter(|| black_box(inst.call("fib", &[Val::I32(black_box(n as i32))]).unwrap()));
        });
    }
    group.finish();
}

fn bench_simple_call(c: &mut Criterion) {
    let module = add_module();
    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).unwrap();
    c.bench_function("simple_call/add(3,4)", |b| {
        b.iter(|| {
            black_box(
                inst.call("add", &[Val::I32(black_box(3)), Val::I32(black_box(4))])
                    .unwrap(),
            )
        })
    });
}

fn bench_host_call(c: &mut Criterion) {
    let module = host_call_module();
    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).unwrap();
    c.bench_function("host_call/round_trip", |b| {
        b.iter(|| black_box(inst.call("call_host", &[Val::I32(black_box(42))]).unwrap()))
    });
}

fn bench_cold_start(c: &mut Criterion) {
    let rt = Runtime::new();
    let mut group = c.benchmark_group("cold_start");

    // Pre-create modules outside the benchmark loop
    let empty_module = Module::new();
    let fib_module_pre = fib_module();
    let fib_bytes = fib_module().to_bytes();
    let fib_module_from_bytes = Module::from_bytes(&fib_bytes).unwrap();

    // Empty module — baseline instantiation cost
    group.bench_function("empty_module", |b| {
        b.iter(|| {
            // Create a NEW instance each iteration, but module is already created
            let inst = rt.instantiate(&empty_module).unwrap();
            black_box(inst)
        })
    });

    // Fib module — realistic instantiation cost
    group.bench_function("fib_module", |b| {
        b.iter(|| {
            let inst = rt.instantiate(&fib_module_pre).unwrap();
            black_box(inst)
        })
    });

    // From bytes — simulates loading from disk
    group.bench_function("fib_module_from_bytes", |b| {
        b.iter(|| {
            // Module is already created from bytes
            let inst = rt.instantiate(&fib_module_from_bytes).unwrap();
            black_box(inst)
        })
    });

    group.finish();
}

fn bench_memory(c: &mut Criterion) {
    use rune::memory::{Memory, PAGE_SIZE};
    let mut group = c.benchmark_group("memory");

    group.bench_function("write_u32", |b| {
        let mut m = Memory::new(1, None);
        let mut offset = 0usize;
        b.iter(|| {
            m.write_u32(
                black_box(offset % (PAGE_SIZE - 4)),
                black_box(0xDEAD_BEEFu32),
            )
            .unwrap();
            offset = offset.wrapping_add(4);
        })
    });

    group.bench_function("read_u32", |b| {
        let m = Memory::new(1, None);
        let mut offset = 0usize;
        b.iter(|| {
            black_box(m.read_u32(black_box(offset % (PAGE_SIZE - 4))).unwrap());
            offset = offset.wrapping_add(4);
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_fibonacci,
    bench_simple_call,
    bench_host_call,
    bench_cold_start,
    bench_memory,
);
criterion_main!(benches);
