#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#define N 262144ULL
static void push(uint64_t *h, uint64_t n, uint64_t v) {
  uint64_t i = n + 1;
  while (i > 1 && h[i / 2] > v) { h[i] = h[i / 2]; i /= 2; }
  h[i] = v;
}
static void pop(uint64_t *h, uint64_t n) {
  uint64_t v = h[n], m = n - 1, i = 1;
  int go = 1;
  while (go && 2 * i <= m) {
    uint64_t c = 2 * i;
    if (c + 1 <= m && h[c + 1] < h[c]) c++;
    uint64_t w = h[c];
    if (w >= v) go = 0; else { h[i] = w; i = c; }
  }
  h[i] = v;
}
int main(void) {
  uint64_t *h = calloc(N + 1, 8);
  uint64_t x = 12345, sum = 0;
  for (int round = 0; round < 4; round++) {
    uint64_t n = 0;
    while (n < N) { x = (x * 1103515245 + 12345) % 4294967296ull; push(h, n, x); n++; }
    while (n > 0) { sum = sum * 31 + h[1]; pop(h, n); n--; }
  }
  printf("%llu\n", (unsigned long long)sum); free(h); return 0;
}
