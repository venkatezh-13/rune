//! Interpreter benchmarks using Criterion.
//!
//! Run with: `cargo bench --bench interpreter_bench`
//!
//! These produce REAL measured numbers, not estimates.
//! Results will look like:
//!
//!   fibonacci/fib(10)    time: [X µs Y µs Z µs]
//!   fibonacci/fib(20)    time: [X ms Y ms Z ms]
//!   host_call/1_call     time: [X ns Y ns Z ns]
//!   cold_start/empty     time: [X µs Y µs Z µs]

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
    m.functions.push(Function {
        name: "fib".into(),
        ty: FuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        },
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
        ],
    });
    m.exports.push(("fib".into(), 0));
    m
}

fn add_module() -> Module {
    let mut m = Module::new();
    m.functions.push(Function {
        name: "add".into(),
        ty: FuncType {
            params: vec![ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        },
        locals: vec![],
        body: vec![Op::LocalGet(0), Op::LocalGet(1), Op::I32Add, Op::Return],
    });
    m.exports.push(("add".into(), 0));
    m
}

fn host_call_module() -> Module {
    let mut m = Module::new();
    m.register_host(
        "noop",
        FuncType { params: vec![ValType::I32], results: vec![ValType::I32] },
        |args| Ok(Some(args[0])),
    );
    m.functions.push(Function {
        name: "call_host".into(),
        ty: FuncType { params: vec![ValType::I32], results: vec![ValType::I32] },
        locals: vec![],
        body: vec![Op::LocalGet(0), Op::CallHost(0), Op::Return],
    });
    m.exports.push(("call_host".into(), 0));
    m
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

/// Measure recursive fibonacci — the interpreter's worst case (call-heavy).
///
/// These numbers directly answer: "how slow is the interpreter vs AOT?"
/// Expected:
///   fib(10):  ~5µs   interpreter  | <100ns AOT
///   fib(20):  ~500µs interpreter  | <10µs  AOT
///   fib(30):  ~50ms  interpreter  | <1ms   AOT
fn bench_fibonacci(c: &mut Criterion) {
    let module = fib_module();
    let rt = Runtime::new();

    let mut group = c.benchmark_group("fibonacci");
    for n in [10u32, 20, 25] {
        group.bench_with_input(BenchmarkId::new("fib", n), &n, |b, &n| {
            let mut inst = rt.instantiate(&module).unwrap();
            b.iter(|| {
                black_box(inst.call("fib", &[Val::I32(black_box(n as i32))]).unwrap())
            });
        });
    }
    group.finish();
}

/// Measure a single function call (no recursion) — baseline dispatch cost.
fn bench_simple_call(c: &mut Criterion) {
    let module = add_module();
    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).unwrap();

    c.bench_function("simple_call/add(3,4)", |b| {
        b.iter(|| {
            black_box(inst.call("add", &[Val::I32(black_box(3)), Val::I32(black_box(4))]).unwrap())
        })
    });
}

/// Measure round-trip host call cost — this is what the "<10ns" spec target refers to.
/// Current interpreter adds dispatch overhead; AOT will call function pointers directly.
fn bench_host_call(c: &mut Criterion) {
    let module = host_call_module();
    let rt = Runtime::new();
    let mut inst = rt.instantiate(&module).unwrap();

    c.bench_function("host_call/round_trip", |b| {
        b.iter(|| {
            black_box(inst.call("call_host", &[Val::I32(black_box(42))]).unwrap())
        })
    });
}

/// Measure module instantiation — this is the "cold start" metric.
/// Loading an already-parsed module is what must be <5ms.
fn bench_cold_start(c: &mut Criterion) {
    let rt = Runtime::new();

    let mut group = c.benchmark_group("cold_start");

    // Empty module
    let empty = Module::new();
    group.bench_function("empty_module", |b| {
        b.iter(|| black_box(rt.instantiate(&empty).unwrap()))
    });

    // Realistic module with fib function
    let fib = fib_module();
    group.bench_function("fib_module", |b| {
        b.iter(|| black_box(rt.instantiate(&fib).unwrap()))
    });

    // Serialise + deserialise (simulates loading from disk)
    let fib_bytes = fib_module().to_bytes();
    group.bench_function("fib_module_from_bytes", |b| {
        b.iter(|| {
            let m = Module::from_bytes(black_box(&fib_bytes)).unwrap();
            black_box(rt.instantiate(&m).unwrap())
        })
    });

    group.finish();
}

/// Measure memory throughput — bounds-checked reads/writes.
fn bench_memory(c: &mut Criterion) {
    use rune::memory::{Memory, PAGE_SIZE};

    let mut group = c.benchmark_group("memory");

    group.bench_function("write_u32", |b| {
        let mut m = Memory::new(1, None);
        let mut offset = 0usize;
        b.iter(|| {
            m.write_u32(black_box(offset % (PAGE_SIZE - 4)), black_box(0xDEAD_BEEFu32)).unwrap();
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
