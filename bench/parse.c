#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define SIZE 16777216
static uint64_t sum(const uint8_t *b) {
  uint64_t total = 0, cur = 0;
  for (uint64_t i = 0; i < SIZE; i++) {
    uint8_t c = b[i];
    if (c == 44) { total += cur; cur = 0; }
    else cur = cur * 10 + (uint64_t)c - 48;
  }
  return total + cur;
}
int main(void) {
  uint8_t *b = malloc(SIZE);
  uint64_t x = 12345;
  for (uint64_t i = 0; i < SIZE; i++) {
    x = x * 6364136223846793005ull + 1442695040888963407ull;
    uint64_t r = x / 4294967296ull;
    b[i] = (r % 7 == 0) ? 44 : (uint8_t)(48 + r / 8 % 10);
  }
  uint64_t t = 0;
  for (int p = 0; p < 8; p++) t += sum(b);
  printf("%llu\n", (unsigned long long)t);
  free(b); return 0;
}
