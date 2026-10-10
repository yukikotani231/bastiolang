/* Tests the Bastio base64 library through its C API (base64.h):
 *   1. the RFC 4648 test vectors;
 *   2. a differential fuzz against an independent strict decoder written
 *      here in C: random inputs, mutated valid encodings, and round trips;
 *   3. with `bench`, throughput against that C decoder.
 * Built under AddressSanitizer + UBSan by run.sh, so a stray read or write in
 * the generated code would stop the test. */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "base64.h"

static int failures;
#define CHECK(c, ...) do { if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

/* ---- The reference: a table-driven strict decoder, as one would write it in C. */
static const char SYM[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static int8_t VAL[256];
static void ref_init(void) {
    memset(VAL, -1, sizeof VAL);
    for (int i = 0; i < 64; i++) VAL[(unsigned char)SYM[i]] = (int8_t)i;
}
/* Returns the decoded length, or -1 - (offset of the first bad byte). */
static long ref_decode(const uint8_t *s, size_t n, uint8_t *out) {
    if (n % 4) return -1 - (long)n;
    size_t pad = 0;
    if (n && s[n - 1] == '=') pad = (n >= 2 && s[n - 2] == '=') ? 2 : 1;
    size_t o = 0;
    for (size_t i = 0; i < n; i += 4) {
        int last = i + 4 == n;
        int v[4];
        for (int k = 0; k < 4; k++) {
            int padded = last && k >= 4 - (int)pad;
            v[k] = padded ? 0 : VAL[s[i + k]];
            if (v[k] < 0) return -1 - (long)(i + k);
        }
        if (last && pad == 2 && v[1] & 15) return -1 - (long)(i + 1);
        if (last && pad == 1 && v[2] & 3) return -1 - (long)(i + 2);
        uint32_t w = (uint32_t)v[0] << 18 | (uint32_t)v[1] << 12 | (uint32_t)v[2] << 6 | (uint32_t)v[3];
        out[o++] = (uint8_t)(w >> 16);
        if (!(last && pad == 2)) out[o++] = (uint8_t)(w >> 8);
        if (!(last && pad >= 1)) out[o++] = (uint8_t)w;
    }
    return (long)o;
}

/* xorshift64*, seeded, so a failure can be replayed. */
static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint64_t rnd(void) { rng ^= rng >> 12; rng ^= rng << 25; rng ^= rng >> 27; return rng * 0x2545f4914f6cdd1dull; }

/* Decodes `s` with both decoders and compares verdicts, offsets and bytes. */
static void agree(const uint8_t *s, size_t n) {
    uint8_t *want = malloc(n / 4 * 3 + 1);
    long r = ref_decode(s, n, want);
    uint8_t *got = NULL;
    size_t got_len = 0, err = 0;
    int rc = base64_decode(s, n, &got, &got_len, &err);
    if (r >= 0) {
        CHECK(rc == 0, "decode of %zu bytes rejected at %zu, reference accepts", n, err);
        if (rc == 0) CHECK(got_len == (size_t)r && memcmp(got, want, got_len) == 0, "decode of %zu bytes differs from the reference", n);
    } else {
        CHECK(rc == -1, "decode of %zu bytes accepted, reference rejects at %ld", n, -1 - r);
        if (rc == -1) CHECK(err == (size_t)(-1 - r), "error offset %zu, reference %ld", err, -1 - r);
    }
    if (rc == 0) base64_free(got);
    free(want);
}

static void vectors(void) {
    static const char *v[][2] = {
        {"", ""}, {"f", "Zg=="}, {"fo", "Zm8="}, {"foo", "Zm9v"},
        {"foob", "Zm9vYg=="}, {"fooba", "Zm9vYmE="}, {"foobar", "Zm9vYmFy"},
    };
    for (size_t i = 0; i < sizeof v / sizeof v[0]; i++) {
        uint8_t *e, *d;
        size_t el, dl, err;
        CHECK(base64_encode((const uint8_t *)v[i][0], strlen(v[i][0]), &e, &el) == 0, "encode %s", v[i][0]);
        CHECK(el == strlen(v[i][1]) && memcmp(e, v[i][1], el) == 0, "encode \"%s\" gave \"%.*s\"", v[i][0], (int)el, (const char *)e);
        base64_free(e);
        CHECK(base64_decode((const uint8_t *)v[i][1], strlen(v[i][1]), &d, &dl, &err) == 0, "decode %s", v[i][1]);
        CHECK(dl == strlen(v[i][0]) && memcmp(d, v[i][0], dl) == 0, "decode \"%s\"", v[i][1]);
        base64_free(d);
    }
}

static void fuzz(long rounds) {
    uint8_t buf[256], *enc, *dec;
    size_t el, dl, err;
    for (long r = 0; r < rounds; r++) {
        size_t n = rnd() % 64;
        for (size_t i = 0; i < n; i++) buf[i] = (uint8_t)rnd();
        /* Round trip. */
        CHECK(base64_encode(buf, n, &enc, &el) == 0 && el == (n + 2) / 3 * 4, "encode length");
        CHECK(base64_decode(enc, el, &dec, &dl, &err) == 0 && dl == n && memcmp(dec, buf, n) == 0, "round trip of %zu bytes", n);
        base64_free(dec);
        /* A valid encoding with a few bytes changed, often to `=` or a near-symbol. */
        static const uint8_t near[] = "=AZaz09+/-_ \n\0\xff";
        for (int k = rnd() % 3; k >= 0 && el; k--)
            enc[rnd() % el] = rnd() % 2 ? near[rnd() % (sizeof near - 1)] : (uint8_t)rnd();
        agree(enc, el - (el && rnd() % 8 == 0));   /* sometimes cut short */
        base64_free(enc);
        /* Arbitrary bytes, drawn mostly from the alphabet. */
        n = rnd() % 32;
        for (size_t i = 0; i < n; i++) buf[i] = rnd() % 8 ? (uint8_t)SYM[rnd() % 64] : rnd() % 4 ? '=' : (uint8_t)rnd();
        agree(buf, n);
    }
}

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

static void bench(void) {
    size_t n = 48u << 20;   /* 48 MiB raw, 64 MiB encoded */
    uint8_t *raw = malloc(n), *enc, *dec, *ref = malloc(n);
    size_t el, dl, err;
    for (size_t i = 0; i < n; i++) raw[i] = (uint8_t)rnd();
    base64_encode(raw, n, &enc, &el);
    double best_b = 1e9, best_c = 1e9;
    for (int k = 0; k < 5; k++) {
        double t0 = now();
        base64_decode(enc, el, &dec, &dl, &err);
        double t1 = now();
        long r = ref_decode(enc, el, ref);
        double t2 = now();
        CHECK(dl == n && r == (long)n && memcmp(dec, ref, n) == 0, "bench decode");
        base64_free(dec);
        if (t1 - t0 < best_b) best_b = t1 - t0;
        if (t2 - t1 < best_c) best_c = t2 - t1;
    }
    printf("decode %zu MiB: bastio %.0f MiB/s, C reference %.0f MiB/s (best of 5)\n",
           el >> 20, (el >> 20) / best_b, (el >> 20) / best_c);
    base64_free(enc);
    free(raw);
    free(ref);
}

int main(int argc, char **argv) {
    ref_init();
    if (argc > 1 && strcmp(argv[1], "bench") == 0) { bench(); return failures != 0; }
    long rounds = argc > 1 ? atol(argv[1]) : 200000;
    vectors();
    fuzz(rounds);
    printf("base64: %s (%ld fuzz rounds)\n", failures ? "FAILED" : "ok", rounds);
    return failures != 0;
}
