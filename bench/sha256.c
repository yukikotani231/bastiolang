#include <stdint.h>
#include <stdio.h>
static inline uint32_t rotr(uint32_t x, unsigned n) { return (x >> n) | (x << (32 - n)); }
static void compress(uint32_t *h, uint32_t *w, const uint32_t *k) {
  for (int t = 16; t < 64; t++) {
    uint32_t x = w[t - 15], y = w[t - 2];
    uint32_t s0 = rotr(x, 7) ^ rotr(x, 18) ^ (x >> 3);
    uint32_t s1 = rotr(y, 17) ^ rotr(y, 19) ^ (y >> 10);
    w[t] = w[t - 16] + s0 + w[t - 7] + s1;
  }
  uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
  for (int t = 0; t < 64; t++) {
    uint32_t s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
    uint32_t ch = (e & f) ^ ((e ^ 4294967295u) & g);
    uint32_t t1 = hh + s1 + ch + k[t] + w[t];
    uint32_t s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
    uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
    uint32_t t2 = s0 + maj;
    hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
  }
  h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
}
int main(void) {
  uint32_t h[8], w[64], k[64];
  for (uint64_t i = 0; i < 8; i++) h[i] = (uint32_t)(1779033703 + i * 1013904242 % 2147483648);
  uint64_t s = 12345;
  for (int i = 0; i < 64; i++) { s = (s * 1103515245 + 12345) % 4294967296; k[i] = (uint32_t)s; w[i] = 0; }
  for (uint64_t blk = 0; blk < 400000; blk++) {
    for (uint64_t j = 0; j < 16; j++) w[j] = h[j % 8] ^ (uint32_t)(blk * 16 + j);
    compress(h, w, k);
  }
  printf("%llu\n", (unsigned long long)h[0]);
  return 0;
}
