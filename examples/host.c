/*
 * examples/host.c — Embedding Rune in a C application
 *
 * This example shows the full lifecycle:
 *   1. Assemble a plugin module in-memory
 *   2. Register host functions
 *   3. Initialize the VM
 *   4. Call exported functions
 *   5. Exchange data through linear memory
 */

#include "../include/rune.h"
#include "../include/rune_asm.h"
#include "../include/rune_bytecode.h"
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

/* ─────────────────────────────────────────────
   Host functions (provided TO the plugin)
───────────────────────────────────────────── */

/* env::print_i32(value: i32) */
static rune_err_t host_print_i32(rune_vm_t *vm, int argc, rune_val_t *argv,
                                   rune_val_t *ret, void *ud) {
    (void)vm; (void)ud;
    if (argc < 1 || argv[0].type != RUNE_TYPE_I32) return RUNE_ERR_TYPE;
    printf("[plugin] %d\n", argv[0].as.i32);
    *ret = rune_void();
    return RUNE_OK;
}

/* env::print_str(ptr: ptr, len: i32) */
static rune_err_t host_print_str(rune_vm_t *vm, int argc, rune_val_t *argv,
                                  rune_val_t *ret, void *ud) {
    (void)ud;
    if (argc < 2) return RUNE_ERR_TYPE;

    uint32_t ptr = argv[0].as.ptr;
    int32_t  len = argv[1].as.i32;
    if (len < 0 || len > 65536) return RUNE_ERR_BOUNDS;

    char *buf = malloc((size_t)len + 1);
    rune_err_t err = rune_vm_mem_read(vm, ptr, buf, (size_t)len);
    if (err != RUNE_OK) { free(buf); return err; }
    buf[len] = '\0';
    printf("[plugin] %s\n", buf);
    free(buf);

    *ret = rune_void();
    return RUNE_OK;
}

/* env::get_time() -> i64  (returns fake timestamp for demo) */
static rune_err_t host_get_time(rune_vm_t *vm, int argc, rune_val_t *argv,
                                  rune_val_t *ret, void *ud) {
    (void)vm; (void)argc; (void)argv; (void)ud;
    *ret = rune_i64(1708560000LL);  /* 2024-02-22 00:00:00 UTC */
    return RUNE_OK;
}

/* ─────────────────────────────────────────────
   Build a plugin module (in-memory, for demo)
───────────────────────────────────────────── */

/*
 * This builds a module equivalent to:
 *
 *   import env::print_i32(i32)
 *   import env::print_str(ptr, i32)
 *   import env::get_time() -> i64
 *
 *   memory 1 page
 *
 *   data[0] = "Hello from Rune!\0"
 *
 *   export fn compute(a: i32, b: i32) -> i32:
 *     let sum = a + b
 *     print_i32(sum)
 *     return sum
 *
 *   export fn greet():
 *     print_str(0, 16)   // prints the data at offset 0
 *
 *   export fn timestamp() -> i64:
 *     return get_time()
 */
static uint8_t *build_demo_module(size_t *out_size) {
    rune_asm_t *a = rune_asm_new();

    /* Types */
    rune_type_t p_i32[]    = { RUNE_TYPE_I32 };
    rune_type_t p_ptr_i32[]= { RUNE_TYPE_PTR, RUNE_TYPE_I32 };
    rune_type_t r_i32[]    = { RUNE_TYPE_I32 };
    rune_type_t r_i64[]    = { RUNE_TYPE_I64 };
    rune_type_t p_2i32[]   = { RUNE_TYPE_I32, RUNE_TYPE_I32 };

    uint16_t t_void_void   = rune_asm_type(a, 0, NULL,       0, NULL);
    uint16_t t_i32_void    = rune_asm_type(a, 1, p_i32,      0, NULL);
    uint16_t t_ptr_i32_void= rune_asm_type(a, 2, p_ptr_i32,  0, NULL);
    uint16_t t_void_i64    = rune_asm_type(a, 0, NULL,       1, r_i64);
    uint16_t t_2i32_i32    = rune_asm_type(a, 2, p_2i32,     1, r_i32);
    (void)t_void_void;

    /* Imports */
    uint32_t fn_print_i32 = rune_asm_import(a, "env", "print_i32", t_i32_void);
    uint32_t fn_print_str = rune_asm_import(a, "env", "print_str", t_ptr_i32_void);
    uint32_t fn_get_time  = rune_asm_import(a, "env", "get_time",  t_void_i64);

    /* Memory: 1 page = 64KiB */
    rune_asm_memory(a, 1, 4);
    rune_asm_export_memory(a, "memory");

    /* Data: "Hello from Rune!" at offset 0 */
    const char *greeting = "Hello from Rune!";
    rune_asm_data(a, 0, greeting, (uint32_t)strlen(greeting));

    /* fn compute(a: i32, b: i32) -> i32  [func idx = 3] */
    /*   Registers: R0=a, R1=b, R2=sum, R3=tmp            */
    uint32_t fn_compute = rune_asm_func(a, t_2i32_i32, 4, 0);
    rune_asm_export_func(a, fn_compute, "compute");

    rune_asm_begin_code(a, fn_compute);
    /*  R2 = R0 + R1  */
    rune_asm_emit(a, OP_ADD32, 2, 0, 1);
    /*  ARG 0 = R2    */
    rune_asm_emit(a, OP_ARG, 0, 2, 0);
    /*  call print_i32(R2) -> discard */
    rune_asm_emit_i(a, OP_CALL_HOST, 3, 0, 0, fn_print_i32);
    /*  R0 = R2 (return value) */
    rune_asm_emit(a, OP_MOV, 0, 2, 0);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    /* fn greet() */
    uint16_t t_void_void2 = rune_asm_type(a, 0, NULL, 0, NULL);
    uint32_t fn_greet = rune_asm_func(a, t_void_void2, 4, 0);
    rune_asm_export_func(a, fn_greet, "greet");

    rune_asm_begin_code(a, fn_greet);
    /* R0 = 0 (ptr to greeting)     */
    rune_asm_ldi32(a, 0, 0);
    /* R1 = 16 (length)             */
    rune_asm_ldi32(a, 1, (int32_t)strlen(greeting));
    /* ARG 0 = R0, ARG 1 = R1       */
    rune_asm_emit(a, OP_ARG, 0, 0, 0);
    rune_asm_emit(a, OP_ARG, 1, 1, 0);
    rune_asm_emit_i(a, OP_CALL_HOST, 2, 0, 0, fn_print_str);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    /* fn timestamp() -> i64 */
    uint32_t fn_ts = rune_asm_func(a, t_void_i64, 2, 0);
    rune_asm_export_func(a, fn_ts, "timestamp");

    rune_asm_begin_code(a, fn_ts);
    rune_asm_emit_i(a, OP_CALL_HOST, 0, 0, 0, fn_get_time);
    rune_asm_emit(a, OP_RET, 0, 0, 0);
    rune_asm_end_code(a);

    uint8_t *binary = rune_asm_finalize(a, out_size);
    rune_asm_free(a);
    return binary;
}

/* ─────────────────────────────────────────────
   Main
───────────────────────────────────────────── */

int main(void) {
    printf("=== Rune Host Example ===\n\n");

    /* 1. Build module */
    size_t   mod_size;
    uint8_t *mod_data = build_demo_module(&mod_size);
    printf("Module size: %zu bytes\n", mod_size);

    /* 2. Load module */
    rune_err_t err;
    rune_module_t *mod = rune_module_load(mod_data, mod_size, &err);
    if (!mod) {
        fprintf(stderr, "Failed to load module: %s\n", rune_err_str(err));
        free(mod_data);
        return 1;
    }
    free(mod_data);

    /* Print imports/exports */
    printf("Imports (%d):\n", rune_module_import_count(mod));
    for (int i = 0; i < rune_module_import_count(mod); i++)
        printf("  %s::%s\n",
               rune_module_import_module(mod, i),
               rune_module_import_name(mod, i));

    printf("Exports (%d):\n", rune_module_export_count(mod));
    for (int i = 0; i < rune_module_export_count(mod); i++)
        printf("  %s\n", rune_module_export_name(mod, i));
    printf("\n");

    /* 3. Create VM */
    rune_config_t cfg = rune_config_default();
    cfg.fuel_limit = 1000000;  /* 1M instructions max per call */

    rune_vm_t *vm = rune_vm_new(mod, &cfg, &err);
    if (!vm) {
        fprintf(stderr, "Failed to create VM: %s\n", rune_err_str(err));
        rune_module_free(mod);
        return 1;
    }

    /* 4. Register host functions */
    rune_vm_register(vm, "env", "print_i32", host_print_i32, NULL);
    rune_vm_register(vm, "env", "print_str", host_print_str, NULL);
    rune_vm_register(vm, "env", "get_time",  host_get_time,  NULL);

    /* 5. Initialize */
    err = rune_vm_init(vm);
    if (err != RUNE_OK) {
        fprintf(stderr, "VM init failed: %s — %s\n",
                rune_err_str(err), rune_vm_last_error(vm));
        rune_vm_free(vm);
        rune_module_free(mod);
        return 1;
    }

    /* 6. Call exported functions */

    /* compute(10, 32) -> should print 42 and return 42 */
    printf("--- compute(10, 32) ---\n");
    rune_val_t args[] = { rune_i32(10), rune_i32(32) };
    rune_val_t result = rune_void();
    err = rune_vm_call(vm, "compute", 2, args, &result);
    if (err != RUNE_OK) {
        fprintf(stderr, "compute() failed: %s\n", rune_vm_last_error(vm));
    } else {
        printf("compute returned: %d\n", result.as.i32);
    }

    /* greet() */
    printf("\n--- greet() ---\n");
    err = rune_vm_call(vm, "greet", 0, NULL, NULL);
    if (err != RUNE_OK)
        fprintf(stderr, "greet() failed: %s\n", rune_vm_last_error(vm));

    /* timestamp() */
    printf("\n--- timestamp() ---\n");
    rune_val_t ts = rune_void();
    err = rune_vm_call(vm, "timestamp", 0, NULL, &ts);
    if (err != RUNE_OK) {
        fprintf(stderr, "timestamp() failed: %s\n", rune_vm_last_error(vm));
    } else {
        printf("timestamp returned: %lld\n", (long long)ts.as.i64);
    }

    printf("\nFuel used: %llu instructions\n", (unsigned long long)rune_vm_fuel_used(vm));

    /* 7. Cleanup */
    rune_vm_free(vm);
    rune_module_free(mod);

    printf("\nDone.\n");
    return 0;
}
