#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define SIZE 16777216ULL
int main(void) {
  uint8_t *b = calloc(SIZE, 1);
  uint64_t x = 12345, total = 0;
  for (uint64_t i = 0; i < SIZE; i++) { x = x * 6364136223846793005ull + 1442695040888963407ull; b[i] = (uint8_t)(x / 4294967296ull % 256); }
  for (int pass = 0; pass < 8; pass++) {
    uint64_t a = 1, s = 0;
    for (uint64_t pos = 0; pos < SIZE;) {
      uint64_t end = pos + 5552; if (end > SIZE) end = SIZE;
      for (uint64_t k = pos; k < end; k++) { a += b[k]; s += a; }
      a %= 65521; s %= 65521; pos = end;
    }
    total += s * 65536 + a;
  }
  printf("%llu\n", (unsigned long long)total);
  free(b);
  return 0;
}
