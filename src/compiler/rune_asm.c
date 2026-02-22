/*
 * rune_asm.c — Bytecode assembler implementation
 */

#include "../include/rune_asm.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <assert.h>

/* ─────────────────────────────────────────────
   Dynamic Buffer
───────────────────────────────────────────── */
typedef struct {
    uint8_t *data;
    size_t   len;
    size_t   cap;
} buf_t;

static void buf_reserve(buf_t *b, size_t extra) {
    size_t need = b->len + extra;
    if (need > b->cap) {
        b->cap = need * 2 + 64;
        b->data = realloc(b->data, b->cap);
        assert(b->data);
    }
}

static void buf_push_u8(buf_t *b, uint8_t v) {
    buf_reserve(b, 1);
    b->data[b->len++] = v;
}

static void buf_push_u16(buf_t *b, uint16_t v) {
    buf_push_u8(b, (uint8_t)(v));
    buf_push_u8(b, (uint8_t)(v >> 8));
}

static void buf_push_u32(buf_t *b, uint32_t v) {
    buf_push_u8(b, (uint8_t)(v));
    buf_push_u8(b, (uint8_t)(v >> 8));
    buf_push_u8(b, (uint8_t)(v >> 16));
    buf_push_u8(b, (uint8_t)(v >> 24));
}

static void buf_push_u64(buf_t *b, uint64_t v) {
    buf_push_u32(b, (uint32_t)v);
    buf_push_u32(b, (uint32_t)(v >> 32));
}

static void buf_push_bytes(buf_t *b, const void *src, size_t n) {
    buf_reserve(b, n);
    memcpy(b->data + b->len, src, n);
    b->len += n;
}

static void buf_push_str8(buf_t *b, const char *s) {
    uint8_t len = (uint8_t)strlen(s);
    buf_push_u8(b, len);
    buf_push_bytes(b, s, len);
}

/* Patch a uint32 at position pos */
static void buf_patch_u32(buf_t *b, size_t pos, uint32_t v) {
    assert(pos + 4 <= b->len);
    b->data[pos+0] = (uint8_t)(v);
    b->data[pos+1] = (uint8_t)(v >> 8);
    b->data[pos+2] = (uint8_t)(v >> 16);
    b->data[pos+3] = (uint8_t)(v >> 24);
}

/* ─────────────────────────────────────────────
   Assembler State
───────────────────────────────────────────── */

#define MAX_TYPES   512
#define MAX_IMPORTS 512
#define MAX_FUNCS   4096
#define MAX_EXPORTS 512
#define MAX_GLOBALS 512
#define MAX_DATA    512

typedef struct {
    uint8_t  param_count;
    uint8_t  return_count;
    rune_type_t params[RUNE_MAX_PARAMS];
    rune_type_t rets[1];
} asm_type_t;

typedef struct {
    char    module[64];
    char    name[64];
    uint16_t type_idx;
} asm_import_t;

typedef struct {
    uint16_t  type_idx;
    uint8_t   reg_count;
    uint8_t   local_count;
    bool      is_import;
    uint32_t  import_idx;
    buf_t     code;
} asm_func_t;

typedef struct {
    rune_export_kind_t kind;
    uint32_t           idx;
    char               name[128];
} asm_export_t;

typedef struct {
    rune_type_t type;
    bool        mutable;
    rune_val_t  init;
} asm_global_t;

typedef struct {
    uint32_t      offset;
    uint8_t      *data;
    uint32_t      size;
} asm_data_t;

struct rune_asm {
    asm_type_t   types[MAX_TYPES];
    uint32_t     type_count;

    asm_import_t imports[MAX_IMPORTS];
    uint32_t     import_count;

    asm_func_t   funcs[MAX_FUNCS];
    uint32_t     func_count;

    asm_export_t exports[MAX_EXPORTS];
    uint32_t     export_count;

    asm_global_t globals[MAX_GLOBALS];
    uint32_t     global_count;

    asm_data_t   data_segs[MAX_DATA];
    uint32_t     data_seg_count;

    bool         has_memory;
    uint16_t     mem_initial;
    uint16_t     mem_max;

    /* Currently open function */
    int32_t      current_func;
};

/* ─────────────────────────────────────────────
   API Implementation
───────────────────────────────────────────── */

rune_asm_t *rune_asm_new(void) {
    rune_asm_t *a = calloc(1, sizeof(rune_asm_t));
    a->current_func = -1;
    return a;
}

void rune_asm_free(rune_asm_t *a) {
    if (!a) return;
    for (uint32_t i = 0; i < a->func_count; i++)
        free(a->funcs[i].code.data);
    for (uint32_t i = 0; i < a->data_seg_count; i++)
        free(a->data_segs[i].data);
    free(a);
}

uint16_t rune_asm_type(rune_asm_t *a,
                        uint8_t pc, const rune_type_t *params,
                        uint8_t rc, const rune_type_t *rets) {
    assert(a->type_count < MAX_TYPES);
    asm_type_t *t = &a->types[a->type_count];
    t->param_count  = pc;
    t->return_count = rc;
    memcpy(t->params, params ? params : (rune_type_t[]){}, pc * sizeof(rune_type_t));
    if (rc && rets) memcpy(t->rets, rets, rc * sizeof(rune_type_t));
    return (uint16_t)a->type_count++;
}

uint32_t rune_asm_import(rune_asm_t *a, const char *mod, const char *name, uint16_t type_idx) {
    assert(a->import_count < MAX_IMPORTS);
    assert(a->func_count < MAX_FUNCS);
    uint32_t imp_idx  = a->import_count;
    uint32_t func_idx = a->func_count;

    asm_import_t *im = &a->imports[imp_idx++];
    strncpy(im->module, mod,  sizeof(im->module) - 1);
    strncpy(im->name,   name, sizeof(im->name) - 1);
    im->type_idx = type_idx;
    a->import_count = imp_idx;

    asm_func_t *f = &a->funcs[a->func_count++];
    f->type_idx   = type_idx;
    f->is_import  = true;
    f->import_idx = imp_idx - 1;
    return func_idx;
}

void rune_asm_memory(rune_asm_t *a, uint16_t initial, uint16_t max) {
    a->has_memory  = true;
    a->mem_initial = initial;
    a->mem_max     = max ? max : initial;
}

uint32_t rune_asm_func(rune_asm_t *a, uint16_t type_idx, uint8_t reg_count, uint8_t local_count) {
    assert(a->func_count < MAX_FUNCS);
    uint32_t idx = a->func_count++;
    asm_func_t *f = &a->funcs[idx];
    f->type_idx    = type_idx;
    f->reg_count   = reg_count;
    f->local_count = local_count;
    f->is_import   = false;
    return idx;
}

void rune_asm_export_func(rune_asm_t *a, uint32_t func_idx, const char *name) {
    assert(a->export_count < MAX_EXPORTS);
    asm_export_t *e = &a->exports[a->export_count++];
    e->kind = RUNE_EXPORT_FUNC;
    e->idx  = func_idx;
    strncpy(e->name, name, sizeof(e->name) - 1);
}

void rune_asm_export_memory(rune_asm_t *a, const char *name) {
    assert(a->export_count < MAX_EXPORTS);
    asm_export_t *e = &a->exports[a->export_count++];
    e->kind = RUNE_EXPORT_MEMORY;
    e->idx  = 0;
    strncpy(e->name, name, sizeof(e->name) - 1);
}

uint32_t rune_asm_global(rune_asm_t *a, rune_type_t type, bool mutable, rune_val_t init) {
    assert(a->global_count < MAX_GLOBALS);
    uint32_t idx = a->global_count++;
    a->globals[idx] = (asm_global_t){ type, mutable, init };
    return idx;
}

void rune_asm_data(rune_asm_t *a, uint32_t offset, const void *data, uint32_t size) {
    assert(a->data_seg_count < MAX_DATA);
    asm_data_t *d = &a->data_segs[a->data_seg_count++];
    d->offset = offset;
    d->size   = size;
    d->data   = malloc(size);
    assert(d->data);
    memcpy(d->data, data, size);
}

void rune_asm_begin_code(rune_asm_t *a, uint32_t func_idx) {
    assert(a->current_func == -1);
    assert(func_idx < a->func_count);
    assert(!a->funcs[func_idx].is_import);
    a->current_func = (int32_t)func_idx;
    a->funcs[func_idx].code.len = 0;
}

void rune_asm_end_code(rune_asm_t *a) {
    assert(a->current_func >= 0);
    a->current_func = -1;
}

void rune_asm_emit(rune_asm_t *a, rune_opcode_t op, uint8_t dst, uint8_t s1, uint8_t s2) {
    assert(a->current_func >= 0);
    buf_t *b = &a->funcs[a->current_func].code;
    uint32_t instr = RUNE_INSTR(op, dst, s1, s2);
    buf_push_u32(b, instr);
}

void rune_asm_emit_i(rune_asm_t *a, rune_opcode_t op, uint8_t dst, uint8_t s1, uint8_t s2, uint32_t imm) {
    rune_asm_emit(a, op, dst, s1, s2);
    buf_push_u32(&a->funcs[a->current_func].code, imm);
}

void rune_asm_emit_i64(rune_asm_t *a, rune_opcode_t op, uint8_t dst, uint64_t imm) {
    rune_asm_emit(a, op, dst, 0, 0);
    buf_t *b = &a->funcs[a->current_func].code;
    buf_push_u32(b, (uint32_t)imm);
    buf_push_u32(b, (uint32_t)(imm >> 32));
}

uint32_t rune_asm_label(rune_asm_t *a) {
    assert(a->current_func >= 0);
    return (uint32_t)(a->funcs[a->current_func].code.len / 4);
}

void rune_asm_patch_jump(rune_asm_t *a, uint32_t patch_word, uint32_t target_word) {
    assert(a->current_func >= 0);
    buf_t *b = &a->funcs[a->current_func].code;
    /* The immediate follows the instruction word, so it's at patch_word+1 */
    size_t imm_pos = (size_t)(patch_word + 1) * 4;
    /* Relative offset: from the word AFTER the immediate to target */
    int32_t rel = (int32_t)target_word - (int32_t)(patch_word + 2);
    buf_patch_u32(b, imm_pos, (uint32_t)rel);
}

/* ─────────────────────────────────────────────
   CRC-32
───────────────────────────────────────────── */
static uint32_t crc32_buf(const uint8_t *data, size_t len) {
    uint32_t crc = ~0u;
    for (size_t i = 0; i < len; i++) {
        crc ^= data[i];
        for (int j = 0; j < 8; j++)
            crc = (crc >> 1) ^ (0xEDB88320u & -(crc & 1));
    }
    return ~crc;
}

/* ─────────────────────────────────────────────
   Finalize — Emit Binary
───────────────────────────────────────────── */

static void emit_section(buf_t *out, rune_sect_id_t id, const buf_t *body) {
    buf_push_u8(out, (uint8_t)id);
    buf_push_u32(out, (uint32_t)body->len);
    buf_push_bytes(out, body->data, body->len);
}

uint8_t *rune_asm_finalize(rune_asm_t *a, size_t *out_size) {
    buf_t out = {0};
    buf_t sec = {0};

    /* Reserve header space */
    buf_reserve(&out, sizeof(rune_file_header_t));
    out.len = sizeof(rune_file_header_t);

    /* ── TYPE section ── */
    sec.len = 0;
    buf_push_u32(&sec, a->type_count);
    for (uint32_t i = 0; i < a->type_count; i++) {
        asm_type_t *t = &a->types[i];
        buf_push_u8(&sec, t->param_count);
        buf_push_u8(&sec, t->return_count);
        for (int j = 0; j < t->param_count; j++)  buf_push_u8(&sec, (uint8_t)t->params[j]);
        for (int j = 0; j < t->return_count; j++) buf_push_u8(&sec, (uint8_t)t->rets[j]);
    }
    if (a->type_count) emit_section(&out, RUNE_SECT_TYPE, &sec);

    /* ── IMPORT section ── */
    sec.len = 0;
    buf_push_u32(&sec, a->import_count);
    for (uint32_t i = 0; i < a->import_count; i++) {
        buf_push_str8(&sec, a->imports[i].module);
        buf_push_str8(&sec, a->imports[i].name);
        buf_push_u16(&sec, a->imports[i].type_idx);
    }
    if (a->import_count) emit_section(&out, RUNE_SECT_IMPORT, &sec);

    /* ── FUNC section ── */
    uint32_t body_funcs = 0;
    for (uint32_t i = 0; i < a->func_count; i++)
        if (!a->funcs[i].is_import) body_funcs++;

    sec.len = 0;
    buf_push_u32(&sec, body_funcs);
    for (uint32_t i = 0; i < a->func_count; i++) {
        if (a->funcs[i].is_import) continue;
        buf_push_u16(&sec, a->funcs[i].type_idx);
        buf_push_u8(&sec,  a->funcs[i].reg_count);
        buf_push_u8(&sec,  a->funcs[i].local_count);
    }
    if (body_funcs) emit_section(&out, RUNE_SECT_FUNC, &sec);

    /* ── MEMORY section ── */
    if (a->has_memory) {
        sec.len = 0;
        buf_push_u16(&sec, a->mem_initial);
        buf_push_u16(&sec, a->mem_max);
        emit_section(&out, RUNE_SECT_MEMORY, &sec);
    }

    /* ── GLOBAL section ── */
    if (a->global_count) {
        sec.len = 0;
        buf_push_u32(&sec, a->global_count);
        for (uint32_t i = 0; i < a->global_count; i++) {
            asm_global_t *g = &a->globals[i];
            buf_push_u8(&sec, (uint8_t)g->type);
            buf_push_u8(&sec, g->mutable ? 1 : 0);
            uint64_t raw = 0;
            switch (g->type) {
                case RUNE_TYPE_I32: raw = (uint64_t)(uint32_t)g->init.as.i32; break;
                case RUNE_TYPE_I64: raw = (uint64_t)g->init.as.i64; break;
                case RUNE_TYPE_F32: memcpy(&raw, &g->init.as.f32, 4); break;
                case RUNE_TYPE_F64: memcpy(&raw, &g->init.as.f64, 8); break;
                default: break;
            }
            buf_push_u64(&sec, raw);
        }
        emit_section(&out, RUNE_SECT_GLOBAL, &sec);
    }

    /* ── EXPORT section ── */
    if (a->export_count) {
        sec.len = 0;
        buf_push_u32(&sec, a->export_count);
        for (uint32_t i = 0; i < a->export_count; i++) {
            asm_export_t *e = &a->exports[i];
            buf_push_u8(&sec, (uint8_t)e->kind);
            buf_push_u32(&sec, e->idx);
            buf_push_str8(&sec, e->name);
        }
        emit_section(&out, RUNE_SECT_EXPORT, &sec);
    }

    /* ── CODE section ── */
    if (body_funcs) {
        sec.len = 0;
        buf_push_u32(&sec, body_funcs);
        for (uint32_t i = 0; i < a->func_count; i++) {
            asm_func_t *f = &a->funcs[i];
            if (f->is_import) continue;
            buf_push_u32(&sec, (uint32_t)f->code.len);
            buf_push_bytes(&sec, f->code.data, f->code.len);
        }
        emit_section(&out, RUNE_SECT_CODE, &sec);
    }

    /* ── DATA section ── */
    if (a->data_seg_count) {
        sec.len = 0;
        buf_push_u32(&sec, a->data_seg_count);
        for (uint32_t i = 0; i < a->data_seg_count; i++) {
            asm_data_t *d = &a->data_segs[i];
            buf_push_u8(&sec, 0); /* memory index */
            buf_push_u32(&sec, d->offset);
            buf_push_u32(&sec, d->size);
            buf_push_bytes(&sec, d->data, d->size);
        }
        emit_section(&out, RUNE_SECT_DATA, &sec);
    }

    free(sec.data);

    /* Patch header */
    rune_file_header_t *hdr = (rune_file_header_t *)out.data;
    memcpy(hdr->magic, RUNE_MAGIC, RUNE_MAGIC_LEN);
    hdr->version  = RUNE_BC_VERSION;
    hdr->flags    = 0;
    hdr->reserved = 0;
    hdr->crc32    = crc32_buf(
        out.data + sizeof(rune_file_header_t),
        out.len  - sizeof(rune_file_header_t));

    *out_size = out.len;
    return out.data;
}
