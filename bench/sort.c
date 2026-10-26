#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
int main(void) {
  uint64_t *a = calloc(16384, 8), x = 12345;
  for (uint64_t i = 0; i < 16384; i++) { x = (x * 1103515245 + 12345) % 4294967296ull; a[i] = x; }
  for (uint64_t i = 1; i < 16384; i++) {
    uint64_t v = a[i], j = i;
    while (j > 0 && a[j - 1] > v) { a[j] = a[j - 1]; j--; }
    a[j] = v;
  }
  uint64_t sum = 0;
  for (uint64_t i = 0; i < 16384; i++) sum += a[i] * i;
  printf("%llu\n", (unsigned long long)sum); free(a); return 0;
}
