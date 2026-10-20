#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define SIZE 33554432ULL
#define PAT 12ULL
static uint64_t count(const uint8_t *b, const uint8_t *p) {
  uint64_t hits = 0;
  for (uint64_t i = 0; i + PAT <= SIZE; i++) {
    uint64_t k = 0;
    while (k < PAT && b[i + k] == p[k]) k++;
    if (k == PAT) hits++;
  }
  return hits;
}
int main(void) {
  uint8_t *b = calloc(SIZE, 1), p[PAT]; uint64_t x = 12345;
  for (uint64_t i = 0; i < SIZE; i++) {
    x = x * 6364136223846793005ULL + 1442695040888963407ULL;
    b[i] = 97 + x / 4294967296ULL % 3;
  }
  for (uint64_t k = 0; k < PAT; k++) p[k] = 97 + k * 7 % 3;
  uint64_t t = 0;
  for (int r = 0; r < 4; r++) t = t * 1000003 + count(b, p);
  printf("%llu\n", (unsigned long long)t); return 0;
}
