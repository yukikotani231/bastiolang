#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static uint64_t slot(uint64_t k) { return (k * 11400714819323198485ull) >> 44; }
static void insert(uint64_t *t, uint64_t k) {
  uint64_t i = slot(k), steps = 0;
  while (t[i] != 0 && t[i] != k && steps < 1048576) { i = (i + 1) % 1048576; steps++; }
  if (t[i] == 0) t[i] = k;
}
static int has(uint64_t *t, uint64_t k) {
  uint64_t i = slot(k), steps = 0;
  while (t[i] != 0 && t[i] != k && steps < 1048576) { i = (i + 1) % 1048576; steps++; }
  return t[i] == k;
}
int main(void) {
  uint64_t *t = calloc(1048576, 8), x = 12345, hits = 0, y = 12345;
  for (int n = 0; n < 500000; n++) { x = (x * 1103515245 + 12345) % 4294967296ull; insert(t, x + 1); }
  for (int m = 0; m < 8000000; m++) { y = (y * 1103515245 + 12345) % 4294967296ull; if (has(t, y + 1)) hits++; }
  printf("%llu\n", (unsigned long long)hits); free(t); return 0;
}
