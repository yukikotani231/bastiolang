#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define SIZE 33554432ULL
static uint64_t encode(const uint8_t *b, uint8_t *out) {
  uint64_t o = 0, i = 0;
  while (i < SIZE) {
    uint8_t c = b[i]; uint64_t n = 1;
    while (i + n < SIZE && n < 255 && b[i + n] == c) n++;
    out[o] = (uint8_t)n; out[o + 1] = c; o += 2; i += n;
  }
  return o;
}
int main(void) {
  uint8_t *b = calloc(SIZE, 1), *out = calloc(2 * SIZE, 1); uint64_t x = 12345, i = 0;
  while (i < SIZE) {
    x = x * 6364136223846793005ULL + 1442695040888963407ULL;
    uint64_t r = x / 4294967296ULL, n = r % 16 + 1; uint8_t c = 97 + r / 16 % 4;
    for (uint64_t k = 0; k < n && i < SIZE; k++) b[i++] = c;
  }
  uint64_t t = 0;
  for (int p = 0; p < 4; p++) {
    uint64_t o = encode(b, out); t += o;
    for (uint64_t j = 0; j < o && j < 2 * SIZE; j++) t = t * 31 + out[j];
  }
  printf("%llu\n", (unsigned long long)t); return 0;
}
