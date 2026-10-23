/* Tests the Bastio QOI library through its C API (qoi.h, generated from the
 * `export fn`s in lib.bastio):
 *   1. a hand-checked vector and the parameter checks;
 *   2. round trips, with every encoding compared byte for byte with an
 *      independent encoder written here in C the way the reference
 *      implementation (qoi.h) writes it, and `qoi_encode_into` compared
 *      with `qoi_encode`;
 *   3. a differential fuzz of the decoder against an independent strict C
 *      decoder: mutated, truncated and extended encodings, compared by
 *      verdict, error offset and pixels;
 *   4. running out of memory, when run.sh caps allocations;
 *   5. with `bench`, encode and decode throughput against the C versions.
 * Built under AddressSanitizer + UBSan by run.sh, so a stray read or write in
 * the generated code would stop the test. */
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "qoi.h"

#define MAX_PIXELS 400000000ull

static int failures;
#define CHECK(c, ...) do { if (!(c)) { failures++; fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__); fprintf(stderr, __VA_ARGS__); fputc('\n', stderr); } } while (0)

/* xorshift64*, seeded, so a failure can be replayed. */
static uint64_t rng = 0x9e3779b97f4a7c15ull;
static uint64_t rnd(void) { rng ^= rng >> 12; rng ^= rng << 25; rng ^= rng >> 27; return rng * 0x2545f4914f6cdd1dull; }

/* ---- The C side: an encoder in the shape of the reference implementation,
 * with signed differences, and a strict decoder. */
typedef union { struct { uint8_t r, g, b, a; } c; uint32_t v; } px_t;
static unsigned hash(px_t p) { return (p.c.r * 3u + p.c.g * 5u + p.c.b * 7u + p.c.a * 11u) % 64; }
static void put32(uint8_t *o, uint32_t v) { o[0] = v >> 24; o[1] = v >> 16; o[2] = v >> 8; o[3] = v; }
static uint32_t get32(const uint8_t *s) { return (uint32_t)s[0] << 24 | (uint32_t)s[1] << 16 | (uint32_t)s[2] << 8 | s[3]; }

static size_t ref_encode(const uint8_t *pix, uint32_t w, uint32_t h, int ch, int cs, uint8_t *out) {
    memcpy(out, "qoif", 4);
    put32(out + 4, w);
    put32(out + 8, h);
    out[12] = (uint8_t)ch;
    out[13] = (uint8_t)cs;
    size_t o = 14;
    px_t index[64];
    memset(index, 0, sizeof index);
    px_t prev = { .c = { 0, 0, 0, 255 } }, px = prev;
    int run = 0;
    size_t len = (size_t)w * h * ch;
    for (size_t i = 0; i < len; i += ch) {
        px.c.r = pix[i]; px.c.g = pix[i + 1]; px.c.b = pix[i + 2];
        if (ch == 4) px.c.a = pix[i + 3];
        if (px.v == prev.v) {
            run++;
            if (run == 62 || i + ch == len) { out[o++] = 0xc0 | (run - 1); run = 0; }
        } else {
            if (run > 0) { out[o++] = 0xc0 | (run - 1); run = 0; }
            unsigned k = hash(px);
            if (index[k].v == px.v) {
                out[o++] = (uint8_t)k;
            } else {
                index[k] = px;
                if (px.c.a == prev.c.a) {
                    signed char vr = (signed char)(px.c.r - prev.c.r);
                    signed char vg = (signed char)(px.c.g - prev.c.g);
                    signed char vb = (signed char)(px.c.b - prev.c.b);
                    signed char vgr = vr - vg, vgb = vb - vg;
                    if (vr > -3 && vr < 2 && vg > -3 && vg < 2 && vb > -3 && vb < 2) {
                        out[o++] = 0x40 | (vr + 2) << 4 | (vg + 2) << 2 | (vb + 2);
                    } else if (vgr > -9 && vgr < 8 && vg > -33 && vg < 32 && vgb > -9 && vgb < 8) {
                        out[o++] = 0x80 | (vg + 32);
                        out[o++] = (vgr + 8) << 4 | (vgb + 8);
                    } else {
                        out[o++] = 0xfe; out[o++] = px.c.r; out[o++] = px.c.g; out[o++] = px.c.b;
                    }
                } else {
                    out[o++] = 0xff; out[o++] = px.c.r; out[o++] = px.c.g; out[o++] = px.c.b; out[o++] = px.c.a;
                }
            }
        }
        prev = px;
    }
    memset(out + o, 0, 7);
    out[o + 7] = 1;
    return o + 8;
}

/* The verdicts of qoi_decode, with its error offsets: 0 and the pixels in
 * *out (malloc'd), or TOO_BIG, or -1 - (offset of the first bad byte). */
#define TOO_BIG (-(1LL << 62))
static long long ref_decode(const uint8_t *s, size_t n, uint64_t max_pixels, uint8_t **out, uint32_t *w, uint32_t *h, int *ch) {
    *out = NULL;
    if (n < 22) return -1 - (long long)n;
    if (memcmp(s, "qoif", 4) != 0) return -1;
    *w = get32(s + 4);
    *h = get32(s + 8);
    *ch = s[12];
    if (*w == 0) return -1 - 4;
    if (*h == 0) return -1 - 8;
    if (*ch != 3 && *ch != 4) return -1 - 12;
    if (s[13] > 1) return -1 - 13;
    uint64_t px = (uint64_t)*w * *h;
    if (px > MAX_PIXELS || px > max_pixels) return TOO_BIG;
    uint8_t *pix = malloc(px * *ch);
    px_t index[64], p = { .c = { 0, 0, 0, 255 } };
    memset(index, 0, sizeof index);
    size_t i = 14, end = n - 8;
    int run = 0;
    long long bad = 0;
    for (uint64_t q = 0; q < px; q++) {
        if (run > 0) {
            run--;
        } else {
            if (i >= end) { bad = -1 - (long long)i; break; }
            uint8_t b1 = s[i];
            if (b1 == 0xfe) {
                if (i + 4 > end) { bad = -1 - (long long)i; break; }
                p.c.r = s[i + 1]; p.c.g = s[i + 2]; p.c.b = s[i + 3]; i += 4;
            } else if (b1 == 0xff) {
                if (i + 5 > end) { bad = -1 - (long long)i; break; }
                p.c.r = s[i + 1]; p.c.g = s[i + 2]; p.c.b = s[i + 3]; p.c.a = s[i + 4]; i += 5;
            } else if ((b1 & 0xc0) == 0x00) {
                p = index[b1]; i += 1;
            } else if ((b1 & 0xc0) == 0x40) {
                p.c.r += ((b1 >> 4) & 3) - 2; p.c.g += ((b1 >> 2) & 3) - 2; p.c.b += (b1 & 3) - 2; i += 1;
            } else if ((b1 & 0xc0) == 0x80) {
                if (i + 2 > end) { bad = -1 - (long long)i; break; }
                int vg = (b1 & 0x3f) - 32;
                uint8_t b2 = s[i + 1];
                p.c.r += vg - 8 + ((b2 >> 4) & 15); p.c.g += vg; p.c.b += vg - 8 + (b2 & 15); i += 2;
            } else {
                run = b1 & 0x3f; i += 1;
            }
        }
        index[hash(p)] = p;
        uint8_t *d = pix + q * *ch;
        d[0] = p.c.r; d[1] = p.c.g; d[2] = p.c.b;
        if (*ch == 4) d[3] = p.c.a;
    }
    if (!bad && run > 0) bad = -1 - (long long)(i - 1);   /* a run past the last pixel */
    if (!bad && i != end) bad = -1 - (long long)i;          /* chunks after the last pixel */
    static const uint8_t marker[8] = { 0, 0, 0, 0, 0, 0, 0, 1 };
    for (int k = 0; !bad && k < 8; k++)
        if (s[end + k] != marker[k]) bad = -1 - (long long)(end + k);
    if (bad) { free(pix); return bad; }
    *out = pix;
    return 0;
}

/* ---- Images to encode: runs, repeats, small and large steps, alpha. */
static void make_image(uint8_t *pix, size_t px, int ch) {
    uint8_t c[4] = { 0, 0, 0, 255 };
    for (size_t q = 0; q < px; q++) {
        uint64_t r = rnd();
        switch (r % 8) {
        case 0: case 1: break;                                              /* a run */
        case 2: for (int k = 0; k < 3; k++) c[k] += (uint8_t)(r >> (8 + 2 * k)) % 4 - 2; break;   /* diff */
        case 3: for (int k = 0; k < 3; k++) c[k] += (uint8_t)(r >> (8 + 6 * k)) % 48 - 24; break; /* luma or not */
        case 4: c[3] = (uint8_t)(r >> 32); break;                           /* new alpha */
        case 5: if (q >= 3) memcpy(c, pix + (q - 1 - (r >> 8) % 3) * ch, ch); break;  /* a recent color */
        default: for (int k = 0; k < 3; k++) c[k] = (uint8_t)(r >> (16 + 8 * k)); break;
        }
        if (ch == 3) c[3] = 255;
        memcpy(pix + q * ch, c, ch);
    }
}

static size_t max_size(uint64_t px, int ch) { return px * (ch + 1) + 22; }

/* Encodes a w by h image both ways, checks the bytes are the same, and
 * decodes the result back. */
static void round_trip(uint32_t w, uint32_t h, int ch, int cs) {
    size_t px = (size_t)w * h, len = px * ch;
    uint8_t *pix = malloc(len ? len : 1);
    make_image(pix, px, ch);
    uint8_t *want = malloc(max_size(px, ch));
    size_t wn = ref_encode(pix, w, h, ch, cs, want);

    bst_e_QoiEncoded e = qoi_encode(pix, len, w, h, ch, cs);
    CHECK(e.tag == bst_e_QoiEncoded_Ok, "encode %ux%u ch %d: tag %llu", w, h, ch, (unsigned long long)e.tag);
    if (e.tag == bst_e_QoiEncoded_Ok) {
        CHECK(e.u.Ok.len == wn && memcmp(e.u.Ok.data, want, wn) == 0, "encode %ux%u ch %d: %llu bytes, want %zu, or different bytes",
              w, h, ch, (unsigned long long)e.u.Ok.len, wn);
        free(e.u.Ok.data);
    }

    /* encode_into: an exact worst-case buffer, with a guard byte after it
     * that must not change; then one byte too small. */
    size_t cap = max_size(px, ch);
    uint8_t *into = malloc(cap + 1);
    memset(into, 0xa5, cap + 1);
    bst_e_QoiWritten wr = qoi_encode_into(pix, len, w, h, ch, cs, into, cap);
    CHECK(wr.tag == bst_e_QoiWritten_Ok && wr.u.Ok == wn && memcmp(into, want, wn) == 0, "encode_into %ux%u ch %d", w, h, ch);
    int untouched = 1;
    for (size_t k = wn; k <= cap; k++) untouched &= into[k] == 0xa5;
    CHECK(untouched, "encode_into %ux%u ch %d wrote past the encoding", w, h, ch);
    memset(into, 0xa5, cap + 1);
    wr = qoi_encode_into(pix, len, w, h, ch, cs, into, cap - 1);
    CHECK(wr.tag == bst_e_QoiWritten_TooSmall && wr.u.TooSmall == cap, "encode_into %ux%u ch %d: too small", w, h, ch);
    CHECK(into[0] == 0xa5, "encode_into wrote into a buffer that is too small");
    free(into);

    bst_e_QoiDecoded d = qoi_decode(want, wn, MAX_PIXELS);
    CHECK(d.tag == bst_e_QoiDecoded_Ok, "decode %ux%u ch %d: tag %llu err %llu", w, h, ch, (unsigned long long)d.tag, (unsigned long long)d.u.Err);
    if (d.tag == bst_e_QoiDecoded_Ok) {
        bst_s_QoiImage im = d.u.Ok;
        CHECK(im.width == w && im.height == h && im.channels == (uint64_t)ch && im.colorspace == (uint64_t)cs, "decode %ux%u: header", w, h);
        CHECK(im.pixels.len == len && memcmp(im.pixels.data, pix, len) == 0, "decode %ux%u ch %d: pixels differ", w, h, ch);
        free(im.pixels.data);
    }
    free(want);
    free(pix);
}

/* Decodes `s` with both decoders and compares verdicts, offsets and pixels. */
static long long compare_decode(const uint8_t *s, size_t n, uint64_t max_pixels) {
    uint8_t *rp;
    uint32_t w, h;
    int ch;
    long long want = ref_decode(s, n, max_pixels, &rp, &w, &h, &ch);
    bst_e_QoiDecoded d = qoi_decode(s, n, max_pixels);
    if (want == 0) {
        CHECK(d.tag == bst_e_QoiDecoded_Ok, "decode of %zu bytes: tag %llu (err %llu), want Ok", n, (unsigned long long)d.tag, (unsigned long long)d.u.Err);
        if (d.tag == bst_e_QoiDecoded_Ok) {
            size_t len = (size_t)w * h * ch;
            CHECK(d.u.Ok.pixels.len == len && memcmp(d.u.Ok.pixels.data, rp, len) == 0, "decode of %zu bytes: pixels differ", n);
            free(d.u.Ok.pixels.data);
        }
    } else if (want == TOO_BIG) {
        CHECK(d.tag == bst_e_QoiDecoded_TooBig, "decode of %zu bytes: tag %llu, want TooBig", n, (unsigned long long)d.tag);
    } else {
        CHECK(d.tag == bst_e_QoiDecoded_Err && d.u.Err == (uint64_t)(-1 - want), "decode of %zu bytes: tag %llu offset %llu, want Err at %lld",
              n, (unsigned long long)d.tag, (unsigned long long)d.u.Err, -1 - want);
        if (d.tag == bst_e_QoiDecoded_Ok) free(d.u.Ok.pixels.data);
    }
    free(rp);
    return want;
}

static void fuzz_decoder(int rounds) {
    long long ok = 0, err = 0, big = 0;
    for (int r = 0; r < rounds; r++) {
        uint32_t w = 1 + rnd() % 24, h = 1 + rnd() % 24;
        int ch = 3 + rnd() % 2;
        size_t px = (size_t)w * h;
        uint8_t *pix = malloc(px * ch);
        make_image(pix, px, ch);
        size_t cap = max_size(px, ch) + 64;
        uint8_t *enc = malloc(cap);
        size_t n = ref_encode(pix, w, h, ch, rnd() % 2, enc);
        free(pix);
        /* Mutate: flip bytes, cut the end, or add bytes. */
        switch (rnd() % 4) {
        case 0: break;
        case 1: for (int k = 1 + rnd() % 3; k > 0; k--) enc[rnd() % n] ^= (uint8_t)(1 + rnd() % 255); break;
        case 2: n = rnd() % n; break;
        default: { size_t m = 1 + rnd() % 32; for (size_t k = 0; k < m; k++) enc[n + k] = (uint8_t)rnd(); n += m; } break;
        }
        /* A flipped header byte can claim billions of pixels: the cap keeps
         * both decoders from asking for gigabytes. */
        long long v = compare_decode(enc, n, 1u << 20);
        if (v == 0) ok++; else if (v == TOO_BIG) big++; else err++;
        free(enc);
    }
    printf("fuzz: %d decodes compared (%lld valid, %lld errors, %lld too big)\n", rounds, ok, err, big);
}

static void test_params(void) {
    uint8_t pix[12] = { 0 }, out[64];
    CHECK(qoi_encode(pix, 12, 0, 4, 3, 0).tag == bst_e_QoiEncoded_Bad, "width 0");
    CHECK(qoi_encode(pix, 12, 4, 0, 3, 0).tag == bst_e_QoiEncoded_Bad, "height 0");
    CHECK(qoi_encode(pix, 12, 2, 2, 5, 0).tag == bst_e_QoiEncoded_Bad, "channels 5");
    CHECK(qoi_encode(pix, 12, 4, 1, 3, 2).tag == bst_e_QoiEncoded_Bad, "colorspace 2");
    CHECK(qoi_encode(pix, 11, 4, 1, 3, 0).tag == bst_e_QoiEncoded_Bad, "length does not match");
    CHECK(qoi_encode(pix, 12, 1ull << 32, 1, 3, 0).tag == bst_e_QoiEncoded_Bad, "width 2^32");
    CHECK(qoi_encode(pix, 12, 20001, 20000, 3, 0).tag == bst_e_QoiEncoded_TooBig, "over 400 million pixels");
    CHECK(qoi_encode_into(pix, 12, 4, 1, 4, 0, out, sizeof out).tag == bst_e_QoiWritten_Bad, "into: length does not match");
    CHECK(qoi_encode_into(pix, 12, 20001, 20000, 3, 0, out, sizeof out).tag == bst_e_QoiWritten_TooBig, "into: too big");

    /* One red pixel: hash 50, a difference of (-1, 0, 0) from (0, 0, 0). */
    static const uint8_t red[4] = { 255, 0, 0, 255 };
    static const uint8_t want[23] = { 'q', 'o', 'i', 'f', 0, 0, 0, 1, 0, 0, 0, 1, 4, 0, 0x5a, 0, 0, 0, 0, 0, 0, 0, 1 };
    bst_e_QoiEncoded e = qoi_encode(red, 4, 1, 1, 4, 0);
    CHECK(e.tag == bst_e_QoiEncoded_Ok && e.u.Ok.len == 23 && memcmp(e.u.Ok.data, want, 23) == 0, "one red pixel");
    if (e.tag == bst_e_QoiEncoded_Ok) free(e.u.Ok.data);

    /* Decoder limits: a header claiming 65536 x 65536 pixels. */
    uint8_t huge[22] = { 'q', 'o', 'i', 'f', 0, 1, 0, 0, 0, 1, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 1 };
    CHECK(qoi_decode(huge, 22, MAX_PIXELS).tag == bst_e_QoiDecoded_TooBig, "over the format's pixel limit");
    huge[5] = 0; huge[6] = 4;   /* 1024 x 65536 */
    CHECK(qoi_decode(huge, 22, 1000).tag == bst_e_QoiDecoded_TooBig, "over the caller's pixel limit");
}

/* Runs only when run.sh caps allocations at 64 MiB (and lets malloc fail):
 * a request beyond the cap must come back as OutOfMemory, with nothing
 * leaked (LeakSanitizer checks at exit). */
static void test_out_of_memory(void) {
    const char *opts = getenv("ASAN_OPTIONS");
    if (!opts || !strstr(opts, "max_allocation_size_mb=64")) {
        printf("out of memory: skipped (needs ASAN_OPTIONS=...max_allocation_size_mb=64, as run.sh sets)\n");
        return;
    }
    /* 10000 x 10000 RGBA: 400 MB of pixels, and the data is just a run. */
    uint8_t big[23] = { 'q', 'o', 'i', 'f', 0, 0, 0x27, 0x10, 0, 0, 0x27, 0x10, 4, 0, 0xfd, 0, 0, 0, 0, 0, 0, 0, 1 };
    CHECK(qoi_decode(big, sizeof big, MAX_PIXELS).tag == bst_e_QoiDecoded_OutOfMemory, "decode: out of memory");
    /* 4500 x 4500 RGB: 61 MB in, but room for 81 MB out is needed. */
    size_t len = 4500ull * 4500 * 3;
    uint8_t *pix = calloc(len, 1);
    CHECK(pix != NULL, "calloc of the input");
    if (pix) {
        bst_e_QoiEncoded e = qoi_encode(pix, len, 4500, 4500, 3, 0);
        CHECK(e.tag == bst_e_QoiEncoded_OutOfMemory, "encode: out of memory");
        if (e.tag == bst_e_QoiEncoded_Ok) free(e.u.Ok.data);
        free(pix);
    }
    printf("out of memory: decode and encode report it\n");
}

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec / 1e9; }

static void bench(void) {
    uint32_t w = 4096, h = 4096;
    int ch = 4;
    size_t px = (size_t)w * h, len = px * ch;
    uint8_t *pix = malloc(len);
    make_image(pix, px, ch);
    uint8_t *ref = malloc(max_size(px, ch)), *out = malloc(max_size(px, ch));
    double mib = len / 1048576.0, t0, best_b = 1e9, best_c = 1e9;
    size_t n = 0;
    for (int k = 0; k < 5; k++) {
        t0 = now(); n = ref_encode(pix, w, h, ch, 0, ref); double tc = now() - t0;
        /* Into a buffer, as the C encoder writes: qoi_encode would add a
         * zeroed worst-case allocation and a copy to the exact size. */
        t0 = now(); qoi_encode_into(pix, len, w, h, ch, 0, out, max_size(px, ch)); double tb = now() - t0;
        if (tc < best_c) best_c = tc;
        if (tb < best_b) best_b = tb;
    }
    printf("encode %.0f MiB of pixels: Bastio %.0f MiB/s, C %.0f MiB/s\n", mib, mib / best_b, mib / best_c);
    best_b = best_c = 1e9;
    for (int k = 0; k < 5; k++) {
        uint8_t *rp; uint32_t rw, rh; int rch;
        t0 = now(); ref_decode(ref, n, MAX_PIXELS, &rp, &rw, &rh, &rch); double tc = now() - t0;
        free(rp);
        t0 = now(); bst_e_QoiDecoded d = qoi_decode(ref, n, MAX_PIXELS); double tb = now() - t0;
        if (d.tag == bst_e_QoiDecoded_Ok) free(d.u.Ok.pixels.data);
        if (tc < best_c) best_c = tc;
        if (tb < best_b) best_b = tb;
    }
    printf("decode to %.0f MiB of pixels: Bastio %.0f MiB/s, C %.0f MiB/s\n", mib, mib / best_b, mib / best_c);
    free(out);
    free(ref);
    free(pix);
}

int main(int argc, char **argv) {
    if (argc > 1 && strcmp(argv[1], "bench") == 0) {
        bench();
        return 0;
    }
    test_params();
    int trips = 0;
    for (int ch = 3; ch <= 4; ch++)
        for (uint32_t w = 1; w <= 9; w++)
            for (uint32_t h = 1; h <= 9; h += 4) { round_trip(w, h, ch, (w + h) % 2); trips++; }
    for (int k = 0; k < 200; k++) { round_trip(1 + rnd() % 300, 1 + rnd() % 40, 3 + rnd() % 2, rnd() % 2); trips++; }
    printf("round trips: %d images, each encoding the same bytes as the C encoder\n", trips);
    fuzz_decoder(20000);
    test_out_of_memory();
    if (failures) {
        fprintf(stderr, "%d failure(s)\n", failures);
        return 1;
    }
    printf("all passed\n");
    return 0;
}
