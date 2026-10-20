#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define L 1024ULL
int main(void) {
  uint64_t *a = calloc(L, 8), *b = calloc(L, 8), *r = calloc(L + L, 8);
  uint64_t s = 12345, total = 0;
  for (uint64_t i = 0; i < L; i++) {
    s = (s * 1103515245 + 12345) % 4294967296ull; a[i] = s;
    s = (s * 1103515245 + 12345) % 4294967296ull; b[i] = s;
  }
  for (int rep = 0; rep < 8; rep++) {
    for (uint64_t i = 0; i < L + L; i++) r[i] = 0;
    for (uint64_t i = 0; i < L; i++) {
      uint64_t ai = a[i] % 4294967296ull, carry = 0;
      for (uint64_t j = 0; j < L; j++) {
        uint64_t cur = r[i + j] % 4294967296ull;
        uint64_t t = ai * (b[j] % 4294967296ull) + cur + carry;
        r[i + j] = t % 4294967296ull;
        carry = t / 4294967296ull;
      }
      r[i + L] = carry;
    }
    for (uint64_t i = 0; i < L; i++) { uint64_t v = r[i] % 4294967296ull; a[i] = v; total += v; }
  }
  printf("%llu\n", (unsigned long long)total);
  free(a); free(b); free(r);
  return 0;
}
