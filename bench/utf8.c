/* C twin of utf8.bastio: the same text, and the same library code as
 * examples/utf8/utf8.bastio (table 3-7 validator and UTF-32 decoder; the
 * decoder runs once, so both have the same callers as in the Bastio build
 * and the C compiler makes the same inlining choices). */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define SIZE 16777216u

static int is_cont(uint8_t b) { return b >= 128 && b <= 191; }

static uint64_t seq(const uint8_t *s, uint64_t n, uint64_t i) {
    uint8_t b0 = s[i];
    if (b0 < 128) return 1;
    if (b0 < 194 || b0 > 244 || i + 1 >= n) return 0;
    uint8_t b1 = s[i + 1], lo = 128, hi = 191;
    if (b0 == 224) lo = 160;
    if (b0 == 237) hi = 159;
    if (b0 == 240) lo = 144;
    if (b0 == 244) hi = 143;
    if (b1 < lo || b1 > hi) return 0;
    if (b0 < 224) return 2;
    if (i + 2 >= n || !is_cont(s[i + 2])) return 0;
    if (b0 < 240) return 3;
    if (i + 3 >= n || !is_cont(s[i + 3])) return 0;
    return 4;
}

/* Code points, or -1 if ill-formed. */
static int64_t validate(const uint8_t *s, uint64_t n) {
    uint64_t i = 0, count = 0;
    while (i < n) {
        while (i < n && s[i] < 128) { i++; count++; }
        if (i < n) {
            uint64_t k = seq(s, n, i);
            if (k == 0) return -1;
            i += k;
            count++;
        }
    }
    return (int64_t)count;
}

/* Decodes into out (4 bytes per code point); code points, or -1. */
static int64_t decode_into(const uint8_t *s, uint64_t n, uint8_t *out, uint64_t cap) {
    int64_t count = validate(s, n);
    if (count < 0 || (uint64_t)count * 4 > cap) return -1;
    uint64_t i = 0, o = 0;
    while (i < n) {
        uint64_t k = seq(s, n, i);
        if (k == 0 || o + 4 > cap) return -1;
        uint64_t b0 = s[i], v = b0;
        if (k == 2) v = b0 % 32 * 64 + s[i + 1] % 64;
        if (k == 3) v = b0 % 16 * 4096 + s[i + 1] % 64 * 64 + s[i + 2] % 64;
        if (k == 4) v = b0 % 8 * 262144 + s[i + 1] % 64 * 4096 + s[i + 2] % 64 * 64 + s[i + 3] % 64;
        out[o] = v % 256; out[o + 1] = v / 256 % 256; out[o + 2] = v / 65536 % 256; out[o + 3] = v / 16777216;
        i += k;
        o += 4;
    }
    return (int64_t)(o / 4);
}

int main(void) {
    uint8_t *b = calloc(SIZE, 1);
    uint64_t x = 12345, i = 0;
    while (i + 4 <= SIZE) {
        x = x * 6364136223846793005ull + 1442695040888963407ull;
        uint64_t r = x / 4294967296u, k = r % 16;
        if (k < 13) { b[i++] = 32 + r / 16 % 95; }
        else if (k == 13) { b[i] = 195; b[i + 1] = 169; i += 2; }
        else if (k == 14) { b[i] = 228; b[i + 1] = 184; b[i + 2] = 173; i += 3; }
        else { b[i] = 240; b[i + 1] = 159; b[i + 2] = 152; b[i + 3] = 128; i += 4; }
    }
    uint64_t total = 0;
    for (int round = 0; round < 20; round++) {
        int64_t c = validate(b, SIZE);
        if (c >= 0) total += (uint64_t)c;
    }
    uint8_t *out = malloc(4 * SIZE);
    int64_t d = decode_into(b, SIZE, out, 4 * SIZE);
    if (d >= 0) total += (uint64_t)d;
    free(out);
    free(b);
    printf("%llu\n", (unsigned long long)total);
    return 0;
}
