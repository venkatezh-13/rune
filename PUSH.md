# How to push this to GitHub

These are the exact commands to get your local repo up to date and push.

## Step 1 — Copy these files into your local repo

Unzip and copy everything into `C:\Users\User\Desktop\rune\`, replacing existing files.

The new/changed files are:

```
README.md               ← rewritten (honest benchmarks, fixed tagline)
Cargo.toml              ← added license, keywords, repository fields
LICENSE                 ← new (required for cargo publish)
benches/
  interpreter_bench.rs  ← new (real Criterion benchmarks)
.github/
  workflows/
    ci.yml              ← new (GitHub Actions CI)
src/
  ffi.rs                ← new (C ABI implementation)
  stack.rs              ← new (native stack for AOT)
  lib.rs                ← updated (added ffi + stack modules)
tests/
  integration_tests.rs  ← updated (added 3 stress tests)
```

## Step 2 — Open terminal in your repo folder

```
cd C:\Users\User\Desktop\rune
```

## Step 3 — Check what changed

```bash
git status
git diff README.md
```

## Step 4 — Stage and commit everything

```bash
git add .
git commit -m "docs: Honest benchmarks, fixed tagline, CI, LICENSE, Criterion benches

- README: 'capability-based' → 'bounds-checked linear memory' (matches implementation)
- README: Projected benchmark table with cargo bench instructions
- README: Fixed Why not Wasmtime? formatting (missing newline before bullets)
- README: LICENSE added to release checklist
- Cargo.toml: Added license, repository, keywords for crates.io
- LICENSE: MIT license file (required for cargo publish)
- benches/interpreter_bench.rs: Real Criterion benchmarks (fib, host calls, cold start, memory)
- .github/workflows/ci.yml: CI on ubuntu + macos, uploads bench artifacts
- src/ffi.rs: C ABI implementation (rune_runtime_new, rune_module_load_bytes, etc.)
- src/stack.rs: Native stack type for AOT phase
- tests/integration_tests.rs: Added bench_fibonacci_30, memory_stress_100_pages, host_callback_loop_100k"
```

## Step 5 — Push

```bash
git push origin main
```

## Step 6 — Verify on GitHub

```bash
curl -s https://raw.githubusercontent.com/venkatezh-13/rune/main/README.md | head -5
```

Expected output:
```
# Rune

A low-latency, embeddable plugin runtime for Rust applications.
Simple C ABI. Bounds-checked linear memory. No browser, no GC, no JIT warmup.
```

## Step 7 — Verify CI passes

Go to: https://github.com/venkatezh-13/rune/actions

You should see a green checkmark within ~3 minutes of pushing.

---

## Step 8 — After CI is green: cargo publish dry run

```bash
cargo publish --dry-run
```

Fix any warnings it reports, then:

```bash
cargo publish
```

---

## What the CI does

- Runs `cargo test --all` on ubuntu and macos
- Runs `cargo clippy -- -D warnings`
- Runs `cargo fmt -- --check`
- Runs `cargo bench` and uploads results as an artifact

If CI fails on `clippy` or `fmt`, run these locally first:
```bash
cargo fmt --all
cargo clippy --all --fix
```
