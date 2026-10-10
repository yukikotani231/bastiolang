/* Tests the Bastio DER library through its generated C API (der.h):
 *   1. hand-written vectors for each rule;
 *   2. a differential fuzz against an independent checker written here in C,
 *      on random DER trees, then damaged ones;
 *   3. walking a structure with der_read, the way a C caller would;
 *   4. with `bench`, throughput against the C checker.
 * Built under AddressSanitizer + UBSan by run.sh. */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "der.h"

static int failures;
#define CHECK(c, ...) do { if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

/* ---- The reference checker: pointers instead of offsets, a header parser
 * that reads the length bytes with a shift, and recursion that passes the
 * remaining span. Same rules as der.bastio (see README.md). ---- */

typedef struct { const uint8_t *base; long bad; } ref_ctx;

/* Parses the header at p (no further than end). Sets *val and *next; 0 on
 * success, -1 if malformed. */
static int ref_header(const uint8_t *p, const uint8_t *end, const uint8_t **val, const uint8_t **next) {
    if (end - p < 2 || (p[0] & 0x1f) == 0x1f) return -1;
    size_t len = p[1];
    const uint8_t *q = p + 2;
    if (len & 0x80) {
        size_t k = len & 0x7f;
        if (k == 0 || k > 4 || (size_t)(end - q) < k || q[0] == 0) return -1;
        len = 0;
        for (size_t j = 0; j < k; j++) len = len << 8 | q[j];
        if (len < 0x80) return -1;
        q += k;
    }
    if ((size_t)(end - q) < len) return -1;
    *val = q;
    *next = q + len;
    return 0;
}

/* Elements filling [p, end) at nesting `depth`; their count, or -1. */
static long ref_elems(ref_ctx *c, const uint8_t *p, const uint8_t *end, int depth) {
    long count = 0;
    while (p < end) {
        const uint8_t *val, *next;
        if (ref_header(p, end, &val, &next)) { c->bad = p - c->base; return -1; }
        if (p[0] & 0x20) {
            if (depth >= 64) { c->bad = p - c->base; return -1; }
            long n = ref_elems(c, val, next, depth + 1);
            if (n < 0) return -1;
            count += n;
        }
        count++;
        p = next;
    }
    return count;
}

/* Element count, or -1 - offset. */
static long ref_check(const uint8_t *s, size_t n) {
    ref_ctx c = { s, 0 };
    const uint8_t *val, *next;
    if (ref_header(s, s + n, &val, &next)) return -1;
    if (next != s + n) return -1 - (next - s);
    if (!(s[0] & 0x20)) return 1;
    long k = ref_elems(&c, val, next, 1);
    return k < 0 ? -1 - c.bad : k + 1;
}

static long verdicts[2];

static void agree(const uint8_t *s, size_t n) {
    long r = ref_check(s, n);
    verdicts[r >= 0]++;
    bst_e_Der d = der_check(s, n);
    if (r >= 0) CHECK(d.tag == bst_e_Der_Ok && d.u.Ok == (uint64_t)r, "%zu bytes: want %ld elements, got tag %llu value %llu", n, r, (unsigned long long)d.tag, (unsigned long long)(d.tag ? d.u.Bad : d.u.Ok));
    else CHECK(d.tag == bst_e_Der_Bad && d.u.Bad == (uint64_t)(-1 - r), "%zu bytes: want error at %ld", n, -1 - r);
}

static void vectors(void) {
    static const struct { const char *hex; long want; } v[] = {
        {"0500", 1},                     /* NULL */
        {"020101", 1},                   /* INTEGER 1 */
        {"3006020101020102", 3},         /* SEQUENCE { 1, 2 } */
        {"3000 05", -3},                 /* trailing byte after the top element */
        {"3003300100", -5},              /* the inner SEQUENCE holds 1 byte, too short for an element */
        {"3003300205", -3},             /* the inner SEQUENCE runs past its container */
        {"300430020500", 3},             /* nested */
        {"1f0100", -1},                  /* high-tag-number form */
        {"0480", -1},                    /* indefinite length */
        {"04850000000001", -1},          /* 5 length bytes */
        {"048100", -1},                  /* long form for length 0 */
        {"04820001ff", -1},              /* leading zero length byte */
        {"05", -1}, {"", -1},            /* too short */
        {"048180", -1},                  /* 128 bytes promised, none there */
    };
    for (size_t i = 0; i < sizeof v / sizeof v[0]; i++) {
        uint8_t buf[64];
        size_t n = 0;
        for (const char *h = v[i].hex; *h; h++) {
            if (*h == ' ') continue;
            unsigned x;
            sscanf(h, "%2x", &x);
            buf[n++] = (uint8_t)x;
            h++;
        }
        long r = ref_check(buf, n);
        CHECK(r == v[i].want, "reference on vector %zu: %ld, want %ld", i, r, v[i].want);
        agree(buf, n);
    }
}

/* Wraps buf[0..n) in a SEQUENCE header (minimal length form); new size. */
static size_t wrap(uint8_t *buf, size_t n) {
    size_t hdr = n < 128 ? 2 : n < 256 ? 3 : 4;
    memmove(buf + hdr, buf, n);
    buf[0] = 0x30;
    if (hdr == 2) buf[1] = (uint8_t)n;
    else if (hdr == 3) { buf[1] = 0x81; buf[2] = (uint8_t)n; }
    else { buf[1] = 0x82; buf[2] = (uint8_t)(n >> 8); buf[3] = (uint8_t)n; }
    return hdr + n;
}

/* The limits: the shortest long-form length, and the deepest nesting. */
static void limits(void) {
    static uint8_t buf[1024];
    buf[0] = 0x04; buf[1] = 0x81; buf[2] = 127;           /* long form for 127: not DER */
    CHECK(ref_check(buf, 130) == -1, "reference accepts a long-form 127");
    agree(buf, 130);
    buf[2] = 128;                                          /* long form for 128: fine */
    CHECK(ref_check(buf, 131) == 1, "reference rejects a long-form 128");
    agree(buf, 131);
    for (int k = 63; k <= 66; k++) {                      /* k SEQUENCEs, one inside the other */
        size_t n = 0;
        for (int j = 0; j < k; j++) n = wrap(buf, n);
        long r = ref_check(buf, n);
        CHECK(k <= 64 ? r == k : r < 0, "reference on %d nested SEQUENCEs: %ld", k, r);
        agree(buf, n);
    }
}

static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint64_t rnd(void) { rng ^= rng >> 12; rng ^= rng << 25; rng ^= rng >> 27; return rng * 0x2545f4914f6cdd1dull; }

/* Writes a random element at p (at most `room` bytes) with up to `depth`
 * more levels below it; returns its size. Lengths use the minimal form. */
static size_t gen(uint8_t *p, size_t room, int depth) {
    uint8_t body[4096];
    size_t len = 0;
    int cons = depth > 0 && rnd() % 2;
    if (cons) {
        int kids = rnd() % 4;
        for (int k = 0; k < kids && len < 3000; k++) len += gen(body + len, 3000 - len, depth - 1);
    } else {
        len = rnd() % 4 == 0 ? 120 + rnd() % 200 : rnd() % 8;
        for (size_t k = 0; k < len; k++) body[k] = (uint8_t)rnd();
    }
    size_t hdr = len < 128 ? 2 : len < 256 ? 3 : 4;
    if (hdr + len > room) {                 /* no room: a NULL, or nothing */
        if (room < 2) return 0;
        p[0] = 5;
        p[1] = 0;
        return 2;
    }
    p[0] = (uint8_t)((rnd() % 31) | (cons ? 0x20 : 0) | (rnd() % 4) << 6);
    if (hdr == 2) p[1] = (uint8_t)len;
    else if (hdr == 3) { p[1] = 0x81; p[2] = (uint8_t)len; }
    else { p[1] = 0x82; p[2] = (uint8_t)(len >> 8); p[3] = (uint8_t)len; }
    memcpy(p + hdr, body, len);
    return hdr + len;
}

static void fuzz(long rounds) {
    static uint8_t buf[8192];
    for (long r = 0; r < rounds; r++) {
        size_t n = gen(buf, sizeof buf - 8, 1 + rnd() % 5);
        switch (rnd() % 5) {
        case 0: break;                                              /* intact */
        case 1: if (n) buf[rnd() % n] = (uint8_t)rnd(); break;      /* any byte */
        case 2: if (n > 1) buf[1 + rnd() % 2 % n] ^= 1 << rnd() % 8; break;  /* a header bit */
        case 3: if (n) n -= 1 + rnd() % n; break;                   /* cut short */
        default: buf[n++] = (uint8_t)rnd(); break;                  /* trailing byte */
        }
        uint8_t *s = malloc(n + 1);   /* its own allocation, for ASan */
        memcpy(s, buf, n);
        agree(s, n);
        free(s);
    }
}

/* Walks a checked structure with der_read and counts its elements, as a C
 * caller would. Every span it is handed is inside the input (proved). */
static long walk(const uint8_t *s, size_t n, uint64_t at, uint64_t end) {
    long count = 0;
    while (at < end) {
        bst_e_Tlv t = der_read(s, n, at);
        if (t.tag != bst_e_Tlv_Ok) return -1;
        if (t.u.Ok.tag & 0x20) {
            long k = walk(s, n, t.u.Ok.value, t.u.Ok.end);
            if (k < 0) return -1;
            count += k;
        }
        count++;
        at = t.u.Ok.end;
    }
    return count;
}

static void walks(long rounds) {
    static uint8_t buf[8192];
    for (long r = 0; r < rounds; r++) {
        size_t n = gen(buf, sizeof buf, 1 + rnd() % 5);
        bst_e_Der d = der_check(buf, n);
        CHECK(d.tag == bst_e_Der_Ok && walk(buf, n, 0, n) == (long)d.u.Ok, "walk of %zu bytes", n);
    }
    CHECK(der_read(buf, 4, 5).tag == bst_e_Tlv_Bad, "der_read past the end");
}

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

static void bench(void) {
    /* One SEQUENCE of ~64 MiB holding many small random trees. */
    size_t cap = 64u << 20, n = 6;
    uint8_t *s = malloc(cap);
    while (n + 8192 < cap) n += gen(s + n, 8192, 3);
    size_t len = n - 6;
    s[0] = 0x30; s[1] = 0x84; s[2] = len >> 24; s[3] = len >> 16; s[4] = len >> 8; s[5] = len;
    double best_b = 1e9, best_c = 1e9;
    for (int k = 0; k < 5; k++) {
        double t0 = now();
        bst_e_Der d = der_check(s, n);
        double t1 = now();
        long r = ref_check(s, n);
        double t2 = now();
        CHECK(d.tag == bst_e_Der_Ok && d.u.Ok == (uint64_t)r, "bench check");
        if (t1 - t0 < best_b) best_b = t1 - t0;
        if (t2 - t1 < best_c) best_c = t2 - t1;
    }
    printf("check %zu MiB: bastio %.0f MiB/s, C reference %.0f MiB/s (best of 5)\n",
           n >> 20, (n >> 20) / best_b, (n >> 20) / best_c);
    free(s);
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "bench") == 0) { bench(); return failures != 0; }
    long rounds = argc > 1 ? atol(argv[1]) : 100000;
    vectors();
    limits();
    fuzz(rounds);
    walks(rounds / 10);
    printf("der: %s (%ld fuzz rounds: %ld accepted, %ld rejected)\n", failures ? "FAILED" : "ok", rounds, verdicts[1], verdicts[0]);
    return failures != 0;
}
