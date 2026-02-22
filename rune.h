/**
 * rune.h — Rune Runtime C Embedding API v0.1
 *
 * Opaque-handle API for embedding Rune in any language with a C FFI.
 * See src/ffi.rs for the Rust implementation (Phase 2).
 */

#ifndef RUNE_H
#define RUNE_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Opaque handles ────────────────────────────────────────────────────────── */

typedef struct RuneRuntime  RuneRuntime;
typedef struct RuneModule   RuneModule;
typedef struct RuneInstance RuneInstance;

/* ── Error codes ───────────────────────────────────────────────────────────── */

typedef enum {
    RUNE_OK                  = 0,
    RUNE_INVALID_MODULE      = 1,
    RUNE_OUT_OF_MEMORY       = 2,
    RUNE_TRAP_OUT_OF_BOUNDS  = 3,
    RUNE_TRAP_DIV_ZERO       = 4,
    RUNE_TRAP_UNREACHABLE    = 5,
    RUNE_TRAP_STACK_OVERFLOW = 6,
    RUNE_TRAP_TYPE_MISMATCH  = 7,
    RUNE_UNDEFINED_EXPORT    = 8,
    RUNE_UNDEFINED_IMPORT    = 9,
    RUNE_HOST_ERROR          = 10,
} RuneError;

/* ── Value types ───────────────────────────────────────────────────────────── */

typedef enum {
    RUNE_I32 = 0x7F,
    RUNE_I64 = 0x7E,
    RUNE_F32 = 0x7D,
    RUNE_F64 = 0x7C,
} RuneValType;

typedef union {
    int32_t  i32;
    int64_t  i64;
    float    f32;
    double   f64;
} RuneVal;

/* ── Host function callback ────────────────────────────────────────────────── */

/**
 * A host function registered via rune_module_register_host().
 *
 * @param instance  The calling instance (for memory access).
 * @param args      Argument values array.
 * @param n_args    Number of arguments.
 * @param result    Write return value here (if any).
 * @param user_data Opaque pointer passed at registration time.
 * @return RUNE_OK on success, or an error code.
 */
typedef RuneError (*RuneHostFn)(
    RuneInstance      *instance,
    const RuneVal     *args,
    size_t             n_args,
    RuneVal           *result,
    void              *user_data
);

/* ── Runtime lifecycle ─────────────────────────────────────────────────────── */

/** Create a new runtime. Must be freed with rune_runtime_free(). */
RuneRuntime *rune_runtime_new(void);

/** Free a runtime. All modules/instances must be freed first. */
void         rune_runtime_free(RuneRuntime *rt);

/* ── Module loading ────────────────────────────────────────────────────────── */

/** Load a module from a .rune file. Returns NULL on error. */
RuneModule *rune_module_load_file(RuneRuntime *rt, const char *path);

/** Load a module from a byte buffer. Returns NULL on error. */
RuneModule *rune_module_load_bytes(RuneRuntime *rt, const uint8_t *data, size_t len);

/** Free a module. */
void        rune_module_free(RuneModule *mod);

/* ── Host function registration ────────────────────────────────────────────── */

/**
 * Register a host function that guest code can call via CallHost.
 * Must be called before rune_instance_new().
 *
 * @param mod          The module to register the function on.
 * @param name         Name used to resolve imports.
 * @param param_types  Array of parameter types.
 * @param n_params     Number of parameters.
 * @param result_type  Return type (0 = void).
 * @param func         The host callback.
 * @param user_data    Opaque pointer forwarded to the callback.
 */
RuneError rune_module_register_host(
    RuneModule    *mod,
    const char    *name,
    RuneValType   *param_types,
    size_t         n_params,
    RuneValType    result_type,
    RuneHostFn     func,
    void          *user_data
);

/* ── Instantiation ─────────────────────────────────────────────────────────── */

/** Create a new instance of a module. Returns NULL on error. */
RuneInstance *rune_instance_new(RuneModule *mod);

/** Free an instance. */
void          rune_instance_free(RuneInstance *inst);

/* ── Function calls ────────────────────────────────────────────────────────── */

/**
 * Call an exported function by name.
 *
 * @param inst      The instance.
 * @param func_name Exported function name.
 * @param args      Argument values.
 * @param n_args    Number of arguments.
 * @param result    Written with the return value (may be NULL for void).
 * @return RUNE_OK, or a trap/error code.
 */
RuneError rune_call(
    RuneInstance  *inst,
    const char    *func_name,
    const RuneVal *args,
    size_t         n_args,
    RuneVal       *result
);

/* ── Memory access ─────────────────────────────────────────────────────────── */

/** Return a pointer to the instance's linear memory base (zero-copy). */
uint8_t *rune_memory_base(RuneInstance *inst);

/** Return the current size of linear memory in bytes. */
size_t   rune_memory_size(RuneInstance *inst);

/** Grow linear memory by delta_pages 64KB pages. Returns RUNE_OUT_OF_MEMORY on failure. */
RuneError rune_memory_grow(RuneInstance *inst, size_t delta_pages);

/** Bounds-checked read from linear memory. */
RuneError rune_memory_read(RuneInstance *inst, size_t offset, void *dst, size_t len);

/** Bounds-checked write to linear memory. */
RuneError rune_memory_write(RuneInstance *inst, size_t offset, const void *src, size_t len);

/* ── Diagnostics ───────────────────────────────────────────────────────────── */

/** Return a human-readable string for an error code. */
const char *rune_error_string(RuneError err);

#ifdef __cplusplus
}
#endif

#endif /* RUNE_H */
