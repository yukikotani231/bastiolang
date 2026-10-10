/* C twin of der.bastio: the same input, and the same library code as
 * examples/der/der.bastio (header returning a struct, recursive check,
 * der_read), so the C compiler sees the same shape. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#define BODY 16777215u

static const uint8_t tmpl[35] = {48,33,2,1,5,4,4,222,173,190,239,48,17,6,3,42,3,4,12,5,104,101,108,108,111,1,1,255,5,0,160,3,2,1,7};

typedef struct { uint8_t tag; uint64_t value, end; } elem;
typedef struct { uint64_t tag; union { elem ok; uint64_t bad; } u; } tlv;
typedef struct { uint64_t tag; uint64_t v; } der;

static tlv header(const uint8_t *s, uint64_t at, uint64_t limit) {
    tlv bad = { 1, { .bad = at } };
    if (limit - at < 2) return bad;
    uint64_t tag = s[at], b = s[at + 1];
    if (tag % 32 == 31 || b == 128 || b > 132) return bad;
    uint64_t p = at + 2, size = b;
    if (b > 128) {
        uint64_t k = b - 128;
        if (limit - p < k || s[p] == 0) return bad;
        size = 0;
        for (uint64_t j = 0; j < k; j++) size = size * 256 + s[p + j];
        if (size < 128) return bad;
        p += k;
    }
    if (limit - p < size) return bad;
    return (tlv){ 0, { .ok = { (uint8_t)tag, p, p + size } } };
}

static int constructed(uint8_t tag) { return tag / 32 % 2 == 1; }

static der elems(const uint8_t *s, uint64_t at, uint64_t limit, uint64_t depth) {
    uint64_t p = at, count = 0;
    while (p < limit) {
        tlv t = header(s, p, limit);
        if (t.tag) return (der){ 1, t.u.bad };
        if (constructed(t.u.ok.tag)) {
            if (depth >= 64) return (der){ 1, p };
            der d = elems(s, t.u.ok.value, t.u.ok.end, depth + 1);
            if (d.tag) return d;
            count += d.v;
        }
        count++;
        p = t.u.ok.end;
    }
    return (der){ 0, count };
}

static der der_check(const uint8_t *s, uint64_t n) {
    tlv t = header(s, 0, n);
    if (t.tag) return (der){ 1, t.u.bad };
    if (t.u.ok.end != n) return (der){ 1, t.u.ok.end };
    if (!constructed(t.u.ok.tag)) return (der){ 0, 1 };
    der d = elems(s, t.u.ok.value, t.u.ok.end, 1);
    if (d.tag) return d;
    return (der){ 0, d.v + 1 };
}

static tlv der_read(const uint8_t *s, uint64_t n, uint64_t at) {
    if (at > n) return (tlv){ 1, { .bad = at } };
    return header(s, at, n);
}

int main(void) {
    uint8_t *b = calloc(BODY + 5, 1);
    b[0] = 48; b[1] = 131; b[2] = 255; b[3] = 255; b[4] = 255;
    for (uint64_t i = 0; i < BODY; i++) b[5 + i] = tmpl[i % 35];
    uint64_t total = 0;
    for (int round = 0; round < 20; round++) {
        der d = der_check(b, BODY + 5);
        if (!d.tag) total += d.v;
    }
    tlv t = der_read(b, BODY + 5, 5);
    if (!t.tag) total += t.u.ok.end - t.u.ok.value;
    free(b);
    printf("%llu\n", (unsigned long long)total);
    return 0;
}
