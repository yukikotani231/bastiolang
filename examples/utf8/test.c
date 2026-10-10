/* Tests the Bastio UTF-8 library through its generated C API (utf8.h):
 *   1. hand-picked vectors at every boundary of table 3-7;
 *   2. a differential fuzz against an independent decoder written here in C;
 *   3. with `bench`, validation throughput against that decoder.
 * Built under AddressSanitizer + UBSan by run.sh. */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "utf8.h"

static int failures;
#define CHECK(c, ...) do { if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

/* The reference: decode each sequence by its lead byte's length, then
 * reject what the value rules out (overlong, surrogate, past U+10FFFF),
 * rather than following table 3-7 as the Bastio code does. Returns the
 * number of code points (and stores them in `cps` if non-NULL), or -1 - the
 * offset of the first ill-formed sequence. */
static long ref_decode(const uint8_t *s, size_t n, uint32_t *cps) {
    long count = 0;
    size_t i = 0;
    while (i < n) {
        uint8_t b = s[i];
        size_t len;
        uint32_t cp, min;
        if (b < 0x80) { len = 1; cp = b; min = 0; }
        else if ((b & 0xe0) == 0xc0) { len = 2; cp = b & 0x1f; min = 0x80; }
        else if ((b & 0xf0) == 0xe0) { len = 3; cp = b & 0x0f; min = 0x800; }
        else if ((b & 0xf8) == 0xf0) { len = 4; cp = b & 0x07; min = 0x10000; }
        else return -1 - (long)i;
        if (n - i < len) return -1 - (long)i;
        for (size_t k = 1; k < len; k++) {
            if ((s[i + k] & 0xc0) != 0x80) return -1 - (long)i;
            cp = cp << 6 | (s[i + k] & 0x3f);
        }
        if (cp < min || cp > 0x10ffff || (cp >= 0xd800 && cp <= 0xdfff)) return -1 - (long)i;
        if (cps) cps[count] = cp;
        count++;
        i += len;
    }
    return count;
}

/* Runs both decoders on `s` and compares verdicts, offsets, counts and the
 * decoded code points. The output buffer is exactly as large as needed (or
 * one code point short, to check TooSmall), in its own allocation. */
static void agree(const uint8_t *s, size_t n, int short_out) {
    uint32_t *want = malloc((n + 1) * sizeof *want);
    long r = ref_decode(s, n, want);
    bst_e_Utf8 v = utf8_validate(s, n);
    if (r >= 0) {
        CHECK(v.tag == bst_e_Utf8_Valid && v.u.Valid == (uint64_t)r, "validate %zu bytes: want %ld code points", n, r);
    } else {
        CHECK(v.tag == bst_e_Utf8_Invalid && v.u.Invalid == (uint64_t)(-1 - r), "validate %zu bytes: want error at %ld", n, -1 - r);
    }
    size_t cap = r >= 0 ? (size_t)r * 4 : n * 4;
    if (short_out && cap >= 4) cap -= 4;
    uint8_t *out = malloc(cap + 1);
    bst_e_Decoded32 d = utf8_decode_into(s, n, out, cap);
    if (r < 0) {
        CHECK(d.tag == bst_e_Decoded32_Invalid && d.u.Invalid == (uint64_t)(-1 - r), "decode %zu bytes: want error at %ld", n, -1 - r);
    } else if ((size_t)r * 4 > cap) {
        CHECK(d.tag == bst_e_Decoded32_TooSmall && d.u.TooSmall == (uint64_t)r * 4, "decode %zu bytes into %zu: want TooSmall", n, cap);
    } else {
        CHECK(d.tag == bst_e_Decoded32_Ok && d.u.Ok == (uint64_t)r, "decode %zu bytes: want %ld code points", n, r);
        for (long k = 0; d.tag == bst_e_Decoded32_Ok && k < r; k++) {
            uint32_t got = out[4 * k] | out[4 * k + 1] << 8 | out[4 * k + 2] << 16 | (uint32_t)out[4 * k + 3] << 24;
            CHECK(got == want[k], "code point %ld: got U+%04X, want U+%04X", k, got, want[k]);
        }
    }
    free(out);
    free(want);
}

static void vectors(void) {
    static const struct { const char *s; long want; } v[] = {
        {"", 0}, {"a", 1}, {"\x7f", 1}, {"\xc2\x80", 1}, {"\xdf\xbf", 1},
        {"\xe0\xa0\x80", 1}, {"\xed\x9f\xbf", 1}, {"\xee\x80\x80", 1}, {"\xef\xbf\xbf", 1},
        {"\xf0\x90\x80\x80", 1}, {"\xf4\x8f\xbf\xbf", 1}, {"h\xc3\xa9llo", 5},
        {"\x80", -1}, {"\xc0\x80", -1}, {"\xc1\xbf", -1}, {"\xc2", -1}, {"\xe0\x9f\xbf", -1},
        {"\xed\xa0\x80", -1}, {"\xed\xbf\xbf", -1}, {"\xf0\x8f\xbf\xbf", -1},
        {"\xf4\x90\x80\x80", -1}, {"\xf5\x80\x80\x80", -1}, {"\xff", -1}, {"ab\xe2\x82", -3},
    };
    for (size_t i = 0; i < sizeof v / sizeof v[0]; i++) {
        size_t n = strlen(v[i].s);
        long r = ref_decode((const uint8_t *)v[i].s, n, NULL);
        CHECK(r == v[i].want, "reference on vector %zu: %ld, want %ld", i, r, v[i].want);
        agree((const uint8_t *)v[i].s, n, 0);
    }
}

static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint64_t rnd(void) { rng ^= rng >> 12; rng ^= rng << 25; rng ^= rng >> 27; return rng * 0x2545f4914f6cdd1dull; }

/* Appends the UTF-8 encoding of `cp` (any value, even ill-formed ones such
 * as surrogates, so the decoders must reject them). */
static size_t put(uint8_t *p, uint32_t cp) {
    if (cp < 0x80) { p[0] = cp; return 1; }
    if (cp < 0x800) { p[0] = 0xc0 | cp >> 6; p[1] = 0x80 | (cp & 0x3f); return 2; }
    if (cp < 0x10000) { p[0] = 0xe0 | cp >> 12; p[1] = 0x80 | (cp >> 6 & 0x3f); p[2] = 0x80 | (cp & 0x3f); return 3; }
    p[0] = 0xf0 | (cp >> 18 & 7); p[1] = 0x80 | (cp >> 12 & 0x3f); p[2] = 0x80 | (cp >> 6 & 0x3f); p[3] = 0x80 | (cp & 0x3f); return 4;
}

/* A random code point, drawn mostly near the boundaries that matter. */
static uint32_t pick(void) {
    static const uint32_t edge[] = {0, 0x7f, 0x80, 0x7ff, 0x800, 0xd7ff, 0xd800, 0xdfff, 0xe000, 0xfffd, 0xffff, 0x10000, 0x10ffff, 0x110000, 0x1fffff};
    switch (rnd() % 4) {
    case 0: return rnd() % 128;
    case 1: return edge[rnd() % (sizeof edge / sizeof edge[0])] + rnd() % 3 - 1;
    case 2: return rnd() % 0x800;
    default: return rnd() % 0x110000;
    }
}

static void fuzz(long rounds) {
    uint8_t buf[256];
    for (long r = 0; r < rounds; r++) {
        size_t n = 0, want = rnd() % 48;
        while (n < want) n += put(buf + n, pick());
        /* Damage: change a byte, cut the end, or add stray bytes. */
        switch (rnd() % 4) {
        case 0: if (n) buf[rnd() % n] = (uint8_t)rnd(); break;
        case 1: if (n) n -= rnd() % (n < 4 ? n : 4) + (n > 0 && rnd() % 2); break;
        case 2: buf[n++] = 0x80 | rnd() % 64; break;
        default: break;
        }
        uint8_t *s = malloc(n + 1);  /* its own allocation, for ASan */
        memcpy(s, buf, n);
        agree(s, n, rnd() % 4 == 0);
        free(s);
    }
}

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

static void bench(void) {
    /* 64 MiB of mixed text: mostly ASCII, with 2-, 3- and 4-byte sequences. */
    size_t cap = 64u << 20, n = 0;
    uint8_t *s = malloc(cap);
    while (n + 4 <= cap) {
        uint32_t cp = rnd() % 8 ? 0x20 + rnd() % 0x5f : rnd() % 2 ? 0x80 + rnd() % 0x780 : rnd() % 2 ? 0x4e00 + rnd() % 0x5000 : 0x1f600 + rnd() % 0x50;
        n += put(s + n, cp);
    }
    double best_b = 1e9, best_c = 1e9;
    for (int k = 0; k < 5; k++) {
        double t0 = now();
        bst_e_Utf8 v = utf8_validate(s, n);
        double t1 = now();
        long r = ref_decode(s, n, NULL);
        double t2 = now();
        CHECK(v.tag == bst_e_Utf8_Valid && v.u.Valid == (uint64_t)r, "bench validate");
        if (t1 - t0 < best_b) best_b = t1 - t0;
        if (t2 - t1 < best_c) best_c = t2 - t1;
    }
    printf("validate %zu MiB: bastio %.0f MiB/s, C reference %.0f MiB/s (best of 5)\n",
           n >> 20, (n >> 20) / best_b, (n >> 20) / best_c);
    free(s);
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "bench") == 0) { bench(); return failures != 0; }
    long rounds = argc > 1 ? atol(argv[1]) : 200000;
    vectors();
    fuzz(rounds);
    printf("utf8: %s (%ld fuzz rounds)\n", failures ? "FAILED" : "ok", rounds);
    return failures != 0;
}
