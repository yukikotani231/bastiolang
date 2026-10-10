#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static uint64_t sum(const uint64_t *b, uint64_t n) { uint64_t s = 0; for (uint64_t i = 0; i < n; i++) s += b[i]; return s; }
int main(void) {
  uint64_t n = 10000000; uint64_t *b = calloc(n, 8);
  for (uint64_t i = 0; i < n; i++) b[i] = (i * 7 + 3) % 1000;
  uint64_t t = 0;
  for (int r = 0; r < 100; r++) t = (t + sum(b, n)) % 1000000007;
  printf("%llu\n", (unsigned long long)t); free(b); return 0;
}
