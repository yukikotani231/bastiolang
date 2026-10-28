/* Tests the Bastio LEB128 library through its C API (leb128.h, generated
 * from the `export fn`s in lib.bastio):
 *   1. known vectors (the examples of the DWARF specification and others);
 *   2. the verdicts for truncated, over-long and overflowing input;
 *   3. a differential fuzz against an independent decoder written here in C:
 *      random bytes at random offsets, and encodings with bytes changed;
 *   4. round trips, and the encoder against a buffer of the exact size, a
 *      larger one (the bytes after the encoding must stay as they were) and
 *      one that is too small.
 * Built under AddressSanitizer + UBSan by run.sh, so a stray read or write in
 * the generated code would stop the test. */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "leb128.h"

static int failures;
#define CHECK(c, ...) do { if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

enum { OK, TRUNCATED, TOO_LONG, OVERFLOW_ };

/* The reference: the textbook loop, with the same verdicts. Returns the kind
 * and, for OK, the value and the bytes used. */
static int ref_decode(const uint8_t *s, size_t n, size_t at, uint64_t *value, size_t *used) {
    uint64_t v = 0;
    if (at > n) return TRUNCATED;
    for (size_t i = 0; i < 10; i++) {
        if (at + i >= n) return TRUNCATED;
        uint8_t b = s[at + i];
        if (i == 9) {
            if (b & 0x80) return TOO_LONG;
            if (b > 1) return OVERFLOW_;
        }
        v |= (uint64_t)(b & 0x7f) << (7 * i);
        if (!(b & 0x80)) { *value = v; *used = i + 1; return OK; }
    }
    return TRUNCATED;   /* not reached: the tenth byte decides above */
}

static int dec(const uint8_t *s, size_t n, size_t at, uint64_t *value, size_t *used) {
    bst_e_Leb128Decoded r = leb128_decode(s, n, at);
    switch (r.tag) {
    case bst_e_Leb128Decoded_Ok: *value = r.u.Ok.value; *used = r.u.Ok.used; return OK;
    case bst_e_Leb128Decoded_Truncated: return TRUNCATED;
    case bst_e_Leb128Decoded_TooLong: return TOO_LONG;
    default: return OVERFLOW_;
    }
}

/* xorshift64*, seeded, so a failure can be replayed. */
static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint64_t rnd(void) { rng ^= rng >> 12; rng ^= rng << 25; rng ^= rng >> 27; return rng * 0x2545f4914f6cdd1dull; }

/* The shortest encoding, written as a C programmer would. */
static size_t ref_encode(uint64_t x, uint8_t *out) {
    size_t i = 0;
    do {
        uint8_t b = x & 0x7f;
        x >>= 7;
        out[i++] = x ? b | 0x80 : b;
    } while (x);
    return i;
}

static void agree(const uint8_t *s, size_t n, size_t at) {
    uint64_t v1 = 0, v2 = 0;
    size_t u1 = 0, u2 = 0;
    int k1 = ref_decode(s, n, at, &v1, &u1), k2 = dec(s, n, at, &v2, &u2);
    CHECK(k1 == k2, "decode at %zu of %zu bytes: verdict %d, reference %d", at, n, k2, k1);
    if (k1 == OK && k2 == OK) CHECK(v1 == v2 && u1 == u2, "decode at %zu: %llu in %zu bytes, reference %llu in %zu",
                                    at, (unsigned long long)v2, u2, (unsigned long long)v1, u1);
}

static void vectors(void) {
    static const struct { uint64_t v; const char *hex; } t[] = {
        {0, "00"}, {1, "01"}, {2, "02"}, {127, "7f"}, {128, "8001"}, {129, "8101"}, {130, "8201"},
        {12857, "b964"}, {624485, "e58e26"}, {16384, "808001"},
        {0xffffffffull, "ffffffff0f"},
        {0xffffffffffffffffull, "ffffffffffffffffff01"},
        {0x8000000000000000ull, "80808080808080808001"},
    };
    for (size_t i = 0; i < sizeof t / sizeof t[0]; i++) {
        uint8_t want[10], got[16];
        size_t wl = strlen(t[i].hex) / 2;
        for (size_t k = 0; k < wl; k++) { unsigned b; sscanf(t[i].hex + 2 * k, "%2x", &b); want[k] = (uint8_t)b; }
        memset(got, 0xa5, sizeof got);
        bst_e_Leb128Written w = leb128_encode_into(t[i].v, got, sizeof got);
        CHECK(w.tag == bst_e_Leb128Written_Ok && w.u.Ok == wl && memcmp(got, want, wl) == 0, "encode %llu", (unsigned long long)t[i].v);
        uint64_t v = 0;
        size_t used = 0;
        CHECK(dec(want, wl, 0, &v, &used) == OK && v == t[i].v && used == wl, "decode %s", t[i].hex);
    }
}

static void verdicts(void) {
    uint64_t v;
    size_t used;
    static const uint8_t trunc1[] = {0x80}, trunc2[] = {0xff, 0xff};
    CHECK(dec(trunc1, 1, 0, &v, &used) == TRUNCATED, "one byte with the continuation bit");
    CHECK(dec(trunc2, 2, 0, &v, &used) == TRUNCATED, "two bytes with the continuation bit");
    CHECK(dec(trunc1, 0, 0, &v, &used) == TRUNCATED, "empty input");
    CHECK(dec(trunc1, 1, 1, &v, &used) == TRUNCATED, "offset at the end");
    CHECK(dec(trunc1, 1, 2, &v, &used) == TRUNCATED, "offset past the end");
    uint8_t ten[11];
    memset(ten, 0xff, sizeof ten);
    ten[9] = 0x02;
    CHECK(dec(ten, 11, 0, &v, &used) == OVERFLOW_, "tenth byte 2: needs 65 bits");
    ten[9] = 0x01;
    CHECK(dec(ten, 11, 0, &v, &used) == OK && v == ~0ull && used == 10, "tenth byte 1: the largest value");
    ten[9] = 0x81;
    CHECK(dec(ten, 11, 0, &v, &used) == TOO_LONG, "eleven bytes");
    ten[9] = 0x00;
    CHECK(dec(ten, 10, 0, &v, &used) == OK && used == 10, "padding to ten bytes is accepted");
    /* The decoder takes one value and says how much it used. */
    static const uint8_t two[] = {0x81, 0x01, 0x05};
    CHECK(dec(two, 3, 0, &v, &used) == OK && v == 129 && used == 2, "first of two values");
    CHECK(dec(two, 3, 2, &v, &used) == OK && v == 5 && used == 1, "second of two values");
}

static void fuzz(long rounds) {
    uint8_t buf[32];
    for (long r = 0; r < rounds; r++) {
        size_t n = rnd() % 24;
        for (size_t i = 0; i < n; i++) {
            uint64_t d = rnd() % 4;   /* mostly continuation bytes, so values get long */
            buf[i] = d < 2 ? (uint8_t)(rnd() | 0x80) : d == 2 ? (uint8_t)(rnd() & 0x7f) : (uint8_t)rnd();
        }
        agree(buf, n, n ? rnd() % (n + 2) : 0);
        /* An encoding of a random value, then a few bytes changed. */
        uint64_t x = rnd() >> (rnd() % 64);
        uint8_t enc[16];
        size_t el = ref_encode(x, enc);
        uint64_t v = 0;
        size_t used = 0;
        CHECK(dec(enc, el, 0, &v, &used) == OK && v == x && used == el, "round trip of %llu", (unsigned long long)x);
        for (int k = rnd() % 3; k > 0; k--) enc[rnd() % el] = (uint8_t)rnd();
        agree(enc, el, 0);
    }
}

/* leb128_encode_into writes into a buffer the caller owns: it must agree
 * with the C encoder, write nothing past the encoding (the bytes after it
 * keep a sentinel), and refuse a buffer that is too small. Each buffer is a
 * separate allocation of exactly the size given, so AddressSanitizer would
 * catch a write outside it. */
static void into(long rounds) {
    for (long r = 0; r < rounds; r++) {
        uint64_t x = rnd() >> (rnd() % 64);
        uint8_t want[16];
        size_t need = ref_encode(x, want), extra = rnd() % 3 == 0 ? rnd() % 8 : 0;
        uint8_t *small = malloc(need - 1 ? need - 1 : 1);
        bst_e_Leb128Written w = leb128_encode_into(x, small, need - 1);
        CHECK(w.tag == bst_e_Leb128Written_TooSmall && w.u.TooSmall == need, "%llu into %zu bytes: not refused", (unsigned long long)x, need - 1);
        free(small);
        uint8_t *out = malloc(need + extra);
        memset(out, 0xa5, need + extra);
        w = leb128_encode_into(x, out, need + extra);
        CHECK(w.tag == bst_e_Leb128Written_Ok && w.u.Ok == need && memcmp(out, want, need) == 0, "encode of %llu differs", (unsigned long long)x);
        for (size_t i = need; i < need + extra; i++) CHECK(out[i] == 0xa5, "byte %zu past the encoding was written", i);
        free(out);
    }
}

int main(int argc, char **argv) {
    long rounds = argc > 1 ? atol(argv[1]) : 200000;
    vectors();
    verdicts();
    fuzz(rounds);
    into(rounds / 10);
    printf("leb128: %s (%ld fuzz rounds)\n", failures ? "FAILED" : "ok", rounds);
    return failures != 0;
}
