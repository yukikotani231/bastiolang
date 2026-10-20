#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
static uint64_t fmt(uint8_t *out, uint64_t n) {
  uint64_t tmp = n, len = 0;
  while (tmp > 0 && len < 20) { tmp /= 10; len++; }
  if (len == 0) { out[0] = 48; return 1; }
  uint64_t v = n, k = len;
  while (k > 0) { k--; out[k] = (uint8_t)(48 + v % 10); v /= 10; }
  return len;
}
int main(void) {
  uint8_t *out = calloc(64, 1);
  uint64_t x = 12345, sum = 0;
  for (uint64_t i = 0; i < 4194304; i++) {
    x = x * 6364136223846793005ull + 1442695040888963407ull;
    uint64_t n = x / 1024 % 1000000007ull;
    uint64_t len = fmt(out, n);
    for (uint64_t j = 0; j < len; j++) sum += out[j];
  }
  printf("%llu\n", (unsigned long long)sum);
  free(out);
  return 0;
}
