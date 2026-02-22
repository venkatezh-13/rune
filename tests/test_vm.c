/*
 * tests/test_vm.c — Unit tests for the Rune VM
 */

#include "../include/rune.h"
#include "../include/rune_asm.h"
#include "../include/rune_bytecode.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <assert.h>

/* ─── Test harness ─── */
static int g_tests = 0, g_pass = 0, g_fail = 0;

#define TEST(name) \
    do { g_tests++; printf("  %-40s", name); } while(0)

#define PASS() \
    do { g_pass++; printf("PASS\n"); } while(0)

#define FAIL(msg) \
    do { g_fail++; printf("FAIL — %s\n", msg); } while(0)

#define ASSERT_EQ_I32(a, b) \
    do { if ((a) == (b)) PASS(); else { \
        char _buf[64]; snprintf(_buf, 64, "%d != %d", (int)(a), (int)(b)); FAIL(_buf); } } while(0)

#define ASSERT_EQ_I64(a, b) \
    do { if ((a) == (b)) PASS(); else { \
        char _buf[64]; snprintf(_buf, 64, "%lld != %lld", (long long)(a), (long long)(b)); FAIL(_buf); } } while(0)

#define ASSERT_OK(err) \
    do { if ((err) == RUNE_OK) PASS(); else { FAIL(rune_err_str(err)); } } while(0)

#define ASSERT_ERR(err, expected) \
    do { if ((err) == (expected)) PASS(); else { \
        char _buf[64]; snprintf(_buf, 64, "got %s, want %s", rune_err_str(err), rune_err_str(expected)); FAIL(_buf); } } while(0)

/* ─── Helpers ─── */

typedef struct { const char *name; rune_err_t err; } mock_t;

static rune_err_t mock_noop(rune_vm_t *vm, int argc, rune_val_t *argv,
                              rune_val_t *ret, void *ud) {
    (void)vm; (void)argc; (void)argv; (void)ud;
    *ret = rune_void();
    return RUNE_OK;
}

/* Build, load, and initialize a module with no imports */
static rune_vm_t *quick_vm(rune_asm_t *a) {
    size_t sz;
    uint8_t *bin = rune_asm_finalize(a, &sz);
    rune_asm_free(a);

    rune_err_t err;
    rune_module_t *mod = rune_module_load(bin, sz, &err);
    free(bin);
    if (!mod) return NULL;

    rune_vm_t *vm = rune_vm_new(mod, NULL, &err);
    if (!vm) { rune_module_free(mod); return NULL; }

    err = rune_vm_init(vm);
    if (err != RUNE_OK) { rune_vm_free(vm); rune_module_free(mod); return NULL; }
    return vm;
}

/* ─────────────────────────────────────────────
   Tests
───────────────────────────────────────────── */

static void test_add_i32(void) {
    printf("\n[i32 arithmetic]\n");

    /* fn add(a: i32, b: i32) -> i32 { return a + b; } */
    rune_asm_t *a = rune_asm_new();
    rune_type_t p[] = { RUNE_TYPE_I32, RUNE_TYPE_I32 };
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t = rune_asm_type(a, 2, p, 1, r);
    uint32_t fn = rune_asm_func(a, t, 4, 0);
    rune_asm_export_func(a, fn, "add");

    rune_asm_begin_code(a, fn);
    rune_asm_emit(a, OP_ADD32, 0, 0, 1);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    rune_vm_t *vm = quick_vm(a);
    assert(vm);

    TEST("add(10, 32) == 42");
    rune_val_t args[] = { rune_i32(10), rune_i32(32) };
    rune_val_t res = rune_void();
    rune_vm_call(vm, "add", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, 42);

    TEST("add(-1, -1) == -2");
    args[0] = rune_i32(-1); args[1] = rune_i32(-1);
    rune_vm_call(vm, "add", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, -2);

    TEST("add(0, 0) == 0");
    args[0] = rune_i32(0); args[1] = rune_i32(0);
    rune_vm_call(vm, "add", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, 0);

    rune_vm_free(vm);
}

static void test_arithmetic(void) {
    printf("\n[arithmetic operations]\n");

    /* fn ops(x: i32) -> i32 — tests mul/sub/div/rem */
    rune_asm_t *a = rune_asm_new();
    rune_type_t p[] = { RUNE_TYPE_I32 };
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t = rune_asm_type(a, 1, p, 1, r);
    uint32_t fn = rune_asm_func(a, t, 8, 0);
    rune_asm_export_func(a, fn, "ops");

    rune_asm_begin_code(a, fn);
    /* R1 = 6; R2 = R0 * R1; R3 = 4; R0 = R2 / R3; R4 = 3; R0 = R0 % R4 */
    rune_asm_ldi32(a, 1, 6);
    rune_asm_emit(a, OP_MUL32, 2, 0, 1);   /* R2 = x * 6 */
    rune_asm_ldi32(a, 3, 4);
    rune_asm_emit(a, OP_DIV32, 4, 2, 3);   /* R4 = R2 / 4 */
    rune_asm_ldi32(a, 5, 3);
    rune_asm_emit(a, OP_REM32, 0, 4, 5);   /* R0 = R4 % 3 */
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    rune_vm_t *vm = quick_vm(a);
    assert(vm);

    /* ops(2): 2*6=12, 12/4=3, 3%3=0 */
    TEST("ops(2): (2*6)/4 % 3 == 0");
    rune_val_t args[] = { rune_i32(2) };
    rune_val_t res = rune_void();
    rune_vm_call(vm, "ops", 1, args, &res);
    ASSERT_EQ_I32(res.as.i32, 0);

    /* ops(3): 3*6=18, 18/4=4, 4%3=1 */
    TEST("ops(3): (3*6)/4 % 3 == 1");
    args[0] = rune_i32(3);
    rune_vm_call(vm, "ops", 1, args, &res);
    ASSERT_EQ_I32(res.as.i32, 1);

    rune_vm_free(vm);
}

static void test_memory(void) {
    printf("\n[linear memory]\n");

    /* Module with 1 page of memory.
     * fn store_load(val: i32) -> i32:
     *   mem[100] = val
     *   return mem[100]
     */
    rune_asm_t *a = rune_asm_new();
    rune_type_t p[] = { RUNE_TYPE_I32 };
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t = rune_asm_type(a, 1, p, 1, r);

    rune_asm_memory(a, 1, 2);

    uint32_t fn = rune_asm_func(a, t, 4, 0);
    rune_asm_export_func(a, fn, "store_load");

    rune_asm_begin_code(a, fn);
    /* R1 = 0 (base pointer) */
    rune_asm_ldi32(a, 1, 0);
    /* store32 R0 at [R1 + 100] */
    rune_asm_emit_i(a, OP_STORE32, 0, 1, 0, 100);
    /* load32 R0 from [R1 + 100] */
    rune_asm_emit_i(a, OP_LOAD32, 0, 1, 0, 100);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    rune_vm_t *vm = quick_vm(a);
    assert(vm);

    TEST("store_load(12345) == 12345");
    rune_val_t args[] = { rune_i32(12345) };
    rune_val_t res = rune_void();
    rune_vm_call(vm, "store_load", 1, args, &res);
    ASSERT_EQ_I32(res.as.i32, 12345);

    TEST("store_load(-99) == -99");
    args[0] = rune_i32(-99);
    rune_vm_call(vm, "store_load", 1, args, &res);
    ASSERT_EQ_I32(res.as.i32, -99);

    rune_vm_free(vm);
}

static void test_branching(void) {
    printf("\n[branching / control flow]\n");

    /* fn max(a: i32, b: i32) -> i32:
     *   if a > b: return a
     *   return b
     */
    rune_asm_t *a = rune_asm_new();
    rune_type_t p[] = { RUNE_TYPE_I32, RUNE_TYPE_I32 };
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t = rune_asm_type(a, 2, p, 1, r);
    uint32_t fn = rune_asm_func(a, t, 4, 0);
    rune_asm_export_func(a, fn, "max");

    rune_asm_begin_code(a, fn);
    /* R2 = (R0 > R1) */
    rune_asm_emit(a, OP_GT32, 2, 0, 1);
    /* JZ R2 → else branch */
    uint32_t jz_word = rune_asm_label(a);
    rune_asm_emit_i(a, OP_JZ, 0, 2, 0, 0);  /* placeholder */
    /* then: R0 = R0, RET */
    rune_asm_emit(a, OP_MOV, 0, 0, 0);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    /* else: */
    uint32_t else_word = rune_asm_label(a);
    rune_asm_patch_jump(a, jz_word, else_word);
    rune_asm_emit(a, OP_MOV, 0, 1, 0);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    rune_vm_t *vm = quick_vm(a);
    assert(vm);

    TEST("max(10, 5) == 10");
    rune_val_t args[] = { rune_i32(10), rune_i32(5) };
    rune_val_t res = rune_void();
    rune_vm_call(vm, "max", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, 10);

    TEST("max(3, 7) == 7");
    args[0] = rune_i32(3); args[1] = rune_i32(7);
    rune_vm_call(vm, "max", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, 7);

    TEST("max(4, 4) == 4");
    args[0] = rune_i32(4); args[1] = rune_i32(4);
    rune_vm_call(vm, "max", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, 4);

    rune_vm_free(vm);
}

static void test_host_calls(void) {
    printf("\n[host function calls]\n");

    rune_asm_t *a = rune_asm_new();
    rune_type_t p[] = { RUNE_TYPE_I32 };
    uint16_t t_sink = rune_asm_type(a, 1, p, 0, NULL);
    rune_type_t p2[] = { RUNE_TYPE_I32, RUNE_TYPE_I32 };
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t_fn = rune_asm_type(a, 2, p2, 1, r);

    uint32_t fn_sink = rune_asm_import(a, "test", "sink", t_sink);

    uint32_t fn = rune_asm_func(a, t_fn, 4, 0);
    rune_asm_export_func(a, fn, "call_twice");

    rune_asm_begin_code(a, fn);
    rune_asm_emit(a, OP_ARG, 0, 0, 0);
    rune_asm_emit_i(a, OP_CALL_HOST, 2, 0, 0, fn_sink);
    rune_asm_emit(a, OP_ARG, 0, 1, 0);
    rune_asm_emit_i(a, OP_CALL_HOST, 2, 0, 0, fn_sink);
    rune_asm_emit(a, OP_ADD32, 0, 0, 1);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    size_t sz;
    uint8_t *bin = rune_asm_finalize(a, &sz);
    rune_asm_free(a);

    rune_err_t err;
    rune_module_t *mod = rune_module_load(bin, sz, &err);
    free(bin);
    assert(mod);

    rune_vm_t *vm = rune_vm_new(mod, NULL, &err);
    assert(vm);

    rune_vm_register(vm, "test", "sink", mock_noop, NULL);
    rune_vm_init(vm);

    TEST("call_twice(3, 7) returns 10");
    rune_val_t args[] = { rune_i32(3), rune_i32(7) };
    rune_val_t res = rune_void();
    err = rune_vm_call(vm, "call_twice", 2, args, &res);
    ASSERT_EQ_I32(res.as.i32, 10);

    rune_vm_free(vm);
    rune_module_free(mod);
}

static void test_fuel_limit(void) {
    printf("\n[fuel limiting]\n");

    /* fn loop_forever(): infinite loop */
    rune_asm_t *a = rune_asm_new();
    uint16_t t = rune_asm_type(a, 0, NULL, 0, NULL);
    uint32_t fn = rune_asm_func(a, t, 2, 0);
    rune_asm_export_func(a, fn, "loop");

    rune_asm_begin_code(a, fn);
    uint32_t top = rune_asm_label(a);
    rune_asm_ldi32(a, 0, 1);
    uint32_t jnz = rune_asm_label(a);
    rune_asm_emit_i(a, OP_JNZ, 0, 0, 0, 0);
    rune_asm_patch_jump(a, jnz, top);  /* jnz → top: infinite loop */
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    size_t sz;
    uint8_t *bin = rune_asm_finalize(a, &sz);
    rune_asm_free(a);

    rune_err_t err;
    rune_module_t *mod = rune_module_load(bin, sz, &err);
    free(bin);
    assert(mod);

    rune_config_t cfg = rune_config_default();
    cfg.fuel_limit = 100;   /* only 100 instructions */

    rune_vm_t *vm = rune_vm_new(mod, &cfg, &err);
    rune_vm_init(vm);

    TEST("infinite loop hits fuel limit");
    err = rune_vm_call(vm, "loop", 0, NULL, NULL);
    ASSERT_ERR(err, RUNE_ERR_FUEL);

    rune_vm_free(vm);
    rune_module_free(mod);
}

static void test_div_by_zero(void) {
    printf("\n[traps]\n");

    rune_asm_t *a = rune_asm_new();
    rune_type_t p[] = { RUNE_TYPE_I32, RUNE_TYPE_I32 };
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t = rune_asm_type(a, 2, p, 1, r);
    uint32_t fn = rune_asm_func(a, t, 4, 0);
    rune_asm_export_func(a, fn, "div");

    rune_asm_begin_code(a, fn);
    rune_asm_emit(a, OP_DIV32, 0, 0, 1);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    rune_vm_t *vm = quick_vm(a);
    assert(vm);

    TEST("div(10, 2) == 5");
    rune_val_t args[] = { rune_i32(10), rune_i32(2) };
    rune_val_t res = rune_void();
    rune_err_t err = rune_vm_call(vm, "div", 2, args, &res);
    if (err == RUNE_OK) ASSERT_EQ_I32(res.as.i32, 5);
    else FAIL(rune_err_str(err));

    TEST("div(10, 0) traps with DIVZERO");
    args[0] = rune_i32(10); args[1] = rune_i32(0);
    err = rune_vm_call(vm, "div", 2, args, &res);
    ASSERT_ERR(err, RUNE_ERR_DIVZERO);

    rune_vm_free(vm);
}

static void test_globals(void) {
    printf("\n[globals]\n");

    rune_asm_t *a = rune_asm_new();

    /* global counter = 0 */
    uint32_t g = rune_asm_global(a, RUNE_TYPE_I32, true, rune_i32(0));

    /* fn increment() -> i32: counter += 1; return counter */
    rune_type_t r[] = { RUNE_TYPE_I32 };
    uint16_t t = rune_asm_type(a, 0, NULL, 1, r);
    uint32_t fn = rune_asm_func(a, t, 4, 0);
    rune_asm_export_func(a, fn, "increment");

    rune_asm_begin_code(a, fn);
    rune_asm_emit_i(a, OP_LDGLOBAL, 0, 0, 0, g);   /* R0 = counter */
    rune_asm_ldi32(a, 1, 1);
    rune_asm_emit(a, OP_ADD32, 0, 0, 1);             /* R0 += 1 */
    rune_asm_emit_i(a, OP_STGLOBAL, 0, 0, 0, g);    /* counter = R0 */
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    rune_vm_t *vm = quick_vm(a);
    assert(vm);

    TEST("increment() == 1 first call");
    rune_val_t res = rune_void();
    rune_vm_call(vm, "increment", 0, NULL, &res);
    ASSERT_EQ_I32(res.as.i32, 1);

    TEST("increment() == 2 second call");
    rune_vm_call(vm, "increment", 0, NULL, &res);
    ASSERT_EQ_I32(res.as.i32, 2);

    TEST("increment() == 3 third call");
    rune_vm_call(vm, "increment", 0, NULL, &res);
    ASSERT_EQ_I32(res.as.i32, 3);

    rune_vm_free(vm);
}

/* ─────────────────────────────────────────────
   Main
───────────────────────────────────────────── */
int main(void) {
    printf("Rune VM Test Suite\n");
    printf("==================\n");

    test_add_i32();
    test_arithmetic();
    test_memory();
    test_branching();
    test_host_calls();
    test_fuel_limit();
    test_div_by_zero();
    test_globals();

    printf("\n─────────────────────────────────\n");
    printf("Results: %d/%d passed", g_pass, g_tests);
    if (g_fail) printf(", %d FAILED", g_fail);
    printf("\n");
    return g_fail ? 1 : 0;
}
