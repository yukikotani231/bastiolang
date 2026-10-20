#include <stdint.h>
#include <stdio.h>
static inline uint32_t rotl(uint32_t x, unsigned n) { return (x << n) | (x >> (32 - n)); }
int main(void) {
  uint32_t a = 1634760805, b = 857760878, c = 2036477234, d = 1797285236;
  for (uint64_t i = 0; i < 16777216; i++) {
    a += b; d ^= a; d = rotl(d, 16);
    c += d; b ^= c; b = rotl(b, 12);
    a += b; d ^= a; d = rotl(d, 8);
    c += d; b ^= c; b = rotl(b, 7);
  }
  printf("%llu\n%llu\n%llu\n%llu\n", (unsigned long long)a, (unsigned long long)b, (unsigned long long)c, (unsigned long long)d);
  return 0;
}
