#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static void init(uint64_t *a, uint64_t m) {
  for (uint64_t i = 0; i < 400; i++) for (uint64_t j = 0; j < 400; j++) a[i * 400 + j] = (i * m + j) % 10;
}
static void mul(const uint64_t *a, const uint64_t *b, uint64_t *c) {
  for (uint64_t i = 0; i < 400; i++)
    for (uint64_t j = 0; j < 400; j++) {
      uint64_t s = 0;
      for (uint64_t k = 0; k < 400; k++) s += a[i * 400 + k] * b[k * 400 + j];
      c[i * 400 + j] = s;
    }
}
int main(void) {
  uint64_t *a = calloc(160000, 8), *b = calloc(160000, 8), *c = calloc(160000, 8);
  init(a, 3); init(b, 7);
  uint64_t t = 0;
  for (uint64_t r = 0; r < 3; r++) { mul(a, b, c); t += c[12345 + r]; }
  printf("%llu\n", (unsigned long long)t);
  free(a); free(b); free(c); return 0;
}
