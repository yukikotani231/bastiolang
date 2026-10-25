#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 1048576ULL
static uint64_t lower(const uint64_t *a, uint64_t key) {
  uint64_t lo = 0, hi = N;
  while (lo < hi) { uint64_t mid = lo + (hi - lo) / 2; if (a[mid] < key) lo = mid + 1; else hi = mid; }
  return lo;
}
int main(void) {
  uint64_t *a = calloc(N, 8);
  for (uint64_t i = 0; i < N; i++) a[i] = i * 3;
  uint64_t x = 12345, sum = 0;
  for (uint64_t q = 0; q < 4000000; q++) { x = (x * 1103515245 + 12345) % 4294967296ull; sum += lower(a, x % (N * 3)); }
  printf("%llu\n", (unsigned long long)sum); free(a); return 0;
}
