/* The C boundary of the Bastio base64 library. This file is hand-written C
 * and is trusted, not proved: it is the only code between a C caller and the
 * proved functions, so it is kept to conversions and the one precondition
 * the Bastio side states (b64_encode's `requires`).
 *
 * The generated C (from lib.bastio) is included as one translation unit; its
 * functions are `static`, so only the three below are visible to the linker.
 * stage0 has no `export` yet (see README.md). */
#define main bastio_base64_unused_main
#include "base64.gen.c"
#undef main

#include "base64.h"

/* Tags of `enum Decoded` in base64.bastio, in declaration order. */
enum { DECODED_OK = 0, DECODED_ERR = 1 };

int base64_decode(const uint8_t *src, size_t len, uint8_t **out, size_t *out_len, size_t *err_at) {
    /* `src` is lent as `&Bytes`, which the type checker allows to be read
     * only (no bset, no bfree, no move), so dropping `const` is sound. */
    bst_bytes in = { len, (unsigned char *)src };
    bst_e_Decoded r = f_b64_decode(in);
    if (r.tag == DECODED_OK) {
        *out = r.u.Ok.data;
        *out_len = r.u.Ok.len;
        return 0;
    }
    *err_at = r.u.Err;
    return -1;
}

int base64_encode(const uint8_t *src, size_t len, uint8_t **out, size_t *out_len) {
    /* b64_encode `requires blen(src) <= B64_MAX_RAW`: at the C boundary the
     * caller cannot be proved to meet it, so it is checked here. */
    if ((uint64_t)len > 0x0800000000000000ull) return -1;
    bst_bytes in = { len, (unsigned char *)src };
    bst_bytes r = f_b64_encode(in);
    *out = r.data;
    *out_len = r.len;
    return 0;
}

/* The hosted prelude allocates Bytes with calloc. */
void base64_free(uint8_t *p) { bst_bfree((bst_bytes){ 0, p }); }
