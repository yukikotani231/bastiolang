#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 1048576ULL
static void pass(const uint64_t *src, uint64_t *dst, uint64_t shift) {
  uint64_t *cnt = calloc(256, 8);
  for (uint64_t i = 0; i < N; i++) cnt[src[i] >> shift & 255]++;
  uint64_t sum = 0;
  for (int b = 0; b < 256; b++) { uint64_t c = cnt[b]; cnt[b] = sum; sum += c; }
  for (uint64_t i = 0; i < N; i++) {
    uint64_t v = src[i], d = v >> shift & 255, p = cnt[d];
    cnt[d] = p + 1;
    if (p < N) dst[p] = v;
  }
  free(cnt);
}
int main(void) {
  uint64_t *a = calloc(N, 8), *b = calloc(N, 8);
  uint64_t x = 12345;
  for (uint64_t i = 0; i < N; i++) { x = (x * 1103515245 + 12345) % 4294967296ull; a[i] = x; }
  for (int r = 0; r < 4; r++) { pass(a, b, r * 8); uint64_t *t = a; a = b; b = t; }
  uint64_t s = 0;
  for (uint64_t i = 0; i < N; i += 4096) s = s * 31 + a[i];
  printf("%llu\n", (unsigned long long)s); free(a); free(b); return 0;
}
