/*
 * vm.c — Rune VM: module loading, memory management, interpreter
 */

#include "../include/rune.h"
#include "../include/rune_bytecode.h"

#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <math.h>
#include <assert.h>
#include <stdarg.h>

/* strdup is POSIX; provide fallback if needed */
#ifndef _POSIX_C_SOURCE
static char *_rune_strdup(const char *s) {
    size_t n = strlen(s) + 1;
    char *p = malloc(n);
    if (p) memcpy(p, s, n);
    return p;
}
#define strdup _rune_strdup
#endif

/* ─────────────────────────────────────────────
   Internal Structures
───────────────────────────────────────────── */

typedef struct {
    rune_type_t  *param_types;
    rune_type_t  *return_types;
    uint8_t       param_count;
    uint8_t       return_count;
} rune_func_type_t;

typedef struct {
    char    *module;
    char    *name;
    uint16_t type_idx;
} rune_import_t;

typedef struct {
    uint16_t  type_idx;
    uint8_t   reg_count;
    uint8_t   local_count;
    const uint8_t *code;      /* pointer into module data */
    uint32_t  code_size;
    bool      is_import;
    uint32_t  import_idx;     /* if is_import */
} rune_func_t;

typedef struct {
    rune_type_t type;
    bool        mutable;
    rune_val_t  value;
} rune_global_t;

typedef struct {
    rune_export_kind_t kind;
    uint32_t           idx;
    char              *name;
} rune_export_t;

struct rune_module {
    /* Raw data (owned) */
    uint8_t *data;
    size_t   data_len;

    /* Parsed sections */
    rune_func_type_t *types;
    uint32_t          type_count;

    rune_import_t    *imports;
    uint32_t          import_count;

    rune_func_t      *funcs;
    uint32_t          func_count;   /* includes imports */

    rune_global_t    *globals;
    uint32_t          global_count;

    rune_export_t    *exports;
    uint32_t          export_count;

    /* Memory section */
    uint16_t          mem_initial_pages;
    uint16_t          mem_max_pages;
    bool              has_memory;

    /* Data segments */
    struct {
        uint32_t       offset;
        const uint8_t *data;
        uint32_t       size;
    } *data_segs;
    uint32_t data_seg_count;

    /* Init function index (-1 if none) */
    int32_t  init_func;
};

/* ─── Host function registration ─── */
typedef struct {
    char          *module;
    char          *name;
    rune_host_fn_t fn;
    void          *userdata;
} rune_host_entry_t;

/* ─── Call frame ─── */
typedef struct {
    uint32_t    func_idx;
    uint32_t    pc;         /* instruction offset (in 4-byte units) */
    rune_val_t *regs;       /* heap-allocated register window */
    uint8_t     reg_count;
} rune_frame_t;

struct rune_vm {
    rune_module_t *mod;
    rune_config_t  cfg;

    /* Host function table */
    rune_host_entry_t *host_fns;
    uint32_t           host_fn_count;
    uint32_t           host_fn_cap;

    /* Linear memory */
    uint8_t  *memory;
    uint32_t  memory_pages;   /* current pages */
    uint32_t  memory_max;     /* max pages */

    /* Globals (copied from module, mutable) */
    rune_val_t *globals;

    /* Call stack */
    rune_frame_t *frames;
    uint32_t      frame_count;

    /* Argument staging buffer for calls */
    rune_val_t arg_buf[RUNE_MAX_PARAMS];
    uint8_t    arg_count;

    /* Diagnostics */
    char     error_buf[256];
    uint64_t fuel_used;
    bool     initialized;
};

/* ─────────────────────────────────────────────
   Error helpers
───────────────────────────────────────────── */
const char *rune_err_str(rune_err_t err) {
    switch (err) {
        case RUNE_OK:               return "OK";
        case RUNE_ERR_BADMODULE:    return "bad module";
        case RUNE_ERR_BADMAGIC:     return "bad magic";
        case RUNE_ERR_VERSION:      return "version mismatch";
        case RUNE_ERR_OOM:          return "out of memory";
        case RUNE_ERR_BOUNDS:       return "memory out of bounds";
        case RUNE_ERR_DIVZERO:      return "division by zero";
        case RUNE_ERR_TYPE:         return "type mismatch";
        case RUNE_ERR_NOEXPORT:     return "export not found";
        case RUNE_ERR_NOIMPORT:     return "unresolved import";
        case RUNE_ERR_STACKOVERFLOW: return "call stack overflow";
        case RUNE_ERR_TRAP:         return "trap";
        case RUNE_ERR_FUEL:         return "fuel exhausted";
        case RUNE_ERR_BADOPCODE:    return "unknown opcode";
        default:                    return "unknown error";
    }
}

static void vm_set_error(rune_vm_t *vm, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(vm->error_buf, sizeof(vm->error_buf), fmt, ap);
    va_end(ap);
}

/* ─────────────────────────────────────────────
   CRC-32 (ISO 3309)
───────────────────────────────────────────── */
static uint32_t crc32_update(uint32_t crc, const uint8_t *data, size_t len) {
    crc = ~crc;
    for (size_t i = 0; i < len; i++) {
        crc ^= data[i];
        for (int j = 0; j < 8; j++)
            crc = (crc >> 1) ^ (0xEDB88320u & -(crc & 1));
    }
    return ~crc;
}

/* ─────────────────────────────────────────────
   Reader helpers (little-endian)
───────────────────────────────────────────── */
typedef struct {
    const uint8_t *base;
    size_t         len;
    size_t         pos;
    bool           error;
} reader_t;

static void reader_init(reader_t *r, const uint8_t *data, size_t len) {
    r->base = data; r->len = len; r->pos = 0; r->error = false;
}

static uint8_t read_u8(reader_t *r) {
    if (r->pos >= r->len) { r->error = true; return 0; }
    return r->base[r->pos++];
}

static uint16_t read_u16(reader_t *r) {
    uint16_t v = 0;
    v |= (uint16_t)read_u8(r);
    v |= (uint16_t)read_u8(r) << 8;
    return v;
}

static uint32_t read_u32(reader_t *r) {
    uint32_t v = 0;
    v |= (uint32_t)read_u8(r);
    v |= (uint32_t)read_u8(r) << 8;
    v |= (uint32_t)read_u8(r) << 16;
    v |= (uint32_t)read_u8(r) << 24;
    return v;
}

static uint64_t read_u64(reader_t *r) {
    uint64_t lo = read_u32(r);
    uint64_t hi = read_u32(r);
    return lo | (hi << 32);
}

static const uint8_t *read_bytes(reader_t *r, size_t n) {
    if (r->pos + n > r->len) { r->error = true; return NULL; }
    const uint8_t *p = r->base + r->pos;
    r->pos += n;
    return p;
}

static char *read_string(reader_t *r, uint8_t len_bytes) {
    uint32_t len = (len_bytes == 1) ? read_u8(r) : read_u32(r);
    if (r->error || len > 4096) { r->error = true; return NULL; }
    char *s = malloc(len + 1);
    if (!s) { r->error = true; return NULL; }
    const uint8_t *src = read_bytes(r, len);
    if (!src) { free(s); r->error = true; return NULL; }
    memcpy(s, src, len);
    s[len] = '\0';
    return s;
}

/* ─────────────────────────────────────────────
   Module Loading
───────────────────────────────────────────── */

static rune_err_t parse_type_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t count = read_u32(r);
    if (r->error || count > RUNE_MAX_FUNCS) return RUNE_ERR_BADMODULE;

    mod->types = calloc(count, sizeof(rune_func_type_t));
    if (!mod->types) return RUNE_ERR_OOM;
    mod->type_count = count;

    for (uint32_t i = 0; i < count; i++) {
        uint8_t pc = read_u8(r);
        uint8_t rc = read_u8(r);
        if (r->error || pc > RUNE_MAX_PARAMS || rc > 1) return RUNE_ERR_BADMODULE;

        rune_func_type_t *t = &mod->types[i];
        t->param_count  = pc;
        t->return_count = rc;

        if (pc > 0) {
            t->param_types = malloc(pc);
            if (!t->param_types) return RUNE_ERR_OOM;
            for (int j = 0; j < pc; j++) t->param_types[j] = (rune_type_t)read_u8(r);
        }
        if (rc > 0) {
            t->return_types = malloc(rc);
            if (!t->return_types) return RUNE_ERR_OOM;
            for (int j = 0; j < rc; j++) t->return_types[j] = (rune_type_t)read_u8(r);
        }
        if (r->error) return RUNE_ERR_BADMODULE;
    }
    return RUNE_OK;
}

static rune_err_t parse_import_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t count = read_u32(r);
    if (r->error || count > RUNE_MAX_FUNCS) return RUNE_ERR_BADMODULE;

    mod->imports = calloc(count, sizeof(rune_import_t));
    if (!mod->imports) return RUNE_ERR_OOM;
    mod->import_count = count;

    for (uint32_t i = 0; i < count; i++) {
        mod->imports[i].module   = read_string(r, 1);
        mod->imports[i].name     = read_string(r, 1);
        mod->imports[i].type_idx = read_u16(r);
        if (r->error) return RUNE_ERR_BADMODULE;
    }
    return RUNE_OK;
}

static rune_err_t parse_func_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t body_count = read_u32(r);
    if (r->error) return RUNE_ERR_BADMODULE;

    uint32_t total = mod->import_count + body_count;
    if (total > RUNE_MAX_FUNCS) return RUNE_ERR_BADMODULE;

    mod->funcs = calloc(total, sizeof(rune_func_t));
    if (!mod->funcs) return RUNE_ERR_OOM;
    mod->func_count = total;

    /* Mark imports */
    for (uint32_t i = 0; i < mod->import_count; i++) {
        mod->funcs[i].is_import  = true;
        mod->funcs[i].import_idx = i;
        mod->funcs[i].type_idx   = mod->imports[i].type_idx;
    }

    /* Read non-import function descriptors */
    for (uint32_t i = 0; i < body_count; i++) {
        rune_func_t *f = &mod->funcs[mod->import_count + i];
        f->type_idx   = read_u16(r);
        f->reg_count  = read_u8(r);
        f->local_count= read_u8(r);
        if (r->error) return RUNE_ERR_BADMODULE;
    }
    return RUNE_OK;
}

static rune_err_t parse_memory_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    mod->mem_initial_pages = read_u16(r);
    mod->mem_max_pages     = read_u16(r);
    if (r->error) return RUNE_ERR_BADMODULE;
    mod->has_memory = true;
    return RUNE_OK;
}

static rune_err_t parse_global_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t count = read_u32(r);
    if (r->error || count > RUNE_MAX_GLOBALS) return RUNE_ERR_BADMODULE;

    mod->globals = calloc(count, sizeof(rune_global_t));
    if (!mod->globals) return RUNE_ERR_OOM;
    mod->global_count = count;

    for (uint32_t i = 0; i < count; i++) {
        rune_global_t *g = &mod->globals[i];
        g->type    = (rune_type_t)read_u8(r);
        g->mutable = read_u8(r) != 0;
        uint64_t raw = read_u64(r);
        if (r->error) return RUNE_ERR_BADMODULE;
        switch (g->type) {
            case RUNE_TYPE_I32: g->value = rune_i32((int32_t)raw); break;
            case RUNE_TYPE_I64: g->value = rune_i64((int64_t)raw); break;
            case RUNE_TYPE_F32: { float f; memcpy(&f, &raw, 4); g->value = rune_f32(f); break; }
            case RUNE_TYPE_F64: { double d; memcpy(&d, &raw, 8); g->value = rune_f64(d); break; }
            default: return RUNE_ERR_BADMODULE;
        }
    }
    return RUNE_OK;
}

static rune_err_t parse_export_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t count = read_u32(r);
    if (r->error || count > 65536) return RUNE_ERR_BADMODULE;

    mod->exports = calloc(count, sizeof(rune_export_t));
    if (!mod->exports) return RUNE_ERR_OOM;
    mod->export_count = count;

    for (uint32_t i = 0; i < count; i++) {
        rune_export_t *e = &mod->exports[i];
        e->kind = (rune_export_kind_t)read_u8(r);
        e->idx  = read_u32(r);
        e->name = read_string(r, 1);
        if (r->error || !e->name) return RUNE_ERR_BADMODULE;
    }
    return RUNE_OK;
}

static rune_err_t parse_code_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t count = read_u32(r);
    uint32_t expected = mod->func_count - mod->import_count;
    if (r->error || count != expected) return RUNE_ERR_BADMODULE;

    for (uint32_t i = 0; i < count; i++) {
        uint32_t body_size = read_u32(r);
        if (r->error || body_size % 4 != 0) return RUNE_ERR_BADMODULE;
        const uint8_t *code = read_bytes(r, body_size);
        if (!code) return RUNE_ERR_BADMODULE;

        rune_func_t *f = &mod->funcs[mod->import_count + i];
        f->code      = code;
        f->code_size = body_size;
    }
    return RUNE_OK;
}

static rune_err_t parse_data_section(rune_module_t *mod, reader_t *r, uint32_t size) {
    uint32_t count = read_u32(r);
    if (r->error || count > 4096) return RUNE_ERR_BADMODULE;

    mod->data_segs = calloc(count, sizeof(*mod->data_segs));
    if (!mod->data_segs) return RUNE_ERR_OOM;
    mod->data_seg_count = count;

    for (uint32_t i = 0; i < count; i++) {
        /* uint8_t mem_idx = */ read_u8(r);
        uint32_t offset = read_u32(r);
        uint32_t seg_size = read_u32(r);
        const uint8_t *data = read_bytes(r, seg_size);
        if (r->error || !data) return RUNE_ERR_BADMODULE;

        mod->data_segs[i].offset = offset;
        mod->data_segs[i].data   = data;
        mod->data_segs[i].size   = seg_size;
    }
    return RUNE_OK;
}

rune_module_t *rune_module_load(const void *raw, size_t len, rune_err_t *out_err) {
#define SET_ERR(e) do { if (out_err) *out_err = (e); } while(0)

    if (!raw || len < sizeof(rune_file_header_t)) { SET_ERR(RUNE_ERR_BADMODULE); return NULL; }

    /* Validate magic */
    if (memcmp(raw, RUNE_MAGIC, RUNE_MAGIC_LEN) != 0) { SET_ERR(RUNE_ERR_BADMAGIC); return NULL; }

    const rune_file_header_t *hdr = (const rune_file_header_t *)raw;
    if (hdr->version != RUNE_BC_VERSION) { SET_ERR(RUNE_ERR_VERSION); return NULL; }

    /* Verify CRC */
    uint32_t computed = crc32_update(0,
        (const uint8_t *)raw + sizeof(rune_file_header_t),
        len - sizeof(rune_file_header_t));
    if (computed != hdr->crc32) { SET_ERR(RUNE_ERR_BADMODULE); return NULL; }

    rune_module_t *mod = calloc(1, sizeof(rune_module_t));
    if (!mod) { SET_ERR(RUNE_ERR_OOM); return NULL; }

    /* Copy data so module owns it */
    mod->data = malloc(len);
    if (!mod->data) { free(mod); SET_ERR(RUNE_ERR_OOM); return NULL; }
    memcpy(mod->data, raw, len);
    mod->data_len   = len;
    mod->init_func  = -1;

    /* Parse sections */
    reader_t r;
    reader_init(&r, mod->data + sizeof(rune_file_header_t),
                len - sizeof(rune_file_header_t));

    rune_err_t err = RUNE_OK;
    while (!r.error && r.pos < r.len) {
        uint8_t  sect_id   = read_u8(&r);
        uint32_t sect_size = read_u32(&r);
        if (r.error) { err = RUNE_ERR_BADMODULE; break; }

        size_t sect_start = r.pos;

        switch ((rune_sect_id_t)sect_id) {
            case RUNE_SECT_TYPE:   err = parse_type_section  (mod, &r, sect_size); break;
            case RUNE_SECT_IMPORT: err = parse_import_section(mod, &r, sect_size); break;
            case RUNE_SECT_FUNC:   err = parse_func_section  (mod, &r, sect_size); break;
            case RUNE_SECT_MEMORY: err = parse_memory_section(mod, &r, sect_size); break;
            case RUNE_SECT_GLOBAL: err = parse_global_section(mod, &r, sect_size); break;
            case RUNE_SECT_EXPORT: err = parse_export_section(mod, &r, sect_size); break;
            case RUNE_SECT_CODE:   err = parse_code_section  (mod, &r, sect_size); break;
            case RUNE_SECT_DATA:   err = parse_data_section  (mod, &r, sect_size); break;
            default:
                /* Skip unknown sections */
                read_bytes(&r, sect_size);
                break;
        }

        if (err != RUNE_OK) break;

        /* Ensure we consumed exactly sect_size bytes */
        size_t consumed = r.pos - sect_start;
        if (consumed < sect_size)
            read_bytes(&r, sect_size - consumed);  /* skip remainder */
    }

    if (err != RUNE_OK || r.error) {
        rune_module_free(mod);
        SET_ERR(err != RUNE_OK ? err : RUNE_ERR_BADMODULE);
        return NULL;
    }

    /* Look for an "_init" export */
    for (uint32_t i = 0; i < mod->export_count; i++) {
        if (mod->exports[i].kind == RUNE_EXPORT_FUNC &&
            strcmp(mod->exports[i].name, "_init") == 0) {
            mod->init_func = (int32_t)mod->exports[i].idx;
            break;
        }
    }

    SET_ERR(RUNE_OK);
    return mod;
#undef SET_ERR
}

rune_module_t *rune_module_load_file(const char *path, rune_err_t *out_err) {
    FILE *f = fopen(path, "rb");
    if (!f) { if (out_err) *out_err = RUNE_ERR_BADMODULE; return NULL; }
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size <= 0) { fclose(f); if (out_err) *out_err = RUNE_ERR_BADMODULE; return NULL; }

    uint8_t *buf = malloc((size_t)size);
    if (!buf) { fclose(f); if (out_err) *out_err = RUNE_ERR_OOM; return NULL; }
    fread(buf, 1, (size_t)size, f);
    fclose(f);

    rune_module_t *mod = rune_module_load(buf, (size_t)size, out_err);
    free(buf);
    return mod;
}

void rune_module_free(rune_module_t *mod) {
    if (!mod) return;

    for (uint32_t i = 0; i < mod->type_count; i++) {
        free(mod->types[i].param_types);
        free(mod->types[i].return_types);
    }
    free(mod->types);

    for (uint32_t i = 0; i < mod->import_count; i++) {
        free(mod->imports[i].module);
        free(mod->imports[i].name);
    }
    free(mod->imports);
    free(mod->funcs);
    free(mod->globals);

    for (uint32_t i = 0; i < mod->export_count; i++)
        free(mod->exports[i].name);
    free(mod->exports);

    free(mod->data_segs);
    free(mod->data);
    free(mod);
}

int rune_module_export_count(const rune_module_t *mod) {
    return (int)mod->export_count;
}
const char *rune_module_export_name(const rune_module_t *mod, int idx) {
    if (idx < 0 || (uint32_t)idx >= mod->export_count) return NULL;
    return mod->exports[idx].name;
}
int rune_module_import_count(const rune_module_t *mod) {
    return (int)mod->import_count;
}
const char *rune_module_import_module(const rune_module_t *mod, int idx) {
    if (idx < 0 || (uint32_t)idx >= mod->import_count) return NULL;
    return mod->imports[idx].module;
}
const char *rune_module_import_name(const rune_module_t *mod, int idx) {
    if (idx < 0 || (uint32_t)idx >= mod->import_count) return NULL;
    return mod->imports[idx].name;
}

/* ─────────────────────────────────────────────
   VM Creation
───────────────────────────────────────────── */

rune_config_t rune_config_default(void) {
    return (rune_config_t){
        .stack_size   = RUNE_CALL_DEPTH,
        .memory_limit = 64u * 1024u * 1024u,
        .fuel_limit   = 0,
    };
}

rune_vm_t *rune_vm_new(rune_module_t *mod, const rune_config_t *cfg, rune_err_t *out_err) {
    rune_vm_t *vm = calloc(1, sizeof(rune_vm_t));
    if (!vm) { if (out_err) *out_err = RUNE_ERR_OOM; return NULL; }

    vm->mod = mod;
    vm->cfg = cfg ? *cfg : rune_config_default();

    vm->host_fn_cap  = 16;
    vm->host_fns     = malloc(vm->host_fn_cap * sizeof(rune_host_entry_t));

    vm->frames       = calloc(vm->cfg.stack_size, sizeof(rune_frame_t));

    if (!vm->host_fns || !vm->frames) {
        rune_vm_free(vm);
        if (out_err) *out_err = RUNE_ERR_OOM;
        return NULL;
    }

    if (out_err) *out_err = RUNE_OK;
    return vm;
}

void rune_vm_free(rune_vm_t *vm) {
    if (!vm) return;
    /* Free call stack register windows */
    for (uint32_t i = 0; i < vm->cfg.stack_size; i++)
        if (vm->frames[i].regs) free(vm->frames[i].regs);
    free(vm->frames);

    for (uint32_t i = 0; i < vm->host_fn_count; i++) {
        free(vm->host_fns[i].module);
        free(vm->host_fns[i].name);
    }
    free(vm->host_fns);
    free(vm->memory);
    free(vm->globals);
    free(vm);
}

rune_err_t rune_vm_register(rune_vm_t *vm, const char *mod_name,
                             const char *name, rune_host_fn_t fn, void *ud) {
    if (vm->initialized) return RUNE_ERR_BADMODULE;

    if (vm->host_fn_count >= vm->host_fn_cap) {
        vm->host_fn_cap *= 2;
        rune_host_entry_t *tmp = realloc(vm->host_fns,
            vm->host_fn_cap * sizeof(rune_host_entry_t));
        if (!tmp) return RUNE_ERR_OOM;
        vm->host_fns = tmp;
    }

    rune_host_entry_t *e = &vm->host_fns[vm->host_fn_count++];
    e->module   = strdup(mod_name);
    e->name     = strdup(name);
    e->fn       = fn;
    e->userdata = ud;
    if (!e->module || !e->name) return RUNE_ERR_OOM;
    return RUNE_OK;
}

/* ─────────────────────────────────────────────
   VM Init — resolve imports, allocate memory, run _init
───────────────────────────────────────────── */

/* Forward declaration */
static rune_err_t vm_exec(rune_vm_t *vm, uint32_t func_idx,
                           int argc, rune_val_t *argv, rune_val_t *ret);

rune_err_t rune_vm_init(rune_vm_t *vm) {
    rune_module_t *mod = vm->mod;

    /* Verify all imports are satisfied */
    for (uint32_t i = 0; i < mod->import_count; i++) {
        bool found = false;
        for (uint32_t j = 0; j < vm->host_fn_count; j++) {
            if (strcmp(vm->host_fns[j].module, mod->imports[i].module) == 0 &&
                strcmp(vm->host_fns[j].name,   mod->imports[i].name)   == 0) {
                found = true;
                break;
            }
        }
        if (!found) {
            vm_set_error(vm, "unresolved import: %s::%s",
                         mod->imports[i].module, mod->imports[i].name);
            return RUNE_ERR_NOIMPORT;
        }
    }

    /* Allocate linear memory */
    if (mod->has_memory) {
        uint32_t max_pages = mod->mem_max_pages > 0 ?
            mod->mem_max_pages : mod->mem_initial_pages;
        if ((size_t)max_pages * RUNE_PAGE_SIZE > vm->cfg.memory_limit) {
            vm_set_error(vm, "memory limit exceeded");
            return RUNE_ERR_OOM;
        }
        vm->memory = calloc(max_pages, RUNE_PAGE_SIZE);
        if (!vm->memory) return RUNE_ERR_OOM;
        vm->memory_pages = mod->mem_initial_pages;
        vm->memory_max   = max_pages;

        /* Apply data segments */
        for (uint32_t i = 0; i < mod->data_seg_count; i++) {
            uint32_t end = mod->data_segs[i].offset + mod->data_segs[i].size;
            if (end > vm->memory_pages * RUNE_PAGE_SIZE) {
                vm_set_error(vm, "data segment out of bounds");
                return RUNE_ERR_BOUNDS;
            }
            memcpy(vm->memory + mod->data_segs[i].offset,
                   mod->data_segs[i].data,
                   mod->data_segs[i].size);
        }
    }

    /* Copy globals */
    if (mod->global_count > 0) {
        vm->globals = malloc(mod->global_count * sizeof(rune_val_t));
        if (!vm->globals) return RUNE_ERR_OOM;
        for (uint32_t i = 0; i < mod->global_count; i++)
            vm->globals[i] = mod->globals[i].value;
    }

    vm->initialized = true;

    /* Run _init if present */
    if (mod->init_func >= 0) {
        rune_val_t ret = rune_void();
        rune_err_t err = vm_exec(vm, (uint32_t)mod->init_func, 0, NULL, &ret);
        if (err != RUNE_OK) return err;
    }

    return RUNE_OK;
}

/* ─────────────────────────────────────────────
   Memory Access
───────────────────────────────────────────── */

void  *rune_vm_memory(rune_vm_t *vm)      { return vm->memory; }
size_t rune_vm_memory_size(rune_vm_t *vm) { return (size_t)vm->memory_pages * RUNE_PAGE_SIZE; }

rune_err_t rune_vm_mem_read(rune_vm_t *vm, uint32_t offset, void *dst, size_t len) {
    if (!vm->memory || (size_t)offset + len > (size_t)vm->memory_pages * RUNE_PAGE_SIZE)
        return RUNE_ERR_BOUNDS;
    memcpy(dst, vm->memory + offset, len);
    return RUNE_OK;
}

rune_err_t rune_vm_mem_write(rune_vm_t *vm, uint32_t offset, const void *src, size_t len) {
    if (!vm->memory || (size_t)offset + len > (size_t)vm->memory_pages * RUNE_PAGE_SIZE)
        return RUNE_ERR_BOUNDS;
    memcpy(vm->memory + offset, src, len);
    return RUNE_OK;
}

rune_err_t rune_vm_mem_readstr(rune_vm_t *vm, uint32_t offset, char *dst, size_t max_len) {
    size_t mem_size = (size_t)vm->memory_pages * RUNE_PAGE_SIZE;
    if (!vm->memory || offset >= mem_size) return RUNE_ERR_BOUNDS;

    size_t avail = mem_size - offset;
    size_t limit = avail < max_len ? avail : max_len;

    for (size_t i = 0; i < limit; i++) {
        dst[i] = (char)vm->memory[offset + i];
        if (dst[i] == '\0') return RUNE_OK;
    }
    if (max_len > 0) dst[max_len - 1] = '\0';
    return RUNE_OK;
}

/* ─────────────────────────────────────────────
   Interpreter Core
───────────────────────────────────────────── */

#define MEM_CHECK(offset, sz) do { \
    if (!vm->memory || (uint64_t)(offset) + (sz) > (uint64_t)vm->memory_pages * RUNE_PAGE_SIZE) { \
        vm_set_error(vm, "memory access out of bounds at 0x%x", (unsigned)(offset)); \
        return RUNE_ERR_BOUNDS; \
    } } while(0)

#define FUEL_TICK() do { \
    if (vm->cfg.fuel_limit > 0) { \
        if (++vm->fuel_used > vm->cfg.fuel_limit) { \
            vm_set_error(vm, "fuel limit exceeded"); \
            return RUNE_ERR_FUEL; \
        } \
    } } while(0)

static inline uint32_t read_imm32(const uint32_t *code, uint32_t *pc) {
    return code[(*pc)++];
}
static inline uint64_t read_imm64(const uint32_t *code, uint32_t *pc) {
    uint64_t lo = code[(*pc)++];
    uint64_t hi = code[(*pc)++];
    return lo | (hi << 32);
}

/*
 * Look up the host function for an import index.
 * Returns NULL if not found.
 */
static rune_host_entry_t *vm_resolve_host(rune_vm_t *vm, uint32_t import_idx) {
    rune_module_t *mod = vm->mod;
    if (import_idx >= mod->import_count) return NULL;
    const char *mname = mod->imports[import_idx].module;
    const char *fname = mod->imports[import_idx].name;
    for (uint32_t i = 0; i < vm->host_fn_count; i++) {
        if (strcmp(vm->host_fns[i].module, mname) == 0 &&
            strcmp(vm->host_fns[i].name,   fname) == 0)
            return &vm->host_fns[i];
    }
    return NULL;
}

static rune_err_t vm_exec(rune_vm_t *vm, uint32_t func_idx,
                           int argc, rune_val_t *argv, rune_val_t *ret_val) {

    if (!vm->initialized && func_idx != (uint32_t)vm->mod->init_func)
        return RUNE_ERR_BADMODULE;

    /* Push frame */
    if (vm->frame_count >= vm->cfg.stack_size) {
        vm_set_error(vm, "call stack overflow");
        return RUNE_ERR_STACKOVERFLOW;
    }

    rune_func_t *fn = &vm->mod->funcs[func_idx];
    if (fn->is_import) {
        /* Direct call to import — dispatch to host */
        rune_host_entry_t *h = vm_resolve_host(vm, fn->import_idx);
        if (!h) { vm_set_error(vm, "unresolved import"); return RUNE_ERR_NOIMPORT; }
        rune_val_t r = rune_void();
        rune_err_t err = h->fn(vm, argc, argv, &r, h->userdata);
        if (ret_val) *ret_val = r;
        return err;
    }

    rune_frame_t *frame = &vm->frames[vm->frame_count++];
    frame->func_idx = func_idx;
    frame->pc       = 0;

    /* Allocate register window */
    if (!frame->regs) {
        frame->regs = calloc(RUNE_MAX_REGS, sizeof(rune_val_t));
        if (!frame->regs) { vm->frame_count--; return RUNE_ERR_OOM; }
    } else {
        memset(frame->regs, 0, RUNE_MAX_REGS * sizeof(rune_val_t));
    }

    /* Copy args into R0..R(argc-1) */
    int copy_n = argc < (int)fn->reg_count ? argc : (int)fn->reg_count;
    for (int i = 0; i < copy_n && i < (int)RUNE_MAX_REGS; i++)
        frame->regs[i] = argv[i];

    const uint32_t *code = (const uint32_t *)fn->code;
    uint32_t code_words   = fn->code_size / 4;

    rune_err_t result = RUNE_OK;

#define R(n)   frame->regs[(uint8_t)(n)]
#define PC     frame->pc
#define NEXT() do { FUEL_TICK(); instr = code[PC++]; \
                    op = RUNE_INSTR_OP(instr); \
                    dst = RUNE_INSTR_DST(instr); \
                    s1  = RUNE_INSTR_S1(instr); \
                    s2  = RUNE_INSTR_S2(instr); } while(0)

    uint32_t instr;
    uint8_t  op, dst, s1, s2;

    while (PC < code_words) {
        NEXT();

        switch ((rune_opcode_t)op) {

        /* ── Control ── */
        case OP_NOP: break;

        case OP_TRAP:
            vm_set_error(vm, "explicit trap in function %u at pc %u", func_idx, PC-1);
            result = RUNE_ERR_TRAP;
            goto done;

        case OP_RET:
            if (ret_val) *ret_val = R(0);   /* R0 is return */
            goto done;

        case OP_JMP: {
            int32_t offset = (int32_t)read_imm32(code, &PC);
            PC = (uint32_t)((int32_t)PC + offset);
            break;
        }
        case OP_JZ: {
            int32_t offset = (int32_t)read_imm32(code, &PC);
            rune_val_t cond = R(s1);
            bool zero = (cond.type == RUNE_TYPE_BOOL) ? !cond.as.b
                      : (cond.type == RUNE_TYPE_I32)  ? (cond.as.i32 == 0)
                      : (cond.type == RUNE_TYPE_I64)  ? (cond.as.i64 == 0)
                      : false;
            if (zero) PC = (uint32_t)((int32_t)PC + offset);
            break;
        }
        case OP_JNZ: {
            int32_t offset = (int32_t)read_imm32(code, &PC);
            rune_val_t cond = R(s1);
            bool zero = (cond.type == RUNE_TYPE_BOOL) ? !cond.as.b
                      : (cond.type == RUNE_TYPE_I32)  ? (cond.as.i32 == 0)
                      : (cond.type == RUNE_TYPE_I64)  ? (cond.as.i64 == 0)
                      : false;
            if (!zero) PC = (uint32_t)((int32_t)PC + offset);
            break;
        }
        case OP_JLT: {
            int32_t offset = (int32_t)read_imm32(code, &PC);
            if (R(s1).as.i32 < R(s2).as.i32)
                PC = (uint32_t)((int32_t)PC + offset);
            break;
        }
        case OP_JLE: {
            int32_t offset = (int32_t)read_imm32(code, &PC);
            if (R(s1).as.i32 <= R(s2).as.i32)
                PC = (uint32_t)((int32_t)PC + offset);
            break;
        }

        case OP_CALL: {
            uint32_t fi = read_imm32(code, &PC);
            /* Collect staged args */
            rune_val_t args[RUNE_MAX_PARAMS];
            int nargs = vm->arg_count;
            memcpy(args, vm->arg_buf, nargs * sizeof(rune_val_t));
            vm->arg_count = 0;

            rune_val_t ret = rune_void();
            result = vm_exec(vm, fi, nargs, args, &ret);
            if (result != RUNE_OK) goto done;
            R(dst) = ret;
            break;
        }

        case OP_CALL_HOST: {
            uint32_t import_idx = read_imm32(code, &PC);
            rune_host_entry_t *h = vm_resolve_host(vm, import_idx);
            if (!h) {
                vm_set_error(vm, "unresolved import %u", import_idx);
                result = RUNE_ERR_NOIMPORT;
                goto done;
            }
            rune_val_t args[RUNE_MAX_PARAMS];
            int nargs = vm->arg_count;
            memcpy(args, vm->arg_buf, nargs * sizeof(rune_val_t));
            vm->arg_count = 0;

            rune_val_t ret = rune_void();
            result = h->fn(vm, nargs, args, &ret, h->userdata);
            if (result != RUNE_OK) goto done;
            R(dst) = ret;
            break;
        }

        case OP_ARG:
            if (s1 < RUNE_MAX_PARAMS)
                vm->arg_buf[s1] = R(s2 ? s2 : dst); /* ARG slot reg */
            /* Actually: ARG <slot> <reg> uses dst=slot, s1=reg */
            vm->arg_buf[dst] = R(s1);
            if (dst >= vm->arg_count) vm->arg_count = dst + 1;
            break;

        /* ── Load Immediate ── */
        case OP_LDI32: {
            uint32_t imm = read_imm32(code, &PC);
            R(dst) = rune_i32((int32_t)imm);
            break;
        }
        case OP_LDI64: {
            uint64_t imm = read_imm64(code, &PC);
            R(dst) = rune_i64((int64_t)imm);
            break;
        }
        case OP_LDF32: {
            uint32_t bits = read_imm32(code, &PC);
            float f; memcpy(&f, &bits, 4);
            R(dst) = rune_f32(f);
            break;
        }
        case OP_LDF64: {
            uint64_t bits = read_imm64(code, &PC);
            double d; memcpy(&d, &bits, 8);
            R(dst) = rune_f64(d);
            break;
        }
        case OP_LDTRUE:  R(dst) = rune_bool(true);  break;
        case OP_LDFALSE: R(dst) = rune_bool(false); break;

        case OP_LDGLOBAL: {
            uint32_t gi = read_imm32(code, &PC);
            if (gi >= vm->mod->global_count) { result = RUNE_ERR_BOUNDS; goto done; }
            R(dst) = vm->globals[gi];
            break;
        }
        case OP_STGLOBAL: {
            uint32_t gi = read_imm32(code, &PC);
            if (gi >= vm->mod->global_count) { result = RUNE_ERR_BOUNDS; goto done; }
            vm->globals[gi] = R(s1);
            break;
        }

        case OP_MOV: R(dst) = R(s1); break;

        /* ── Integer i32 ── */
        case OP_ADD32:  R(dst) = rune_i32(R(s1).as.i32 + R(s2).as.i32); break;
        case OP_SUB32:  R(dst) = rune_i32(R(s1).as.i32 - R(s2).as.i32); break;
        case OP_MUL32:  R(dst) = rune_i32(R(s1).as.i32 * R(s2).as.i32); break;
        case OP_DIV32:
            if (R(s2).as.i32 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i32(R(s1).as.i32 / R(s2).as.i32); break;
        case OP_DIVU32:
            if (R(s2).as.i32 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i32((int32_t)((uint32_t)R(s1).as.i32 / (uint32_t)R(s2).as.i32)); break;
        case OP_REM32:
            if (R(s2).as.i32 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i32(R(s1).as.i32 % R(s2).as.i32); break;
        case OP_REMU32:
            if (R(s2).as.i32 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i32((int32_t)((uint32_t)R(s1).as.i32 % (uint32_t)R(s2).as.i32)); break;
        case OP_NEG32:    R(dst) = rune_i32(-R(s1).as.i32); break;
        case OP_AND32:    R(dst) = rune_i32(R(s1).as.i32 & R(s2).as.i32); break;
        case OP_OR32:     R(dst) = rune_i32(R(s1).as.i32 | R(s2).as.i32); break;
        case OP_XOR32:    R(dst) = rune_i32(R(s1).as.i32 ^ R(s2).as.i32); break;
        case OP_SHL32:    R(dst) = rune_i32(R(s1).as.i32 << (R(s2).as.i32 & 31)); break;
        case OP_SHR32:    R(dst) = rune_i32(R(s1).as.i32 >> (R(s2).as.i32 & 31)); break;
        case OP_SHRU32:   R(dst) = rune_i32((int32_t)((uint32_t)R(s1).as.i32 >> (R(s2).as.i32 & 31))); break;
        case OP_NOT32:    R(dst) = rune_i32(~R(s1).as.i32); break;
        case OP_CLZ32:    R(dst) = rune_i32(R(s1).as.i32 ? __builtin_clz((uint32_t)R(s1).as.i32) : 32); break;
        case OP_CTZ32:    R(dst) = rune_i32(R(s1).as.i32 ? __builtin_ctz((uint32_t)R(s1).as.i32) : 32); break;
        case OP_POPCNT32: R(dst) = rune_i32(__builtin_popcount((uint32_t)R(s1).as.i32)); break;

        /* ── Integer i64 ── */
        case OP_ADD64: R(dst) = rune_i64(R(s1).as.i64 + R(s2).as.i64); break;
        case OP_SUB64: R(dst) = rune_i64(R(s1).as.i64 - R(s2).as.i64); break;
        case OP_MUL64: R(dst) = rune_i64(R(s1).as.i64 * R(s2).as.i64); break;
        case OP_DIV64:
            if (R(s2).as.i64 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i64(R(s1).as.i64 / R(s2).as.i64); break;
        case OP_DIVU64:
            if (R(s2).as.i64 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i64((int64_t)((uint64_t)R(s1).as.i64 / (uint64_t)R(s2).as.i64)); break;
        case OP_REM64:
            if (R(s2).as.i64 == 0) { result = RUNE_ERR_DIVZERO; goto done; }
            R(dst) = rune_i64(R(s1).as.i64 % R(s2).as.i64); break;
        case OP_AND64: R(dst) = rune_i64(R(s1).as.i64 & R(s2).as.i64); break;
        case OP_OR64:  R(dst) = rune_i64(R(s1).as.i64 | R(s2).as.i64); break;
        case OP_XOR64: R(dst) = rune_i64(R(s1).as.i64 ^ R(s2).as.i64); break;
        case OP_SHL64: R(dst) = rune_i64(R(s1).as.i64 << (R(s2).as.i64 & 63)); break;
        case OP_SHR64: R(dst) = rune_i64(R(s1).as.i64 >> (R(s2).as.i64 & 63)); break;
        case OP_NOT64: R(dst) = rune_i64(~R(s1).as.i64); break;
        case OP_NEG64: R(dst) = rune_i64(-R(s1).as.i64); break;

        /* ── Float f32 ── */
        case OP_FADD32:   R(dst) = rune_f32(R(s1).as.f32 + R(s2).as.f32); break;
        case OP_FSUB32:   R(dst) = rune_f32(R(s1).as.f32 - R(s2).as.f32); break;
        case OP_FMUL32:   R(dst) = rune_f32(R(s1).as.f32 * R(s2).as.f32); break;
        case OP_FDIV32:   R(dst) = rune_f32(R(s1).as.f32 / R(s2).as.f32); break;
        case OP_FABS32:   R(dst) = rune_f32(fabsf(R(s1).as.f32)); break;
        case OP_FNEG32:   R(dst) = rune_f32(-R(s1).as.f32); break;
        case OP_FSQRT32:  R(dst) = rune_f32(sqrtf(R(s1).as.f32)); break;
        case OP_FMIN32:   R(dst) = rune_f32(fminf(R(s1).as.f32, R(s2).as.f32)); break;
        case OP_FMAX32:   R(dst) = rune_f32(fmaxf(R(s1).as.f32, R(s2).as.f32)); break;
        case OP_FFLOOR32: R(dst) = rune_f32(floorf(R(s1).as.f32)); break;
        case OP_FCEIL32:  R(dst) = rune_f32(ceilf(R(s1).as.f32)); break;
        case OP_FROUND32: R(dst) = rune_f32(roundf(R(s1).as.f32)); break;

        /* ── Float f64 ── */
        case OP_FADD64:   R(dst) = rune_f64(R(s1).as.f64 + R(s2).as.f64); break;
        case OP_FSUB64:   R(dst) = rune_f64(R(s1).as.f64 - R(s2).as.f64); break;
        case OP_FMUL64:   R(dst) = rune_f64(R(s1).as.f64 * R(s2).as.f64); break;
        case OP_FDIV64:   R(dst) = rune_f64(R(s1).as.f64 / R(s2).as.f64); break;
        case OP_FABS64:   R(dst) = rune_f64(fabs(R(s1).as.f64)); break;
        case OP_FNEG64:   R(dst) = rune_f64(-R(s1).as.f64); break;
        case OP_FSQRT64:  R(dst) = rune_f64(sqrt(R(s1).as.f64)); break;
        case OP_FMIN64:   R(dst) = rune_f64(fmin(R(s1).as.f64, R(s2).as.f64)); break;
        case OP_FMAX64:   R(dst) = rune_f64(fmax(R(s1).as.f64, R(s2).as.f64)); break;
        case OP_FFLOOR64: R(dst) = rune_f64(floor(R(s1).as.f64)); break;
        case OP_FCEIL64:  R(dst) = rune_f64(ceil(R(s1).as.f64)); break;
        case OP_FROUND64: R(dst) = rune_f64(round(R(s1).as.f64)); break;

        /* ── Comparisons ── */
        case OP_EQ32:  R(dst) = rune_bool(R(s1).as.i32 == R(s2).as.i32); break;
        case OP_NE32:  R(dst) = rune_bool(R(s1).as.i32 != R(s2).as.i32); break;
        case OP_LT32:  R(dst) = rune_bool(R(s1).as.i32 <  R(s2).as.i32); break;
        case OP_LE32:  R(dst) = rune_bool(R(s1).as.i32 <= R(s2).as.i32); break;
        case OP_GT32:  R(dst) = rune_bool(R(s1).as.i32 >  R(s2).as.i32); break;
        case OP_GE32:  R(dst) = rune_bool(R(s1).as.i32 >= R(s2).as.i32); break;
        case OP_LTU32: R(dst) = rune_bool((uint32_t)R(s1).as.i32 < (uint32_t)R(s2).as.i32); break;
        case OP_LEU32: R(dst) = rune_bool((uint32_t)R(s1).as.i32 <= (uint32_t)R(s2).as.i32); break;
        case OP_EQ64:  R(dst) = rune_bool(R(s1).as.i64 == R(s2).as.i64); break;
        case OP_NE64:  R(dst) = rune_bool(R(s1).as.i64 != R(s2).as.i64); break;
        case OP_LT64:  R(dst) = rune_bool(R(s1).as.i64 <  R(s2).as.i64); break;
        case OP_LE64:  R(dst) = rune_bool(R(s1).as.i64 <= R(s2).as.i64); break;
        case OP_FEQ32: R(dst) = rune_bool(R(s1).as.f32 == R(s2).as.f32); break;
        case OP_FLT32: R(dst) = rune_bool(R(s1).as.f32 <  R(s2).as.f32); break;
        case OP_FEQ64: R(dst) = rune_bool(R(s1).as.f64 == R(s2).as.f64); break;
        case OP_FLT64: R(dst) = rune_bool(R(s1).as.f64 <  R(s2).as.f64); break;

        /* ── Conversions ── */
        case OP_I32_TO_I64:  R(dst) = rune_i64((int64_t)R(s1).as.i32); break;
        case OP_I64_TO_I32:  R(dst) = rune_i32((int32_t)R(s1).as.i64); break;
        case OP_U32_TO_I64:  R(dst) = rune_i64((int64_t)(uint32_t)R(s1).as.i32); break;
        case OP_I32_TO_F32:  R(dst) = rune_f32((float)R(s1).as.i32); break;
        case OP_I32_TO_F64:  R(dst) = rune_f64((double)R(s1).as.i32); break;
        case OP_F32_TO_I32:  R(dst) = rune_i32((int32_t)R(s1).as.f32); break;
        case OP_F64_TO_I32:  R(dst) = rune_i32((int32_t)R(s1).as.f64); break;
        case OP_F32_TO_F64:  R(dst) = rune_f64((double)R(s1).as.f32); break;
        case OP_F64_TO_F32:  R(dst) = rune_f32((float)R(s1).as.f64); break;
        case OP_I64_TO_F64:  R(dst) = rune_f64((double)R(s1).as.i64); break;
        case OP_F64_TO_I64:  R(dst) = rune_i64((int64_t)R(s1).as.f64); break;
        case OP_BOOL_TO_I32: R(dst) = rune_i32(R(s1).as.b ? 1 : 0); break;

        /* ── Memory Loads ── */
        case OP_LOAD8: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 1);
            R(dst) = rune_i32((uint8_t)vm->memory[off]);
            break;
        }
        case OP_LOAD8S: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 1);
            R(dst) = rune_i32((int8_t)vm->memory[off]);
            break;
        }
        case OP_LOAD16: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 2);
            uint16_t v; memcpy(&v, vm->memory + off, 2);
            R(dst) = rune_i32(v);
            break;
        }
        case OP_LOAD16S: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 2);
            int16_t v; memcpy(&v, vm->memory + off, 2);
            R(dst) = rune_i32(v);
            break;
        }
        case OP_LOAD32: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 4);
            uint32_t v; memcpy(&v, vm->memory + off, 4);
            R(dst) = rune_i32((int32_t)v);
            break;
        }
        case OP_LOAD64: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 8);
            uint64_t v; memcpy(&v, vm->memory + off, 8);
            R(dst) = rune_i64((int64_t)v);
            break;
        }
        case OP_LOADF32: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 4);
            float v; memcpy(&v, vm->memory + off, 4);
            R(dst) = rune_f32(v);
            break;
        }
        case OP_LOADF64: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 8);
            double v; memcpy(&v, vm->memory + off, 8);
            R(dst) = rune_f64(v);
            break;
        }

        /* ── Memory Stores ── */
        case OP_STORE8: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 1);
            vm->memory[off] = (uint8_t)R(dst).as.i32;
            break;
        }
        case OP_STORE16: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 2);
            uint16_t v = (uint16_t)R(dst).as.i32;
            memcpy(vm->memory + off, &v, 2);
            break;
        }
        case OP_STORE32: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 4);
            uint32_t v = (uint32_t)R(dst).as.i32;
            memcpy(vm->memory + off, &v, 4);
            break;
        }
        case OP_STORE64: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 8);
            uint64_t v = (uint64_t)R(dst).as.i64;
            memcpy(vm->memory + off, &v, 8);
            break;
        }
        case OP_STOREF32: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 4);
            float v = R(dst).as.f32;
            memcpy(vm->memory + off, &v, 4);
            break;
        }
        case OP_STOREF64: {
            uint32_t off = R(s1).as.i32 + (uint32_t)read_imm32(code, &PC);
            MEM_CHECK(off, 8);
            double v = R(dst).as.f64;
            memcpy(vm->memory + off, &v, 8);
            break;
        }

        /* ── Memory ops ── */
        case OP_MEM_SIZE:
            R(dst) = rune_i32((int32_t)vm->memory_pages);
            break;
        case OP_MEM_GROW: {
            uint32_t req = (uint32_t)R(s1).as.i32;
            uint32_t new_pages = vm->memory_pages + req;
            if (new_pages > vm->memory_max) {
                R(dst) = rune_i32(-1);
            } else {
                /* Memory is pre-allocated to max; just adjust page count */
                memset(vm->memory + vm->memory_pages * RUNE_PAGE_SIZE, 0,
                       req * RUNE_PAGE_SIZE);
                R(dst) = rune_i32((int32_t)vm->memory_pages);
                vm->memory_pages = new_pages;
            }
            break;
        }
        case OP_MEM_COPY: {
            uint32_t pdst  = (uint32_t)R(dst).as.i32;
            uint32_t psrc  = (uint32_t)R(s1).as.i32;
            uint32_t sz    = (uint32_t)R(s2).as.i32;
            MEM_CHECK(pdst, sz);
            MEM_CHECK(psrc, sz);
            memmove(vm->memory + pdst, vm->memory + psrc, sz);
            break;
        }
        case OP_MEM_FILL: {
            uint32_t pdst  = (uint32_t)R(dst).as.i32;
            int      val   = R(s1).as.i32;
            uint32_t sz    = (uint32_t)R(s2).as.i32;
            MEM_CHECK(pdst, sz);
            memset(vm->memory + pdst, val, sz);
            break;
        }

        default:
            vm_set_error(vm, "unknown opcode 0x%02x at func %u pc %u", op, func_idx, PC-1);
            result = RUNE_ERR_BADOPCODE;
            goto done;
        }
    }

    /* Fell off the end — implicit return with R0 */
    if (ret_val) *ret_val = frame->regs[0];

done:
    vm->frame_count--;
    return result;

#undef R
#undef PC
#undef NEXT
}

/* ─────────────────────────────────────────────
   Public Call Interface
───────────────────────────────────────────── */

rune_err_t rune_vm_call(rune_vm_t *vm, const char *name,
                         int argc, rune_val_t *argv, rune_val_t *ret) {
    if (!vm->initialized) {
        vm_set_error(vm, "VM not initialized, call rune_vm_init first");
        return RUNE_ERR_BADMODULE;
    }

    /* Find export */
    for (uint32_t i = 0; i < vm->mod->export_count; i++) {
        rune_export_t *e = &vm->mod->exports[i];
        if (e->kind == RUNE_EXPORT_FUNC && strcmp(e->name, name) == 0) {
            rune_val_t r = rune_void();
            rune_err_t err = vm_exec(vm, e->idx, argc, argv, &r);
            if (ret) *ret = r;
            return err;
        }
    }

    vm_set_error(vm, "export not found: %s", name);
    return RUNE_ERR_NOEXPORT;
}

void rune_vm_refuel(rune_vm_t *vm, uint64_t fuel) {
    vm->fuel_used   = 0;
    vm->cfg.fuel_limit = fuel;
}

const char *rune_vm_last_error(const rune_vm_t *vm) { return vm->error_buf; }
uint64_t    rune_vm_fuel_used(const rune_vm_t *vm)  { return vm->fuel_used; }
