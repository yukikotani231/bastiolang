#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define SIZE 33554432ULL
static uint64_t count(const uint8_t *b) {
  uint64_t lines = 0, words = 0, cur = 0, best = 0; int inword = 0;
  for (uint64_t i = 0; i < SIZE; i++) {
    uint8_t c = b[i];
    if (c == 10) lines++;
    if (c == 32 || c == 10) { inword = 0; cur = 0; }
    else { if (!inword) { inword = 1; words++; } cur++; if (cur > best) best = cur; }
  }
  return lines * 1000003 + words * 31 + best;
}
int main(void) {
  uint8_t *b = calloc(SIZE, 1); uint64_t x = 12345;
  for (uint64_t i = 0; i < SIZE; i++) {
    x = x * 6364136223846793005ULL + 1442695040888963407ULL;
    uint64_t r = x / 4294967296ULL, k = r % 8;
    if (k == 0) b[i] = 32; else if (k == 1 && r / 8 % 8 == 0) b[i] = 10; else b[i] = 97 + r / 8 % 26;
  }
  uint64_t t = 0;
  for (int p = 0; p < 8; p++) t += count(b);
  printf("%llu\n", (unsigned long long)t); return 0;
}
