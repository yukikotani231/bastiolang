#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 1048576ULL
int main(void) {
  uint64_t *t = calloc(N + 1, 8);
  uint64_t x = 12345, total = 0;
  for (uint64_t k = 0; k < 2097152; k++) {
    x = (x * 1103515245 + 12345) % 4294967296ull;
    uint64_t i = x / 16 % N + 1;
    uint64_t v = x / 4096 % 100;
    while (i <= N) { t[i] += v; i += i & (0 - i); }
    x = (x * 1103515245 + 12345) % 4294967296ull;
    uint64_t j = x / 16 % N + 1, s = 0;
    while (j > 0) { s += t[j]; j &= j - 1; }
    total += s;
  }
  printf("%llu\n", (unsigned long long)total);
  free(t);
  return 0;
}
