#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 1048576ULL
int main(void) {
  uint64_t *p = calloc(N, 8);
  for (uint64_t i = 0; i < N; i++) p[i] = i;
  uint64_t x = 12345;
  for (uint64_t k = 0; k < 2097152; k++) {
    x = (x * 1103515245 + 12345) % 4294967296ull;
    uint64_t ra = x / 16 % N;
    x = (x * 1103515245 + 12345) % 4294967296ull;
    uint64_t rb = x / 16 % N;
    while (p[ra] % N != ra) { uint64_t gg = p[p[ra] % N] % N; p[ra] = gg; ra = gg; }
    while (p[rb] % N != rb) { uint64_t gg = p[p[rb] % N] % N; p[rb] = gg; rb = gg; }
    if (ra != rb) p[ra] = rb;
  }
  uint64_t roots = 0, sum = 0;
  for (uint64_t j = 0; j < N; j++) {
    uint64_t r = j;
    while (p[r] % N != r) { uint64_t gg = p[p[r] % N] % N; p[r] = gg; r = gg; }
    if (r == j) roots++;
    sum += r;
  }
  printf("%llu\n%llu\n", (unsigned long long)roots, (unsigned long long)sum);
  free(p);
  return 0;
}
